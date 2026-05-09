//! P5：每位玩家 AOI（感興趣區域）廣泛階段。
//!
//! 由傳輸廣播線程用來解析`BroadcastPolicy::AoiEntity`
//! 進入「(x, y)」座標而不觸及規格儲存（傳輸
//! 線程不包含“World”）。網格是一個輕量級、快取友善的儲存桶
//! 由「(cell_x, cell_y)」鍵入的查找，從相同的預收集中重建每個刻度
//! 該心跳已經在「State::send_heartbeat」中執行。
//!
//! 設計注意事項：
//! - `AOI_CELL_SIZE = 256.0` 與 omfx 使用的視窗半尺寸相符
//! （典型的焊盤區域約為 1024x768，因此每個視口接觸 16-32 個單元格 —
//! 小到足以進行線性掃描，大到足以保持「細胞」淺）。
//! - `rebuild` 取代每個刻度的網格內容；實體流失（生成/死亡）
//! 自然處理，無需單獨新增/刪除簿記。
//! - `positions` 保留為一個平面 `HashMap<u64, (f32,f32)>`，用於 O(1) 查找
//! 透過“entity_id”（策略快速路徑）。 「細胞」地圖是為未來而存在的
//! 半徑查詢 (`query(center, radius, cb)`) 傳輸不會
//! 目前正在調用，但需要在 P6 中進行寬相半徑/VFX 扇出。
//!
//! 資源是 `Arc<Mutex<AoiGrid>>` 因此傳輸執行緒 (tokio) 可以
//! 與遊戲循環 (rayon) 共享唯讀存取權限。便宜：重建是
//! 本質上是兩張平面地圖的重新分配。

use hashbrown::HashMap;

/// 遊戲世界單位中單一 AOI 單元的邊長。
/// 選擇 256，以便 1024x768 視口接觸 ≤ 16 個單元格（4x3 加填充）。
pub const AOI_CELL_SIZE: f32 = 256.0;

/// 單元內單一實體的位置快照。
#[derive(Copy, Clone, Debug)]
pub struct AoiEntry {
    pub entity_id: u64,
    pub pos: (f32, f32),
}

/// AOI 寬相網格。從預先收集的迭代器中每次更新重建。
///
/// 「cells」透過整數「(cell_x, cell_y)」儲存未來半徑的實體
/// 查詢。 `positions` 是一個平面的entity_id → (x,y) 找出熱路徑 —
/// `BroadcastPolicy::AoiEntity` 正是這一點。
#[derive(Default, Debug)]
pub struct AoiGrid {
    cells: HashMap<(i32, i32), Vec<AoiEntry>>,
    positions: HashMap<u64, (f32, f32)>,
}

impl AoiGrid {
    pub fn new() -> Self {
        Self {
            cells: HashMap::new(),
            positions: HashMap::new(),
        }
    }

    /// 一次性替換網格內容。調用者提供所有活著的
    /// 具有 (id, pos) 的實體 — 通常在同一迭代期間收集
    /// 心跳/可見度差異已經存在，因此成本被攤提。
    pub fn rebuild(&mut self, entries: impl IntoIterator<Item = AoiEntry>) {
        self.cells.clear();
        self.positions.clear();
        for entry in entries {
            self.positions.insert(entry.entity_id, entry.pos);
            let key = Self::cell_key(entry.pos);
            self.cells.entry(key).or_insert_with(Vec::new).push(entry);
        }
    }

    /// O(1) 透過entity_id 進行位置查找。如果實體不是，則傳回“None”
    /// 在最後一個重建快照中（例如，重建後產生此刻度，或者
    /// 已經死了）。傳輸策略處理程序將“無”視為“廣播”
    /// 所有會話」—比默默放棄事件更安全。
    #[inline]
    pub fn lookup_pos(&self, entity_id: u64) -> Option<(f32, f32)> {
        self.positions.get(&entity_id).copied()
    }

