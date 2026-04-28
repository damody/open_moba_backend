//! Sweep and Prune (SAP) spatial index。對 (Id, Item) 為 generic。
//!
//! 設計（仿原 `PosData::SearchNN_XY` 但 generic 化 + slot map）：
//! - 實際 entry 資料存 `slots: Vec<Option<Slot<Id, Item>>>`（slot map），
//!   `free_slots: Vec<u32>` 收已釋放的 slot index 重用避免 Vec 一直長。
//! - `id_to_slot: HashMap<Id, u32>` 給 remove/update 反查 slot。
//! - `xs / ys: Vec<AxisRef>` 為 Copy 結構 `{ slot: u32, coord: f32 }`，
//!   實作 `Radixable<f32>` → 可以走 `voracious_mt_sort(4)` 並行 radix 排序。
//!
//! Query 流程：
//! 1. 兩軸 binary_search 找 [center-r, center+r] 範圍；
//! 2. X 範圍的 slots 收進 `BTreeSet<u32>`；
//! 3. 對 Y 範圍 iterate，slot 在 X set 才視為候選；
//! 4. 取 slots[slot] 的 position 算 distance，過濾 + clone Entry 回傳。

use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};
use std::hash::Hash;
use vek::Vec2;
use voracious_radix_sort::{Radixable, RadixSort};

use super::spatial_index::{Bounds, Entry, SpatialIndex};

/// 軸索引 entry：只存 slot index + coord，全 Copy 才能餵給 voracious。
#[derive(Copy, Clone, Debug)]
struct AxisRef {
    slot: u32,
    coord: f32,
}

// PartialOrd/PartialEq 只看 coord，與 PosXIndex 原模式一致。
// 注意：兩個 coord 相同的 AxisRef 會被視為相等（即使 slot 不同），
// binary_search 仍會工作但不保證找到特定 slot — 我們在 remove 時會線性掃同 coord 範圍。
impl PartialEq for AxisRef {
    fn eq(&self, other: &Self) -> bool { self.coord == other.coord }
}
impl PartialOrd for AxisRef {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.coord.partial_cmp(&other.coord)
    }
}
impl Radixable<f32> for AxisRef {
    type Key = f32;
    #[inline]
    fn key(&self) -> Self::Key { self.coord }
}

#[derive(Debug, Clone)]
struct Slot<Id, Item> {
    id: Id,
    item: Item,
    position: Vec2<f32>,
    bounding_radius: f32,
}

pub struct SweepAndPrune<Id, Item> {
    slots: Vec<Option<Slot<Id, Item>>>,
    free_slots: Vec<u32>,
    id_to_slot: HashMap<Id, u32>,
    xs: Vec<AxisRef>,
    ys: Vec<AxisRef>,
    /// 最大 bounding_radius 快取，用於 query 時擴張軸範圍避免漏 entry。
    /// 對 collision 用例 (Entity, ()) 永遠是 0；vision 用例會非 0。
    max_bounding_radius: f32,
}

