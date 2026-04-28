//! Sweep and Prune (SAP) spatial index。對 (Id, Item) 為 generic。
//!
//! 演算法：維護按 X / Y 軸 entry.position 排序的兩個 Vec；query 時兩軸各自 binary_search
//! 找 [center-radius, center+radius] 區段，掃出候選並對交集做 distance 過濾。
//! 對「大量靜態或慢速移動實體 + 高頻範圍查詢」最佳。
//!
//! 動態更新：insert/remove 都需要保持兩個 sorted Vec 的 invariants：
//! - insert：binary_search 找位置 + Vec::insert（O(n) 複製，hundreds 量級可接受）
//! - remove：先在 id_index 拿位置，再從兩個 Vec 各自 binary_search + remove
//! - update = remove + insert
//!
//! 由於每個 entry 在 X 軸 / Y 軸 sorted Vec 中各只出現一次（不像 SHG/QuadTree 跨多 cell），
//! 這個 impl 是 1-1 映射，沒有跨 cell 的去重需求。

use std::cmp::Ordering;
use std::collections::HashMap;
use std::hash::Hash;
use vek::Vec2;

use super::spatial_index::{Bounds, Entry, SpatialIndex};

/// 軸索引 entry：只存 (Id, position 該軸座標)，用來 sort + binary search。
#[derive(Debug, Clone)]
struct AxisEntry<Id> {
    id: Id,
    coord: f32,
}

pub struct SweepAndPrune<Id, Item> {
    /// 按 position.x 升冪排序
    xs: Vec<AxisEntry<Id>>,
    /// 按 position.y 升冪排序
    ys: Vec<AxisEntry<Id>>,
    /// id → (item, position, bounding_radius)：source of truth
    items: HashMap<Id, (Item, Vec2<f32>, f32)>,
}

impl<Id, Item> SweepAndPrune<Id, Item>
where
    Id: Clone + Eq + Hash + Ord + Send + Sync + 'static,
    Item: Clone + Send + Sync + 'static,
{
    pub fn new() -> Self {
        Self {
            xs: Vec::new(),
            ys: Vec::new(),
            items: HashMap::new(),
        }
    }

    fn insert_axis(arr: &mut Vec<AxisEntry<Id>>, entry: AxisEntry<Id>) {
        // 找插入位置：先按 coord 排，相同 coord 用 id Ord 排
        let idx = arr
            .binary_search_by(|probe| {
                probe.coord
                    .partial_cmp(&entry.coord)
                    .unwrap_or(Ordering::Equal)
                    .then_with(|| probe.id.cmp(&entry.id))
            })
            .unwrap_or_else(|i| i);
        arr.insert(idx, entry);
    }

    fn remove_axis(arr: &mut Vec<AxisEntry<Id>>, id: &Id, coord: f32) -> bool {
        // 用 binary_search 找到 coord 的範圍，線性掃 id 比對；同 coord 元素不太可能很多
        let pos = arr.binary_search_by(|probe| {
            probe.coord.partial_cmp(&coord).unwrap_or(Ordering::Equal)
                .then_with(|| probe.id.cmp(id))
        });
        match pos {
            Ok(i) => {
                arr.remove(i);
                true
            }
            Err(_) => {
                // 浮點 / NaN 防身：fallback 線性找
                if let Some(i) = arr.iter().position(|e| e.id == *id) {
                    arr.remove(i);
                    true
                } else {
                    false
                }
            }
        }
    }

    /// 在 sorted axis array 上找出 coord 落在 [lo, hi] 區間內的所有 entry index 範圍。
    fn axis_range(arr: &[AxisEntry<Id>], lo: f32, hi: f32) -> (usize, usize) {
        let lower = arr
            .binary_search_by(|probe| {
                probe.coord.partial_cmp(&lo).unwrap_or(Ordering::Equal)
            })
            .unwrap_or_else(|i| i);
        let upper = arr
            .binary_search_by(|probe| {
                probe.coord.partial_cmp(&hi).unwrap_or(Ordering::Equal)
                    .then(Ordering::Greater)
            })
            .unwrap_or_else(|i| i);
        // lower 可能落在「等於 lo」的中間；保守往左找直到 < lo
        let mut l = lower;
        while l > 0 && arr[l - 1].coord >= lo {
            l -= 1;
        }
        let mut r = upper;
        while r < arr.len() && arr[r].coord <= hi {
            r += 1;
        }
        (l, r)
    }
}

