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

use std::sync::{Arc, Mutex};
use tokio::time::{interval, Duration};
use crossbeam_channel::Sender;

use crate::lockstep::{
    InputBuffer, InputForPlayer, LockstepFrame, LockstepState, StateHash, TickBatch,
};
use crate::transport::OutboundMsg;

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
        }
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

        // Periodic state hash.
        if tick % self.config.state_hash_interval == 0 {
            let hash = self.placeholder_state_hash(tick);
            let sh = StateHash { tick, hash };
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
}