impl<Id, Item> SweepAndPrune<Id, Item>
where
    Id: Clone + Eq + Hash + Ord + Send + Sync + 'static,
    Item: Clone + Send + Sync + 'static,
{
    pub fn new() -> Self {
        Self {
            slots: Vec::new(),
            free_slots: Vec::new(),
            id_to_slot: HashMap::new(),
            xs: Vec::new(),
            ys: Vec::new(),
            max_bounding_radius: 0.0,
        }
    }

    /// 配一個 slot index 給新 entry：優先重用 free_slots，否則 push 新 slot
    fn alloc_slot(&mut self, slot_data: Slot<Id, Item>) -> u32 {
        if let Some(idx) = self.free_slots.pop() {
            self.slots[idx as usize] = Some(slot_data);
            idx
        } else {
            let idx = self.slots.len() as u32;
            self.slots.push(Some(slot_data));
            idx
        }
    }

    /// 釋放 slot：標記成 None 並回收 index 到 free_slots
    fn free_slot(&mut self, idx: u32) {
        self.slots[idx as usize] = None;
        self.free_slots.push(idx);
    }

    /// 在已排序的 axis array 上插入 AxisRef 到正確位置
    fn axis_insert_sorted(arr: &mut Vec<AxisRef>, item: AxisRef) {
        let pos = arr
            .binary_search_by(|probe| {
                probe.coord
                    .partial_cmp(&item.coord)
                    .unwrap_or(Ordering::Equal)
                    .then(Ordering::Greater) // 同 coord 時插到後面，避免 instability
            })
            .unwrap_or_else(|i| i);
        arr.insert(pos, item);
    }

    /// 從已排序的 axis array 移除指定 slot 的 AxisRef（已知 coord）
    fn axis_remove(arr: &mut Vec<AxisRef>, slot: u32, coord: f32) {
        // binary_search 找到 coord 區段，線性掃 slot 比對
        let lower = arr
            .binary_search_by(|probe| probe.coord.partial_cmp(&coord).unwrap_or(Ordering::Equal))
            .unwrap_or_else(|i| i);
        // 從 lower 往兩側擴張找 slot 相符的元素（同 coord 通常 1 個，極端時可能多個）
        let mut l = lower;
        while l > 0 && arr[l - 1].coord == coord { l -= 1; }
        while l < arr.len() && arr[l].coord == coord {
            if arr[l].slot == slot {
                arr.remove(l);
                return;
            }
            l += 1;
        }
        // fallback: 線性掃（理論上不會走到，但浮點 NaN 等怪情況保險）
        if let Some(i) = arr.iter().position(|a| a.slot == slot) {
            arr.remove(i);
        }
    }

    /// query 時的 axis range：找 coord 落在 [lo, hi] 區間內的 [l, r) index
    fn axis_range(arr: &[AxisRef], lo: f32, hi: f32) -> (usize, usize) {
        // 用 binary_search_by 找下界 (>= lo) / 上界 (> hi)，搭配 saturate
        let l = arr.partition_point(|p| p.coord < lo);
        let r = arr.partition_point(|p| p.coord <= hi);
        (l, r)
    }

    /// 重新計算 max_bounding_radius（O(n)）。在 remove 時若移除的是 max 才需呼叫。
    fn recompute_max_radius(&mut self) {
        self.max_bounding_radius = self.slots.iter()
            .filter_map(|s| s.as_ref().map(|s| s.bounding_radius))
            .fold(0.0_f32, f32::max);
    }
}

