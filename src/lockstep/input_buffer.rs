//! Input buffering for server-paced lockstep.
//!
//! Players submit InputSubmit packets carrying a target_tick (typically
//! current_server_tick + 3 for a 50ms input delay at 60Hz). Server collects
//! them per tick. When a tick fires, the buffer drains all inputs targeted
//! at that tick into a `TickBatch`.
//!
//! Late inputs (target_tick already past) are dropped with a log line —
//! the player's client missed the deadline and will see the action lost.
//! Phase 3+ may add a "soft delay extension" policy.

use std::collections::BTreeMap;
use crate::lockstep::PlayerInput;

#[derive(Default)]
pub struct InputBuffer {
    /// target_tick → player_id → input.
    /// Outer BTreeMap so drain_for_tick is O(log N) on the tick key.
    /// Inner BTreeMap keyed by player_id for deterministic iteration order
    /// in TickBatch composition.
    by_tick: BTreeMap<u32, BTreeMap<u32, PlayerInput>>,
}

impl InputBuffer {
    pub fn new() -> Self {
        Self {
            by_tick: BTreeMap::new(),
        }
    }

    /// Submit one input. Returns false if `target_tick < current_tick` (late).
    /// If the same player submits twice for the same tick, the latter wins
    /// (overwrite policy — clients should not double-submit; if they do,
    /// the second is interpreted as a correction).
    pub fn submit(
        &mut self,
        current_tick: u32,
        player_id: u32,
        target_tick: u32,
        input: PlayerInput,
    ) -> bool {
        if target_tick < current_tick {
            return false; // late
        }
        self.by_tick
            .entry(target_tick)
            .or_insert_with(BTreeMap::new)
            .insert(player_id, input);
        true
    }

    /// Drain all inputs targeted at this tick. Returns sorted by player_id
    /// (BTreeMap iteration is in key order — required for deterministic
    /// TickBatch composition across all peers).
    pub fn drain_for_tick(&mut self, tick: u32) -> Vec<(u32, PlayerInput)> {
        self.by_tick
            .remove(&tick)
            .map(|m| m.into_iter().collect())
            .unwrap_or_default()
    }

    /// Drop everything older than `before_tick` — call periodically for
    /// cleanup so the buffer doesn't accumulate orphan submissions whose
    /// owning tick was somehow skipped.
    pub fn evict_older(&mut self, before_tick: u32) {
        self.by_tick.retain(|&t, _| t >= before_tick);
    }

    /// Total pending inputs across all future ticks (for diagnostics).
    pub fn pending_count(&self) -> usize {
        self.by_tick.values().map(|m| m.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lockstep::{NoOp, PlayerInputEnum};

    fn noop() -> PlayerInput {
        PlayerInput {
            action: Some(PlayerInputEnum::NoOp(NoOp {})),
        }
    }

    #[test]
    fn submit_and_drain() {
        let mut b = InputBuffer::new();
        assert!(b.submit(0, 1, 5, noop()));
        assert!(b.submit(0, 2, 5, noop()));
        let drained = b.drain_for_tick(5);
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].0, 1); // sorted by player_id
        assert_eq!(drained[1].0, 2);
        // Already drained — second drain returns empty.
        assert!(b.drain_for_tick(5).is_empty());
    }

    #[test]
    fn late_input_rejected() {
        let mut b = InputBuffer::new();
        assert!(!b.submit(10, 1, 5, noop())); // target=5 < current=10
        assert_eq!(b.pending_count(), 0);
    }

    #[test]
    fn evict_older() {
        let mut b = InputBuffer::new();
        b.submit(0, 1, 1, noop());
        b.submit(0, 1, 2, noop());
        b.submit(0, 1, 3, noop());
        b.evict_older(2);
        assert!(b.drain_for_tick(1).is_empty());
        assert_eq!(b.drain_for_tick(2).len(), 1);
        assert_eq!(b.drain_for_tick(3).len(), 1);
    }
}
