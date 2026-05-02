//! 60Hz tick broadcaster. Runs in a tokio task spawned from main.rs.
//!
//! Each tick:
//!   1. Advance `LockstepState.current_tick`.
//!   2. Drain `InputBuffer` for that tick into a `TickBatch`.
//!   3. Send the batch out via `OutboundMsg::lockstep_frame(...)` so the
//!      kcp transport's broadcast thread (Task 2.3) emits tag 0x11 to all
//!      connected lockstep sessions.
//!   4. Every `state_hash_interval` ticks, also emit a `StateHash` (tag 0x12)
//!      for desync detection.
//!
//! Phase 2 status:
//!  - `placeholder_state_hash` is a stand-in (`tick * golden_ratio`); Phase 3
//!    replaces it with `omoba_sim::state_hash::hash_sorted_by_id` over the
//!    real ECS state.
//!  - The legacy 30Hz simulation dispatcher continues to run unchanged.
//!  - `server_events` is always empty in Phase 2; Phase 5 will inject
//!    player_join / wave_start / etc. server-authoritative events.
//!
//! Phase 3.4 status:
//!  - Optional `state_hash_rx` channel is fed by the dispatcher tick loop in
//!    `state::core::State::tick`, which calls `compute_state_hash(&world)`
//!    every `state_hash_interval` ticks (using its own dispatcher tick number).
//!    Broadcaster `try_recv`s the latest pending value when its 60Hz state-
//!    hash interval fires.
//!  - The dispatcher (30Hz) and broadcaster (60Hz) are not aligned; this is
//!    intentional. The hash is timestamped with the dispatcher's tick at
//!    compute time; the broadcaster forwards `(tick, hash)` verbatim. Lag is
//!    bounded to one dispatcher tick (~33ms at 30Hz) which is well under any
//!    reasonable desync detection window.
//!  - When `state_hash_rx` is `None` (legacy / test setups) the broadcaster
//!    falls back to `placeholder_state_hash` so existing tests keep passing.

use std::sync::{Arc, Mutex};
use tokio::time::{interval, Duration};
use crossbeam_channel::{Receiver, Sender};

use crate::lockstep::{
    InputBuffer, InputForPlayer, LockstepFrame, LockstepState, StateHash, TickBatch,
};
use crate::transport::OutboundMsg;

/// Phase 3.4: payload published by the dispatcher tick loop after computing
/// `compute_state_hash(&world)`. The broadcaster forwards verbatim — `tick`
/// here is the dispatcher's tick (30Hz cadence), distinct from the
/// broadcaster's own 60Hz `LockstepState.current_tick`.
pub type StateHashSample = (u32, u64);

#[derive(Clone, Copy, Debug)]
pub struct TickBroadcasterConfig {
    /// Tick period in microseconds. 16_667 = 60Hz.
    pub tick_period_us: u64,
    /// Emit a StateHash every N ticks. 600 = 10s @ 60Hz.
    pub state_hash_interval: u32,
}

impl Default for TickBroadcasterConfig {
    fn default() -> Self {
        Self {
            tick_period_us: 16_667,
            state_hash_interval: 600,
        }
    }
}

pub struct TickBroadcaster {
    config: TickBroadcasterConfig,
    input_buffer: Arc<Mutex<InputBuffer>>,
    state: Arc<Mutex<LockstepState>>,
    out_tx: Sender<OutboundMsg>,
    /// Phase 3.4: optional source of dispatcher-computed state hashes. When
    /// `Some`, broadcaster `try_recv`s on every state-hash tick and forwards
    /// the latest pending sample (logging a warn + emitting hash=0 if the
    /// channel is empty). `None` falls back to `placeholder_state_hash`.
    state_hash_rx: Option<Receiver<StateHashSample>>,
}

impl TickBroadcaster {
    pub fn new(
        config: TickBroadcasterConfig,
        input_buffer: Arc<Mutex<InputBuffer>>,
        state: Arc<Mutex<LockstepState>>,
        out_tx: Sender<OutboundMsg>,
    ) -> Self {
        Self {
            config,
            input_buffer,
            state,
            out_tx,
            state_hash_rx: None,
        }
    }

    /// Phase 3.4: attach a dispatcher-side state-hash source. Builder-style
    /// so existing call sites (incl. tests) don't break.
    pub fn with_state_hash_rx(mut self, rx: Receiver<StateHashSample>) -> Self {
        self.state_hash_rx = Some(rx);
        self
    }

    /// Spawn the 60Hz tick loop. Runs until `out_tx` is closed (channel
    /// disconnect surfaces as send error and we log+exit).
    pub async fn run(self) {
        let mut ticker = interval(Duration::from_micros(self.config.tick_period_us));
        // tokio's first interval tick fires immediately; skip it so the
        // first published tick lands at +period rather than at t=0.
        ticker.tick().await;
        loop {
            ticker.tick().await;
            if !self.fire_one_tick() {
                log::info!("TickBroadcaster: outbound channel closed, exiting tick loop");
                break;
            }
        }
    }

