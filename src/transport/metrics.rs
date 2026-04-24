//! KCP transport byte/message counters.
//!
//! These primitives exist so integration tests (and future optimization phases)
//! can measure per-event bytes/msg totals and compare against a baseline.
//!
//! The counter is safe to call from the KCP broadcast thread: the per-event
//! map uses a `parking_lot::Mutex` (short critical section — one `entry()`
//! lookup + two `u64` adds), and the global totals use relaxed atomics.

use std::collections::HashMap as StdHashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};

use hashbrown::{Equivalent, HashMap};
use parking_lot::Mutex;

/// Borrowed composite key used to probe `per_event` without allocating.
///
/// Hashes and compares the same way the owned `(String, String)` key does
/// (hashbrown's tuple `Hash` impl hashes each field in order), so the hot
/// path can look up a stored `(String, String)` entry via a `(&str, &str)`
/// wrapped in `BorrowedKey`.
#[derive(Copy, Clone)]
struct BorrowedKey<'a> {
    msg_type: &'a str,
    action: &'a str,
}

impl Hash for BorrowedKey<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Must match `<(String, String) as Hash>::hash`, which hashes each
        // field in order. `String`'s Hash defers to `str`'s Hash, so hashing
        // `&str` here is identical.
        self.msg_type.hash(state);
        self.action.hash(state);
    }
}

impl Equivalent<(String, String)> for BorrowedKey<'_> {
    fn equivalent(&self, key: &(String, String)) -> bool {
        self.msg_type == key.0.as_str() && self.action == key.1.as_str()
    }
}

/// Per-event and global bytes/msg totals observed on the KCP wire.
///
/// The per-event map is keyed on `(msg_type, action)` as owned strings, but
/// the hot path looks up via `BorrowedKey<'_>` (a `(&str, &str)` wrapper)
/// using hashbrown's `Equivalent` trait, so `record()` avoids any allocation
/// once a key is registered.
#[derive(Debug)]
pub struct KcpBytesCounter {
    /// key = (msg_type, action) e.g. ("hero", "stats"), ("creep", "M")
    /// value = (bytes, msgs)
    per_event: Mutex<HashMap<(String, String), (u64, u64)>>,
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

    /// Record one event of `bytes` bytes under `(msg_type, action)`.
    ///
    /// Hot path (key already present): no allocation — probe the map with a
    /// borrowed `BorrowedKey` wrapping `(&str, &str)` via hashbrown's
    /// `Equivalent` trait.
    ///
    /// Cold path (first occurrence): allocates two `String`s for the key.
    pub fn record(&self, msg_type: &str, action: &str, bytes: usize) {
        let bytes = bytes as u64;
        {
            let mut map = self.per_event.lock();
            // Hot path: already-registered keys skip the allocation.
            let borrowed = BorrowedKey { msg_type, action };
            if let Some(entry) = map.get_mut(&borrowed) {
                entry.0 += bytes;
                entry.1 += 1;
            } else {
                map.insert((msg_type.to_owned(), action.to_owned()), (bytes, 1));
            }
        }
        self.total_bytes.fetch_add(bytes, Ordering::Relaxed);
        self.total_msgs.fetch_add(1, Ordering::Relaxed);
    }

    /// Take a deep-copy snapshot of the current totals. Caller can inspect the
    /// snapshot even after `reset()` is called.
    pub fn snapshot(&self) -> KcpCounterSnapshot {
        let map = self.per_event.lock();
        let per_event: StdHashMap<(String, String), (u64, u64)> = map
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
    pub per_event: StdHashMap<(String, String), (u64, u64)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Convenience for test assertions — look up by `(&str, &str)`.
    fn get<'a>(
        snap: &'a KcpCounterSnapshot,
        msg_type: &str,
        action: &str,
    ) -> Option<&'a (u64, u64)> {
        snap.per_event.get(&(msg_type.to_owned(), action.to_owned()))
    }

    #[test]
    fn record_new_and_existing_keys_accumulate() {
        let c = KcpBytesCounter::new();
        c.record("hero", "stats", 100);
        c.record("hero", "stats", 50);
        c.record("creep", "M", 200);

        let snap = c.snapshot();
        assert_eq!(snap.total_bytes, 350);
        assert_eq!(snap.total_msgs, 3);
        assert_eq!(get(&snap, "hero", "stats"), Some(&(150u64, 2u64)));
        assert_eq!(get(&snap, "creep", "M"), Some(&(200u64, 1u64)));
        assert_eq!(snap.per_event.len(), 2);
    }

    #[test]
    fn snapshot_is_a_deep_copy() {
        let c = KcpBytesCounter::new();
        c.record("a", "b", 10);
        let snap = c.snapshot();

        // Mutating the live counter must not change the snapshot.
        c.record("a", "b", 999);
        c.record("x", "y", 1);
        assert_eq!(snap.total_bytes, 10);
        assert_eq!(snap.total_msgs, 1);
        assert_eq!(get(&snap, "a", "b"), Some(&(10u64, 1u64)));
        assert!(get(&snap, "x", "y").is_none());
    }

    #[test]
    fn reset_clears_everything() {
        let c = KcpBytesCounter::new();
        c.record("hero", "stats", 100);
        c.record("creep", "M", 200);

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
        c.record("hero", "stats", 7);
        let again = c.snapshot();
        assert_eq!(again.total_bytes, 7);
        assert_eq!(again.total_msgs, 1);
        assert_eq!(get(&again, "hero", "stats"), Some(&(7u64, 1u64)));
    }

    /// Regression test for the hot-path optimization: after a key is inserted
    /// once, subsequent `record()` calls must not allocate new `String`s for
    /// the key. We can't directly observe allocations, but we can at least
    /// assert the map doesn't grow beyond one entry when the same `(msg_type,
    /// action)` is recorded many times.
    #[test]
    fn hot_path_does_not_duplicate_keys() {
        let c = KcpBytesCounter::new();
        for _ in 0..1000 {
            c.record("hero", "stats", 10);
        }
        let snap = c.snapshot();
        assert_eq!(snap.per_event.len(), 1);
        assert_eq!(get(&snap, "hero", "stats"), Some(&(10_000u64, 1000u64)));
        assert_eq!(snap.total_bytes, 10_000);
        assert_eq!(snap.total_msgs, 1000);
    }
}