impl<Id, Item> SpatialIndex<Id, Item> for SweepAndPrune<Id, Item>
where
    Id: Clone + Eq + Hash + Ord + Send + Sync + 'static,
    Item: Clone + Send + Sync + 'static,
{
    fn initialize(&mut self, _bounds: Bounds, entries: Vec<Entry<Id, Item>>) {
        self.xs.clear();
        self.ys.clear();
        self.items.clear();
        // batch initialize：先收齊再一次 sort，比 N 次 insert 快
        for e in entries {
            let id = e.id.clone();
            self.xs.push(AxisEntry { id: id.clone(), coord: e.position.x });
            self.ys.push(AxisEntry { id: id.clone(), coord: e.position.y });
            self.items.insert(id, (e.item, e.position, e.bounding_radius));
        }
        self.xs.sort_by(|a, b| {
            a.coord.partial_cmp(&b.coord).unwrap_or(Ordering::Equal)
                .then_with(|| a.id.cmp(&b.id))
        });
        self.ys.sort_by(|a, b| {
            a.coord.partial_cmp(&b.coord).unwrap_or(Ordering::Equal)
                .then_with(|| a.id.cmp(&b.id))
        });
    }

    fn insert(&mut self, entry: Entry<Id, Item>) {
        // 同 id 重插：先 remove 舊的
        if let Some((_old_item, old_pos, _old_r)) = self.items.remove(&entry.id) {
            Self::remove_axis(&mut self.xs, &entry.id, old_pos.x);
            Self::remove_axis(&mut self.ys, &entry.id, old_pos.y);
        }
        Self::insert_axis(&mut self.xs, AxisEntry { id: entry.id.clone(), coord: entry.position.x });
        Self::insert_axis(&mut self.ys, AxisEntry { id: entry.id.clone(), coord: entry.position.y });
        self.items.insert(entry.id, (entry.item, entry.position, entry.bounding_radius));
    }

    fn remove(&mut self, id: &Id) -> bool {
        let (_, pos, _) = match self.items.remove(id) {
            Some(v) => v,
            None => return false,
        };
        Self::remove_axis(&mut self.xs, id, pos.x);
        Self::remove_axis(&mut self.ys, id, pos.y);
        true
    }

    fn update(&mut self, entry: Entry<Id, Item>) {
        self.remove(&entry.id);
        self.insert(entry);
    }

    fn query_in_range(&self, center: Vec2<f32>, radius: f32) -> Vec<Entry<Id, Item>> {
        // 預估最大 bounding_radius 以擴大 axis 掃描範圍（不掃過會漏 entry）
        let max_r = self.items.values().map(|(_, _, r)| *r).fold(0.0_f32, f32::max);
        let extended = radius + max_r;

        let (lx, rx) = Self::axis_range(&self.xs, center.x - extended, center.x + extended);
        let (ly, ry) = Self::axis_range(&self.ys, center.y - extended, center.y + extended);

        // 取 X 範圍的 id set；對 Y 範圍 iterate，做交集
        let mut x_ids: std::collections::BTreeSet<Id> = std::collections::BTreeSet::new();
        for i in lx..rx {
            x_ids.insert(self.xs[i].id.clone());
        }

        let mut results: Vec<Entry<Id, Item>> = Vec::new();
        for i in ly..ry {
            let id = &self.ys[i].id;
            if !x_ids.contains(id) {
                continue;
            }
            if let Some((item, pos, r)) = self.items.get(id) {
                let extended_r = radius + r.max(0.0);
                if pos.distance(center) <= extended_r {
                    results.push(Entry {
                        id: id.clone(),
                        item: item.clone(),
                        position: *pos,
                        bounding_radius: *r,
                    });
                }
            }
        }
        results
    }

    fn count_nodes(&self) -> usize {
        self.items.len()
    }

    fn name(&self) -> &'static str { "sap" }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(id: &str, x: f32, y: f32, r: f32) -> Entry<String, ()> {
        Entry::new(id.to_string(), (), Vec2::new(x, y), r)
    }

    fn world_bounds() -> Bounds {
        Bounds::new(Vec2::new(0.0, 0.0), Vec2::new(1000.0, 1000.0))
    }

    fn ids_of(results: &[Entry<String, ()>]) -> Vec<String> {
        let mut v: Vec<String> = results.iter().map(|e| e.id.clone()).collect();
        v.sort();
        v
    }

    #[test]
    fn insert_into_initialized_then_query() {
        let mut s: SweepAndPrune<String, ()> = SweepAndPrune::new();
        s.initialize(world_bounds(), vec![]);
        s.insert(pt("a", 100.0, 100.0, 10.0));
        s.insert(pt("b", 800.0, 800.0, 10.0));

        assert_eq!(ids_of(&s.query_in_range(Vec2::new(100.0, 100.0), 50.0)), vec!["a"]);
        assert_eq!(ids_of(&s.query_in_range(Vec2::new(800.0, 800.0), 50.0)), vec!["b"]);
    }

    #[test]
    fn remove_drops_entry_from_subsequent_queries() {
        let mut s: SweepAndPrune<String, ()> = SweepAndPrune::new();
        s.initialize(world_bounds(), vec![
            pt("a", 100.0, 100.0, 10.0),
            pt("b", 120.0, 110.0, 10.0),
        ]);
        assert_eq!(ids_of(&s.query_in_range(Vec2::new(110.0, 105.0), 100.0)), vec!["a", "b"]);
        assert!(s.remove(&"a".to_string()));
        assert_eq!(ids_of(&s.query_in_range(Vec2::new(110.0, 105.0), 100.0)), vec!["b"]);
        assert!(!s.remove(&"a".to_string()));
    }

    #[test]
    fn update_moves_entry_in_query_results() {
        let mut s: SweepAndPrune<String, ()> = SweepAndPrune::new();
        s.initialize(world_bounds(), vec![pt("mover", 100.0, 100.0, 5.0)]);
        assert_eq!(ids_of(&s.query_in_range(Vec2::new(100.0, 100.0), 20.0)), vec!["mover"]);

        s.update(pt("mover", 900.0, 900.0, 5.0));
        assert!(ids_of(&s.query_in_range(Vec2::new(100.0, 100.0), 20.0)).is_empty());
        assert_eq!(ids_of(&s.query_in_range(Vec2::new(900.0, 900.0), 20.0)), vec!["mover"]);
    }

    #[test]
    fn axis_arrays_stay_sorted_after_many_mutations() {
        let mut s: SweepAndPrune<String, ()> = SweepAndPrune::new();
        s.initialize(world_bounds(), vec![]);
        for i in 0..50 {
            let x = (i as f32 * 13.7) % 900.0 + 50.0;
            let y = (i as f32 * 7.3) % 900.0 + 50.0;
            s.insert(pt(&format!("o{}", i), x, y, 5.0));
        }
        for i in (0..50).step_by(2) {
            assert!(s.remove(&format!("o{}", i)));
        }

        // Invariant: xs/ys sorted by coord
        for w in s.xs.windows(2) {
            assert!(w[0].coord <= w[1].coord, "xs not sorted: {} > {}", w[0].coord, w[1].coord);
        }
        for w in s.ys.windows(2) {
            assert!(w[0].coord <= w[1].coord, "ys not sorted: {} > {}", w[0].coord, w[1].coord);
        }
        // 每個 id 都應在兩個 axis array 各自只出現一次
        assert_eq!(s.xs.len(), 25);
        assert_eq!(s.ys.len(), 25);
    }

    #[test]
    fn query_handles_large_bounding_radius_extension() {
        let mut s: SweepAndPrune<String, ()> = SweepAndPrune::new();
        s.initialize(world_bounds(), vec![
            pt("big", 500.0, 500.0, 200.0),    // 大半徑：query 中心離 position 600 也應命中（500+200+query_r）
            pt("far", 100.0, 100.0, 5.0),
        ]);

        // query 中心在 (700, 500)，radius 50 — big.position 距 200，加 big radius 200 + query 50 = 250 > 200 ✓
        let q = s.query_in_range(Vec2::new(700.0, 500.0), 50.0);
        assert_eq!(ids_of(&q), vec!["big"]);
    }
}