    /// Fires one tick. Returns false if the outbound channel is closed
    /// (indicates the transport shut down — caller exits the loop).
    fn fire_one_tick(&self) -> bool {
        // Advance tick counter.
        let tick = {
            let mut s = self.state.lock().unwrap();
            s.current_tick = s.current_tick.wrapping_add(1);
            s.current_tick
        };

        // Drain inputs targeted at this tick.
        let inputs: Vec<(u32, _)> =
            self.input_buffer.lock().unwrap().drain_for_tick(tick);
        let inputs_proto: Vec<InputForPlayer> = inputs
            .into_iter()
            .map(|(player_id, input)| InputForPlayer {
                player_id,
                input: Some(input),
            })
            .collect();

        let batch = TickBatch {
            tick,
            inputs: inputs_proto,
            // Phase 2: empty; Phase 5+ injects PlayerJoin/WaveStart/etc.
            server_events: vec![],
        };

        let msg = OutboundMsg::lockstep_frame(LockstepFrame::TickBatch(batch));
        if let Err(e) = self.out_tx.send(msg) {
            log::warn!("TickBroadcaster failed to send TickBatch: {e}");
            return false;
        }

        // Periodic state hash. Phase 3.4 sources the hash from
        // `state_hash_rx` (dispatcher) when available; otherwise falls back
        // to `placeholder_state_hash`.
        if tick % self.config.state_hash_interval == 0 {
            let (hash_tick, hash) = self.latest_state_hash(tick);
            let sh = StateHash { tick: hash_tick, hash };
            let msg = OutboundMsg::lockstep_frame(LockstepFrame::StateHash(sh));
            if let Err(e) = self.out_tx.send(msg) {
                log::warn!("TickBroadcaster failed to send StateHash: {e}");
                return false;
            }
        }

        // Periodic cleanup of stale future inputs (e.g. submissions that
        // referenced a tick we already passed because the player was
        // disconnected and reconnected).
        if tick % 60 == 0 {
            self.input_buffer
                .lock()
                .unwrap()
                .evict_older(tick.saturating_sub(120));
        }

        true
    }

    /// PHASE 3: replace with real `omoba_sim::state_hash::hash_sorted_by_id`
    /// over the authoritative ECS state. The placeholder is deterministic
    /// so the wire path can be exercised in Phase 2 integration tests.
    fn placeholder_state_hash(&self, tick: u32) -> u64 {
        (tick as u64).wrapping_mul(0x9E3779B97F4A7C15)
    }

    /// Phase 3.4: returns `(tick_to_broadcast, hash)`.
    ///
    /// - If `state_hash_rx` is wired: drain the channel and forward the
    ///   newest pending sample (newer dispatcher samples discard older ones).
    ///   The returned tick is the dispatcher's tick at compute time, which
    ///   may lag the broadcaster's `tick` by up to one dispatcher tick (~33ms
    ///   at 30Hz). Empty channel → log warn + return `(tick, 0)`.
    /// - If `state_hash_rx` is None: fall back to `placeholder_state_hash`.
    fn latest_state_hash(&self, broadcaster_tick: u32) -> (u32, u64) {
        match &self.state_hash_rx {
            Some(rx) => {
                let mut latest: Option<StateHashSample> = None;
                while let Ok(sample) = rx.try_recv() {
                    latest = Some(sample);
                }
                match latest {
                    Some((t, h)) => (t, h),
                    None => {
                        log::warn!(
                            "TickBroadcaster: no fresh state_hash sample at tick {}, broadcasting hash=0",
                            broadcaster_tick
                        );
                        (broadcaster_tick, 0)
                    }
                }
            }
            None => (broadcaster_tick, self.placeholder_state_hash(broadcaster_tick)),
        }
    }
}

#[cfg(test)]
mod tests {
    //! Phase 2.5 unit tests for TickBroadcaster.
    //!
    //! These exercise `fire_one_tick` directly by constructing a broadcaster
    //! with a mock outbound channel (crossbeam_channel::unbounded), seeding
    //! the InputBuffer with synthetic submissions, and asserting on the
    //! resulting `OutboundMsg::lockstep_frame` payloads. Avoids tokio
    //! runtime dependency by skipping `run()` and calling `fire_one_tick`
    //! synchronously.
    use super::*;
    use crate::lockstep::{NoOp, PlayerInput, PlayerInputEnum};
    use crossbeam_channel::unbounded;

    fn noop_input() -> PlayerInput {
        PlayerInput {
            action: Some(PlayerInputEnum::NoOp(NoOp {})),
        }
    }

