//! KCP 傳輸位元組/訊息計數器。
//!
//! 這些原語的存在是為了進行整合測試（以及未來的最佳化階段）
//! 可以測量每個事件的位元組/訊息總數並與基準進行比較。
//!
//! 計數器可以安全地從 KCP 廣播線程呼叫：每個事件
//! 地圖使用 `parking_lot::Mutex` （短關鍵部分 - 一個 `entry()`
//! 找出+兩個“u64”新增），全域總計使用寬鬆原子。

use std::collections::HashMap as StdHashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};

use hashbrown::{Equivalent, HashMap};
use parking_lot::Mutex;

/// 借用複合鍵用於探測“per_event”而不進行分配。
///
/// 以與擁有的“(String, String)”鍵相同的方式進行雜湊和比較
/// （hashbrown 的元組 `Hash` impl 按順序對每個字段進行哈希），所以熱門
/// 路徑可以透過「(&str, &str)」尋找儲存的「(String, String)」條目
/// 包裹在“BorrowedKey”中。
#[derive(Copy, Clone)]
struct BorrowedKey<'a> {
    msg_type: &'a str,
    action: &'a str,
}

impl Hash for BorrowedKey<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // 必須匹配`<(String, String) as Hash>::hash`，它對每個進行散列
        // 欄位按順序排列。 `String` 的哈希值遵循 `str` 的哈希值，因此哈希
        // 這裡的「&str」是相同的。
        self.msg_type.hash(state);
        self.action.hash(state);
    }
}

impl Equivalent<(String, String)> for BorrowedKey<'_> {
    fn equivalent(&self, key: &(String, String)) -> bool {
        self.msg_type == key.0.as_str() && self.action == key.1.as_str()
    }
}

/// 在 KCP 線路上觀察到的每個事件和全域位元組/訊息總數。
///
/// 每個事件映射以「(msg_type, action)」為鍵作為擁有的字串，但是
/// 熱路徑透過 `BorrowedKey<'_>` （一個 `(&str, &str)` 包裝器尋找）
/// 使用 hashbrown 的“Equivalent”特徵，因此“record()”避免任何分配
/// 一旦註冊了密鑰。
#[derive(Debug)]
pub struct KcpBytesCounter {
    /// key = (msg_type, action) 例如（“英雄”，“統計數據”），（“蠕變”，“M”）
    /// 值=（字節，訊息）
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

    /// 在“(msg_type, action)”下記錄一個“bytes”位元組的事件。
    ///
    /// 熱路徑（密鑰已存在）：無分配 - 使用 a 探測地圖
    /// 透過 hashbrown 借用 `BorrowedKey` 包裝 `(&str, &str)`
    /// “同等”特徵。
    ///
    /// 冷路徑（第一次出現）：為鍵分配兩個“String”。
    pub fn record(&self, msg_type: &str, action: &str, bytes: usize) {
        let bytes = bytes as u64;
        {
            let mut map = self.per_event.lock();
            // 熱路徑：已註冊的鍵跳過分配。
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

    /// 取得目前總數的深層複製快照。呼叫者可以檢查
    /// 即使在呼叫“reset()”之後也可以產生快照。
    pub fn snapshot(&self) -> KcpCounterSnapshot {
        let map = self.per_event.lock();
        let per_event: StdHashMap<(String, String), (u64, u64)> =
            map.iter().map(|(k, v)| (k.clone(), *v)).collect();
        KcpCounterSnapshot {
            total_bytes: self.total_bytes.load(Ordering::Relaxed),
            total_msgs: self.total_msgs.load(Ordering::Relaxed),
            per_event,
        }
    }

    /// 清除所有每個事件條目並將全域總數歸零。
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

/// 櫃檯狀態的普通、擁有的副本。跨越 API 邊界，因此
/// map 使用 `std::collections::HashMap` （不是 hashbrown）。
#[derive(Debug, Clone, Default)]
pub struct KcpCounterSnapshot {
    pub total_bytes: u64,
    pub total_msgs: u64,
    pub per_event: StdHashMap<(String, String), (u64, u64)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 方便測試斷言－透過「(&str, &str)」尋找。
    fn get<'a>(
        snap: &'a KcpCounterSnapshot,
        msg_type: &str,
        action: &str,
    ) -> Option<&'a (u64, u64)> {
        snap.per_event
            .get(&(msg_type.to_owned(), action.to_owned()))
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

        // 改變即時計數器不得改變快照。
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

        // 保留快照以證明重置不會影響先前拍攝的副本。
        let before = c.snapshot();
        c.reset();

        let after = c.snapshot();
        assert_eq!(after.total_bytes, 0);
        assert_eq!(after.total_msgs, 0);
        assert!(after.per_event.is_empty());

        // 預重置快照仍然有其數據。
        assert_eq!(before.total_bytes, 300);
        assert_eq!(before.total_msgs, 2);

        // 重置後可以重新錄製。
        c.record("hero", "stats", 7);
        let again = c.snapshot();
        assert_eq!(again.total_bytes, 7);
        assert_eq!(again.total_msgs, 1);
        assert_eq!(get(&again, "hero", "stats"), Some(&(7u64, 1u64)));
    }

    /// 熱路徑優化的回歸測試：插入密鑰後
    /// 一旦，後續的“record()”呼叫不得分配新的“String”
    /// 關鍵。我們無法直接觀察分配情況，但至少可以
    /// 斷言當相同的 `(msg_type,
    /// action)`被記錄了很多次。
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
