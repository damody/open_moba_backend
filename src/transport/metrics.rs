//! KCP transport byte/message counters.
//!
//! These primitives exist so integration tests (and future optimization phases)
//! can measure per-event bytes/msg totals and compare against a baseline.
//!
//! The counter is safe to call from the KCP broadcast thread: the per-event
//! map uses a `parking_lot::Mutex` (short critical section — one `entry()`
//! lookup + two `u64` adds), and the global totals use relaxed atomics.

use std::collections::HashMap as StdHashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use hashbrown::HashMap;
use parking_lot::Mutex;

/// Per-event and global bytes/msg totals observed on the KCP wire.
pub struct KcpBytesCounter {
    /// key = event kind string (e.g. "hero.stats", "creep.M")
    /// value = (bytes, msgs)
    per_event: Mutex<HashMap<String, (u64, u64)>>,
    total_bytes: AtomicU64,
    total_msgs: AtomicU64,
}

impl KcpBytesCounter {
    pub fn new() -> Self {
        Self {
            per_event: Mutex::new(HashMap::new()),
            total_bytes: AtomicU64::new(0),
            total_msgs: AtomicU64::new(0),
        }
    }

    /// Record one event of `bytes` bytes under `kind`.
    ///
    /// First occurrence of `kind` allocates a `String` key (cold path); the
    /// hot path (key already present) only bumps two counters in the map plus
    /// two relaxed atomics.
    pub fn record(&self, kind: &str, bytes: usize) {
        let bytes = bytes as u64;
        {
            let mut map = self.per_event.lock();
            // Hot path: already-registered keys skip the allocation.
            if let Some(entry) = map.get_mut(kind) {
                entry.0 += bytes;
                entry.1 += 1;
            } else {
                map.insert(kind.to_owned(), (bytes, 1));
            }
        }
        self.total_bytes.fetch_add(bytes, Ordering::Relaxed);
        self.total_msgs.fetch_add(1, Ordering::Relaxed);
    }

    /// Take a deep-copy snapshot of the current totals. Caller can inspect the
    /// snapshot even after `reset()` is called.
    pub fn snapshot(&self) -> KcpCounterSnapshot {
        let map = self.per_event.lock();
        let per_event: StdHashMap<String, (u64, u64)> = map
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        KcpCounterSnapshot {
            total_bytes: self.total_bytes.load(Ordering::Relaxed),
            total_msgs: self.total_msgs.load(Ordering::Relaxed),
            per_event,
        }
    }

    /// Clear all per-event entries and zero the global totals.
    pub fn reset(&self) {
        self.per_event.lock().clear();
        self.total_bytes.store(0, Ordering::Relaxed);
        self.total_msgs.store(0, Ordering::Relaxed);
    }
}

impl Default for KcpBytesCounter {
    fn default() -> Self {
        Self::new()
    }
}

/// Plain, owned copy of the counter's state. Crosses API boundaries, so the
/// map uses `std::collections::HashMap` (not hashbrown).
#[derive(Debug, Clone, Default)]
pub struct KcpCounterSnapshot {
    pub total_bytes: u64,
    pub total_msgs: u64,
    pub per_event: StdHashMap<String, (u64, u64)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_new_and_existing_keys_accumulate() {
        let c = KcpBytesCounter::new();
        c.record("hero.stats", 100);
        c.record("hero.stats", 50);
        c.record("creep.M", 200);

        let snap = c.snapshot();
        assert_eq!(snap.total_bytes, 350);
        assert_eq!(snap.total_msgs, 3);
        assert_eq!(snap.per_event.get("hero.stats"), Some(&(150u64, 2u64)));
        assert_eq!(snap.per_event.get("creep.M"), Some(&(200u64, 1u64)));
        assert_eq!(snap.per_event.len(), 2);
    }

    #[test]
    fn snapshot_is_a_deep_copy() {
        let c = KcpBytesCounter::new();
        c.record("a.b", 10);
        let snap = c.snapshot();

        // Mutating the live counter must not change the snapshot.
        c.record("a.b", 999);
        c.record("x.y", 1);
        assert_eq!(snap.total_bytes, 10);
        assert_eq!(snap.total_msgs, 1);
        assert_eq!(snap.per_event.get("a.b"), Some(&(10u64, 1u64)));
        assert!(snap.per_event.get("x.y").is_none());
    }

    #[test]
    fn reset_clears_everything() {
        let c = KcpBytesCounter::new();
        c.record("hero.stats", 100);
        c.record("creep.M", 200);

        // Keep a snapshot to prove reset doesn't touch previously-taken copies.
        let before = c.snapshot();
        c.reset();

        let after = c.snapshot();
        assert_eq!(after.total_bytes, 0);
        assert_eq!(after.total_msgs, 0);
        assert!(after.per_event.is_empty());

        // Pre-reset snapshot still has its data.
        assert_eq!(before.total_bytes, 300);
        assert_eq!(before.total_msgs, 2);

        // Can record again after reset.
        c.record("hero.stats", 7);
        let again = c.snapshot();
        assert_eq!(again.total_bytes, 7);
        assert_eq!(again.total_msgs, 1);
        assert_eq!(again.per_event.get("hero.stats"), Some(&(7u64, 1u64)));
    }
}