    /// Helper: pull every msg from rx into a Vec, classify by frame type.
    fn drain_frames(rx: &crossbeam_channel::Receiver<OutboundMsg>) -> Vec<LockstepFrame> {
        let mut out = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            if let Some(frame) = msg.lockstep_frame {
                out.push(frame);
            }
        }
        out
    }

    fn make_broadcaster(
        config: TickBroadcasterConfig,
    ) -> (
        TickBroadcaster,
        Arc<Mutex<InputBuffer>>,
        Arc<Mutex<LockstepState>>,
        crossbeam_channel::Receiver<OutboundMsg>,
    ) {
        let buf = Arc::new(Mutex::new(InputBuffer::new()));
        let state = Arc::new(Mutex::new(LockstepState::new(0xCAFE_BABE_DEAD_BEEF)));
        let (tx, rx) = unbounded();
        let bc = TickBroadcaster::new(config, buf.clone(), state.clone(), tx);
        (bc, buf, state, rx)
    }

    #[test]
    fn fires_tick_batches_with_buffered_inputs() {
        let cfg = TickBroadcasterConfig {
            tick_period_us: 16_667,
            state_hash_interval: 600,
        };
        let (bc, buf, _state, rx) = make_broadcaster(cfg);

        // Seed: at tick 5, 2 players' inputs (player 7 + player 3) so we
        // can confirm BTreeMap ordering carried through to TickBatch.
        {
            let mut b = buf.lock().unwrap();
            assert!(b.submit(0, 7, 5, noop_input()));
            assert!(b.submit(0, 3, 5, noop_input()));
        }

        // Fire 5 ticks. Tick 1..=4 should be empty TickBatch, tick 5 has 2 inputs.
        for _ in 0..5 {
            assert!(bc.fire_one_tick(), "fire_one_tick returned false (channel closed?)");
        }

        let frames = drain_frames(&rx);
        assert_eq!(frames.len(), 5, "expected 5 TickBatch frames, got {}", frames.len());

        for (i, frame) in frames.iter().enumerate() {
            let expect_tick = (i + 1) as u32;
            match frame {
                LockstepFrame::TickBatch(b) => {
                    assert_eq!(b.tick, expect_tick);
                    if expect_tick == 5 {
                        assert_eq!(b.inputs.len(), 2, "tick 5 should carry 2 inputs");
                        // BTreeMap iteration order: 3, then 7.
                        assert_eq!(b.inputs[0].player_id, 3);
                        assert_eq!(b.inputs[1].player_id, 7);
                    } else {
                        assert!(b.inputs.is_empty(), "tick {} should be empty", expect_tick);
                    }
                }
                other => panic!("expected TickBatch frame, got {:?}", other),
            }
        }
    }

    #[test]
    fn emits_state_hash_at_interval_multiples() {
        // Use small interval (3) to avoid firing 600 ticks in the test.
        let cfg = TickBroadcasterConfig {
            tick_period_us: 16_667,
            state_hash_interval: 3,
        };
        let (bc, _buf, _state, rx) = make_broadcaster(cfg);

        // Fire 7 ticks → expect StateHash at ticks 3 and 6.
        for _ in 0..7 {
            assert!(bc.fire_one_tick());
        }

        let frames = drain_frames(&rx);
        // 7 TickBatch + 2 StateHash = 9 frames.
        assert_eq!(frames.len(), 9, "frames = {:?}", frames);

        let mut tick_batch_count = 0;
        let mut state_hash_ticks = Vec::new();
        for frame in &frames {
            match frame {
                LockstepFrame::TickBatch(_) => tick_batch_count += 1,
                LockstepFrame::StateHash(sh) => state_hash_ticks.push(sh.tick),
                other => panic!("unexpected frame variant: {:?}", other),
            }
        }
        assert_eq!(tick_batch_count, 7);
        assert_eq!(state_hash_ticks, vec![3, 6]);
    }

    #[test]
    fn placeholder_state_hash_is_deterministic_pin() {
        // Pin the exact placeholder formula. Phase 3 changes this — when
        // it does, this test should fail and be retired (see comment in
        // placeholder_state_hash).
        let cfg = TickBroadcasterConfig::default();
        let (bc, _buf, _state, _rx) = make_broadcaster(cfg);

        // tick=600 * 0x9E3779B97F4A7C15 (golden-ratio constant).
        let expected_600: u64 = 600u64.wrapping_mul(0x9E3779B97F4A7C15);
        assert_eq!(bc.placeholder_state_hash(600), expected_600);

        // tick=1: the constant itself.
        assert_eq!(bc.placeholder_state_hash(1), 0x9E3779B97F4A7C15);

        // tick=0 always hashes to 0 (multiplicative identity).
        assert_eq!(bc.placeholder_state_hash(0), 0);
    }

    #[test]
    fn returns_false_when_outbound_channel_closes() {
        let cfg = TickBroadcasterConfig::default();
        let (bc, _buf, _state, rx) = make_broadcaster(cfg);
        drop(rx); // close the channel
        // First send fails → fire_one_tick returns false.
        assert!(!bc.fire_one_tick());
    }

    #[test]
    fn evicts_old_inputs_every_60_ticks() {
        // Verify the periodic cleanup branch (`tick % 60 == 0`).
        let cfg = TickBroadcasterConfig {
            tick_period_us: 16_667,
            state_hash_interval: 100_000, // disable state hash for this test
        };
        let (bc, buf, _state, _rx) = make_broadcaster(cfg);

        // Submit one orphan input at tick=10 (will be drained on tick 10),
        // and one orphan at tick=200 that we'll never reach via fire.
        // Then manually re-submit a stale future-input afterwards to test
        // eviction.
        // Easier: directly insert and check pending_count behavior.
        {
            let mut b = buf.lock().unwrap();
            b.submit(0, 1, 200, noop_input());
            assert_eq!(b.pending_count(), 1);
        }

        // Fire 60 ticks. At tick=60, evict_older(60-120=saturating 0) is called
        // — saturating_sub(120) on tick=60 is 0, so nothing pre-tick-0 is
        // evicted (tick=200 input survives).
        for _ in 0..60 {
            assert!(bc.fire_one_tick());
        }
        assert_eq!(buf.lock().unwrap().pending_count(), 1, "tick=200 input should survive");

        // Fire to tick 180. At tick=120 we evict_older(0); at tick=180 we
        // evict_older(60) — still doesn't hit the tick=200 entry.
        for _ in 60..180 {
            assert!(bc.fire_one_tick());
        }
        assert_eq!(buf.lock().unwrap().pending_count(), 1);

        // Fire to tick 240 — passes tick 200, so it's drained naturally.
        for _ in 180..240 {
            assert!(bc.fire_one_tick());
        }
        assert_eq!(buf.lock().unwrap().pending_count(), 0);
    }

    /// Phase 3.5: when a `state_hash_rx` is wired (the production
    /// configuration), broadcaster forwards the dispatcher-published hash
    /// verbatim instead of the placeholder. Verifies the (tick, hash) pair
    /// landing in `LockstepFrame::StateHash` matches what was sent on the
    /// channel, and that placeholder formula is bypassed.
    #[test]
    fn broadcaster_uses_real_hash_when_rx_provided() {
        let cfg = TickBroadcasterConfig {
            tick_period_us: 16_667,
            state_hash_interval: 3, // small interval so we hit it quickly
        };
        let (buf, state) = (
            Arc::new(Mutex::new(InputBuffer::new())),
            Arc::new(Mutex::new(LockstepState::new(0xCAFE_BABE_DEAD_BEEF))),
        );
        let (out_tx, out_rx) = unbounded();
        let (hash_tx, hash_rx) = unbounded::<StateHashSample>();

        let bc = TickBroadcaster::new(cfg, buf.clone(), state.clone(), out_tx)
            .with_state_hash_rx(hash_rx);

        // Send a known dispatcher sample: tick=42, hash=0xCAFE_FOOD_DEAD_FEED.
        let known_hash: u64 = 0xCAFE_F00D_DEAD_FEED;
        let dispatcher_tick: u32 = 42;
        hash_tx.send((dispatcher_tick, known_hash)).expect("send hash sample");

        // Fire 3 ticks → at tick=3 the broadcaster fires its state-hash
        // interval and drains the channel.
        for _ in 0..3 {
            assert!(bc.fire_one_tick());
        }

        // Look for the StateHash frame.
        let frames = drain_frames(&out_rx);
        let mut found_state_hash = false;
        for frame in &frames {
            if let LockstepFrame::StateHash(sh) = frame {
                assert_eq!(
                    sh.hash, known_hash,
                    "broadcaster must forward the dispatcher's real hash, not placeholder"
                );
                assert_eq!(
                    sh.tick, dispatcher_tick,
                    "broadcaster must forward the dispatcher's tick stamp, not its own 60Hz tick"
                );
                // Sanity: the placeholder for broadcaster_tick=3 would be
                // 3 * 0x9E3779B97F4A7C15 — should NOT match.
                let placeholder_3: u64 = 3u64.wrapping_mul(0x9E3779B97F4A7C15);
                assert_ne!(
                    sh.hash, placeholder_3,
                    "broadcaster fell back to placeholder despite rx wired"
                );
                found_state_hash = true;
            }
        }
        assert!(found_state_hash, "expected a StateHash frame in {:?}", frames);
    }
}