    /// 半徑查詢：對“radius”內的每個實體呼叫“cb(entity_id)”
    /// 的「中心」。用於 VFX 扇出/AOI 飛濺策略（保留用於
    /// P6；目前未被廣播線程呼叫）。
    pub fn query<F: FnMut(u64)>(&self, center: (f32, f32), radius: f32, mut cb: F) {
        let r2 = radius * radius;
        let (min_cx, min_cy) = Self::cell_key((center.0 - radius, center.1 - radius));
        let (max_cx, max_cy) = Self::cell_key((center.0 + radius, center.1 + radius));
        for cx in min_cx..=max_cx {
            for cy in min_cy..=max_cy {
                let Some(bucket) = self.cells.get(&(cx, cy)) else {
                    continue;
                };
                for e in bucket {
                    let dx = e.pos.0 - center.0;
                    let dy = e.pos.1 - center.1;
                    if dx * dx + dy * dy <= r2 {
                        cb(e.entity_id);
                    }
                }
            }
        }
    }

    /// 目前實體計數。對於診斷/測試很有用。
    pub fn len(&self) -> usize {
        self.positions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }

    #[inline]
    fn cell_key(pos: (f32, f32)) -> (i32, i32) {
        let cx = (pos.0 / AOI_CELL_SIZE).floor() as i32;
        let cy = (pos.1 / AOI_CELL_SIZE).floor() as i32;
        (cx, cy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u64, x: f32, y: f32) -> AoiEntry {
        AoiEntry {
            entity_id: id,
            pos: (x, y),
        }
    }

    #[test]
    fn rebuild_inserts_positions() {
        let mut g = AoiGrid::new();
        g.rebuild([entry(1, 10.0, 20.0), entry(2, 300.0, 400.0)]);
        assert_eq!(g.lookup_pos(1), Some((10.0, 20.0)));
        assert_eq!(g.lookup_pos(2), Some((300.0, 400.0)));
        assert_eq!(g.lookup_pos(99), None);
        assert_eq!(g.len(), 2);
    }

    #[test]
    fn rebuild_replaces_previous_contents() {
        let mut g = AoiGrid::new();
        g.rebuild([entry(1, 10.0, 20.0)]);
        g.rebuild([entry(2, 50.0, 60.0)]);
        assert_eq!(g.lookup_pos(1), None, "stale entry must be evicted");
        assert_eq!(g.lookup_pos(2), Some((50.0, 60.0)));
    }

    #[test]
    fn query_hits_entities_within_radius() {
        let mut g = AoiGrid::new();
        g.rebuild([
            entry(1, 100.0, 100.0), // inside radius from (120,120)
            entry(2, 130.0, 130.0), // inside
            entry(3, 500.0, 500.0), // outside
            entry(4, 120.0, 120.0), // at center
        ]);
        let mut hits: Vec<u64> = Vec::new();
        g.query((120.0, 120.0), 50.0, |id| hits.push(id));
        hits.sort();
        assert_eq!(hits, vec![1, 2, 4]);
    }

    #[test]
    fn query_empty_grid_yields_nothing() {
        let g = AoiGrid::new();
        let mut hits: Vec<u64> = Vec::new();
        g.query((0.0, 0.0), 1000.0, |id| hits.push(id));
        assert!(hits.is_empty());
    }

    #[test]
    fn cell_size_partitions_correctly() {
        // pos (0,0) 與 (AOI_CELL_SIZE, AOI_CELL_SIZE) 的單元格鍵必須不同。
        let mut g = AoiGrid::new();
        g.rebuild([
            entry(1, 0.0, 0.0),
            entry(2, AOI_CELL_SIZE + 1.0, AOI_CELL_SIZE + 1.0),
        ]);
        // (0,0) 周圍的小半徑只能找到實體 1。
        let mut hits: Vec<u64> = Vec::new();
        g.query((0.0, 0.0), 10.0, |id| hits.push(id));
        assert_eq!(hits, vec![1]);
    }

    #[test]
    fn negative_coordinates_are_supported() {
        let mut g = AoiGrid::new();
        g.rebuild([entry(1, -300.0, -300.0), entry(2, 300.0, 300.0)]);
        assert_eq!(g.lookup_pos(1), Some((-300.0, -300.0)));
        let mut hits: Vec<u64> = Vec::new();
        g.query((-290.0, -290.0), 100.0, |id| hits.push(id));
        assert_eq!(hits, vec![1]);
    }
}