impl<Id, Item> SpatialIndex<Id, Item> for SweepAndPrune<Id, Item>
where
    Id: Clone + Eq + Hash + Ord + Send + Sync + 'static,
    Item: Clone + Send + Sync + 'static,
{
    fn initialize(&mut self, _bounds: Bounds, entries: Vec<Entry<Id, Item>>) {
        self.slots.clear();
        self.free_slots.clear();
        self.id_to_slot.clear();
        self.xs.clear();
        self.ys.clear();
        self.max_bounding_radius = 0.0;

        // batch fill
        self.slots.reserve(entries.len());
        self.xs.reserve(entries.len());
        self.ys.reserve(entries.len());

        for entry in entries {
            let slot = self.slots.len() as u32;
            self.id_to_slot.insert(entry.id.clone(), slot);
            self.xs.push(AxisRef { slot, coord: entry.position.x });
            self.ys.push(AxisRef { slot, coord: entry.position.y });
            if entry.bounding_radius > self.max_bounding_radius {
                self.max_bounding_radius = entry.bounding_radius;
            }
            self.slots.push(Some(Slot {
                id: entry.id,
                item: entry.item,
                position: entry.position,
                bounding_radius: entry.bounding_radius,
            }));
        }

        // 並行 radix sort — 對應原 PosData 的 voracious_mt_sort(4) 用法
        if self.xs.len() >= 2 {
            self.xs.voracious_mt_sort(4);
            self.ys.voracious_mt_sort(4);
        }
    }

    fn insert(&mut self, entry: Entry<Id, Item>) {
        // 同 id 重插：先 remove 舊的，再 insert 新的
        if let Some(&old_slot) = self.id_to_slot.get(&entry.id) {
            if let Some(Some(old)) = self.slots.get(old_slot as usize) {
                let old_x = old.position.x;
                let old_y = old.position.y;
                Self::axis_remove(&mut self.xs, old_slot, old_x);
                Self::axis_remove(&mut self.ys, old_slot, old_y);
            }
            self.free_slot(old_slot);
            self.id_to_slot.remove(&entry.id);
            self.recompute_max_radius();
        }

        let r = entry.bounding_radius;
        let pos = entry.position;
        let id = entry.id.clone();
        let slot = self.alloc_slot(Slot {
            id: entry.id,
            item: entry.item,
            position: pos,
            bounding_radius: r,
        });
        self.id_to_slot.insert(id, slot);
        Self::axis_insert_sorted(&mut self.xs, AxisRef { slot, coord: pos.x });
        Self::axis_insert_sorted(&mut self.ys, AxisRef { slot, coord: pos.y });
        if r > self.max_bounding_radius {
            self.max_bounding_radius = r;
        }
    }

    fn remove(&mut self, id: &Id) -> bool {
        let slot = match self.id_to_slot.remove(id) {
            Some(s) => s,
            None => return false,
        };
        let (was_max, old_pos) = match self.slots.get(slot as usize) {
            Some(Some(s)) => {
                let was_max = s.bounding_radius >= self.max_bounding_radius;
                (was_max, s.position)
            }
            _ => return false,
        };
        Self::axis_remove(&mut self.xs, slot, old_pos.x);
        Self::axis_remove(&mut self.ys, slot, old_pos.y);
        self.free_slot(slot);
        if was_max {
            self.recompute_max_radius();
        }
        true
    }

    fn update(&mut self, entry: Entry<Id, Item>) {
        // remove + insert（共用 self.insert 已處理同 id 重插）
        self.insert(entry);
    }

    /// SAP 專屬的 incremental rebuild 路徑：保留 slot map / id_to_slot，只 diff 增減的部份；
    /// xs/ys 則完整重建並 voracious_mt_sort 一次（high-churn 場景下比 N 次 axis_insert 便宜很多）。
    ///
    /// 相對 default `bulk_replace = initialize`：避免 HashMap 全 clear 後再插回 N 筆造成的 bucket
    /// reallocation；slot index 對未變動的 entry 也保持穩定，便於上層做 cache（若需要）。
    fn bulk_replace(&mut self, _bounds: Bounds, entries: Vec<Entry<Id, Item>>) {
        use std::collections::HashSet;

        // 收集新 id set，後面用來找 to_remove
        let new_ids: HashSet<Id> = entries.iter().map(|e| e.id.clone()).collect();

        // Step 1: 移除舊 set 但不在新 set 的 entry（slot 釋放、不動 xs/ys，後面會整批重建）
        let to_remove: Vec<Id> = self.id_to_slot.keys()
            .filter(|id| !new_ids.contains(*id))
            .cloned()
            .collect();
        let mut removed_max_radius = false;
        for id in &to_remove {
            if let Some(slot) = self.id_to_slot.remove(id) {
                if let Some(Some(s)) = self.slots.get(slot as usize) {
                    if s.bounding_radius >= self.max_bounding_radius {
                        removed_max_radius = true;
                    }
                }
                self.slots[slot as usize] = None;
                self.free_slots.push(slot);
            }
        }

        // Step 2: 對新 entries — 已存在 id 就 in-place 更新 slot；否則 alloc 新 slot
        for entry in entries {
            let r = entry.bounding_radius;
            if let Some(&slot) = self.id_to_slot.get(&entry.id) {
                if let Some(slot_ref) = self.slots.get_mut(slot as usize) {
                    if let Some(s) = slot_ref.as_mut() {
                        s.item = entry.item;
                        s.position = entry.position;
                        s.bounding_radius = r;
                    }
                }
            } else {
                let id = entry.id.clone();
                let slot = self.alloc_slot(Slot {
                    id: entry.id,
                    item: entry.item,
                    position: entry.position,
                    bounding_radius: r,
                });
                self.id_to_slot.insert(id, slot);
            }
            if r > self.max_bounding_radius {
                self.max_bounding_radius = r;
            }
        }

        if removed_max_radius {
            self.recompute_max_radius();
        }

        // Step 3: 從目前 live slots 重建 xs/ys，再做 voracious_mt_sort 一次。
        // 比起對每個 axis_insert/remove 各做 O(n)，trash-and-resort 在 high-churn 場景線性勝出。
        self.xs.clear();
        self.ys.clear();
        self.xs.reserve(self.id_to_slot.len());
        self.ys.reserve(self.id_to_slot.len());
        for (idx, slot_opt) in self.slots.iter().enumerate() {
            if let Some(s) = slot_opt {
                self.xs.push(AxisRef { slot: idx as u32, coord: s.position.x });
                self.ys.push(AxisRef { slot: idx as u32, coord: s.position.y });
            }
        }
        if self.xs.len() >= 2 {
            self.xs.voracious_mt_sort(4);
            self.ys.voracious_mt_sort(4);
        }
    }

    fn query_in_range(&self, center: Vec2<f32>, radius: f32) -> Vec<Entry<Id, Item>> {
        let extended = radius + self.max_bounding_radius;
        let (lx, rx) = Self::axis_range(&self.xs, center.x - extended, center.x + extended);
        let (ly, ry) = Self::axis_range(&self.ys, center.y - extended, center.y + extended);

        // X 範圍 slots 集合
        let mut x_slots: BTreeSet<u32> = BTreeSet::new();
        for i in lx..rx {
            x_slots.insert(self.xs[i].slot);
        }

        // Y 範圍 iterate，slot 在 X set 才當候選
        let mut results: Vec<Entry<Id, Item>> = Vec::new();
        for i in ly..ry {
            let slot = self.ys[i].slot;
            if !x_slots.contains(&slot) {
                continue;
            }
            if let Some(Some(s)) = self.slots.get(slot as usize) {
                let extended_r = radius + s.bounding_radius.max(0.0);
                if s.position.distance(center) <= extended_r {
                    results.push(Entry {
                        id: s.id.clone(),
                        item: s.item.clone(),
                        position: s.position,
                        bounding_radius: s.bounding_radius,
                    });
                }
            }
        }
        results
    }

    fn count_nodes(&self) -> usize {
        self.id_to_slot.len()
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

        for w in s.xs.windows(2) {
            assert!(w[0].coord <= w[1].coord, "xs not sorted: {} > {}", w[0].coord, w[1].coord);
        }
        for w in s.ys.windows(2) {
            assert!(w[0].coord <= w[1].coord, "ys not sorted: {} > {}", w[0].coord, w[1].coord);
        }
        assert_eq!(s.xs.len(), 25);
        assert_eq!(s.ys.len(), 25);
        // free_slots 應有累積（被 remove 的 slot index）
        assert!(s.free_slots.len() >= 25);
    }

    #[test]
    fn query_handles_large_bounding_radius_extension() {
        let mut s: SweepAndPrune<String, ()> = SweepAndPrune::new();
        s.initialize(world_bounds(), vec![
            pt("big", 500.0, 500.0, 200.0),
            pt("far", 100.0, 100.0, 5.0),
        ]);

        let q = s.query_in_range(Vec2::new(700.0, 500.0), 50.0);
        assert_eq!(ids_of(&q), vec!["big"]);
    }

    #[test]
    fn voracious_sort_invariant_after_initialize() {
        // 大量 entry 用 voracious_mt_sort 排序後，xs/ys 必須完全 sorted
        let mut s: SweepAndPrune<String, ()> = SweepAndPrune::new();
        let entries: Vec<_> = (0..500).map(|i| {
            let x = ((i * 977 + 13) % 1000) as f32;
            let y = ((i * 31 + 7) % 1000) as f32;
            pt(&format!("e{}", i), x, y, 0.0)
        }).collect();
        s.initialize(world_bounds(), entries);

        for w in s.xs.windows(2) {
            assert!(w[0].coord <= w[1].coord, "xs voracious sort failed at {}", w[0].coord);
        }
        for w in s.ys.windows(2) {
            assert!(w[0].coord <= w[1].coord, "ys voracious sort failed at {}", w[0].coord);
        }
    }

    #[test]
    fn bulk_replace_keeps_unchanged_slots() {
        // bulk_replace 對 entity set 大致不變的情況，slot index 應該保持穩定（不會被當成新建）
        let mut s: SweepAndPrune<String, ()> = SweepAndPrune::new();
        s.initialize(world_bounds(), vec![
            pt("a", 100.0, 100.0, 5.0),
            pt("b", 500.0, 500.0, 5.0),
            pt("c", 900.0, 900.0, 5.0),
        ]);
        let slot_a_before = *s.id_to_slot.get(&"a".to_string()).unwrap();
        let slot_c_before = *s.id_to_slot.get(&"c".to_string()).unwrap();

        // bulk_replace：a 動了、b 不見了、c 不變、新增 d
        s.bulk_replace(world_bounds(), vec![
            pt("a", 200.0, 200.0, 5.0),
            pt("c", 900.0, 900.0, 5.0),
            pt("d", 700.0, 100.0, 5.0),
        ]);

        // a / c 的 slot index 應該保持不變（in-place update）
        assert_eq!(*s.id_to_slot.get(&"a".to_string()).unwrap(), slot_a_before,
            "a's slot identity should be preserved across bulk_replace");
        assert_eq!(*s.id_to_slot.get(&"c".to_string()).unwrap(), slot_c_before,
            "c's slot identity should be preserved across bulk_replace");
        assert!(!s.id_to_slot.contains_key(&"b".to_string()), "b should be removed");
        assert!(s.id_to_slot.contains_key(&"d".to_string()), "d should be added");

        // query 結果正確
        assert_eq!(ids_of(&s.query_in_range(Vec2::new(200.0, 200.0), 30.0)), vec!["a"]);
        assert!(ids_of(&s.query_in_range(Vec2::new(500.0, 500.0), 30.0)).is_empty());
        assert_eq!(ids_of(&s.query_in_range(Vec2::new(900.0, 900.0), 30.0)), vec!["c"]);
        assert_eq!(ids_of(&s.query_in_range(Vec2::new(700.0, 100.0), 30.0)), vec!["d"]);

        // xs/ys invariant: sorted
        for w in s.xs.windows(2) {
            assert!(w[0].coord <= w[1].coord);
        }
        for w in s.ys.windows(2) {
            assert!(w[0].coord <= w[1].coord);
        }
    }

    #[test]
    fn slot_reuse_after_remove() {
        // 移除後立即 insert 應該重用 slot index，slots Vec 不會無限長
        let mut s: SweepAndPrune<String, ()> = SweepAndPrune::new();
        s.initialize(world_bounds(), vec![pt("a", 100.0, 100.0, 5.0)]);
        let initial_slots_len = s.slots.len();

        for i in 0..10 {
            s.insert(pt(&format!("tmp{}", i), 200.0, 200.0, 5.0));
            assert!(s.remove(&format!("tmp{}", i)));
        }
        // 經過 10 次 insert+remove 後，slots Vec 不應該持續增長
        // initial 1 個 + 1 個 reuse slot（每次 insert 使用同一個 free slot）= 2 個
        assert!(s.slots.len() <= initial_slots_len + 1,
            "slot map grew unexpectedly: {} → {}", initial_slots_len, s.slots.len());
    }
}
