//! Spatial Hash Grid 實作。
//!
//! 設計：固定 cell_size，每個 entry 把它的 AABB 覆蓋的所有 cells 都 push 進去；
//! 同步維護 `id → cells` 的反向映射讓 remove 不必掃整張表。
//! 對均勻分布且物件大小相近的場景比 QuadTree 快；對長尾大物件會浪費。

use std::collections::{BTreeSet, HashMap};
use vek::Vec2;

use crate::comp::circular_vision::{ObstacleInfo, ObstacleType};
use super::spatial_index::{Bounds, SpatialIndex, TreeEntry};

type Cell = (i32, i32);

pub struct SpatialHashGrid {
    cell_size: f32,
    /// cell coord → entries（同 entry 跨多 cell 會 clone 進每一格）
    cells: HashMap<Cell, Vec<TreeEntry>>,
    /// id → 該 entry 占據的所有 cell 座標（給 remove 用，避免掃整張 cells）
    id_cells: HashMap<String, Vec<Cell>>,
}

impl SpatialHashGrid {
    pub fn new(cell_size: f32) -> Self {
        let cs = if cell_size > 0.0 { cell_size } else { 128.0 };
        Self {
            cell_size: cs,
            cells: HashMap::new(),
            id_cells: HashMap::new(),
        }
    }

    /// 取得 entry 的外接 AABB（位置 ± bounding_radius）
    fn entry_aabb(obstacle: &ObstacleInfo) -> (Vec2<f32>, Vec2<f32>) {
        let pos = obstacle.position;
        let r = obstacle_bounding_radius(obstacle);
        (Vec2::new(pos.x - r, pos.y - r), Vec2::new(pos.x + r, pos.y + r))
    }

    fn world_to_cell(&self, p: Vec2<f32>) -> Cell {
        ((p.x / self.cell_size).floor() as i32,
         (p.y / self.cell_size).floor() as i32)
    }

    /// 列出 AABB 覆蓋到的所有 cell 座標
    fn cells_for_aabb(&self, min: Vec2<f32>, max: Vec2<f32>) -> Vec<Cell> {
        let (cx0, cy0) = self.world_to_cell(min);
        let (cx1, cy1) = self.world_to_cell(max);
        let mut out = Vec::with_capacity(((cx1 - cx0 + 1) * (cy1 - cy0 + 1)).max(1) as usize);
        for cy in cy0..=cy1 {
            for cx in cx0..=cx1 {
                out.push((cx, cy));
            }
        }
        out
    }

    fn insert_internal(&mut self, entry: TreeEntry) {
        let (min, max) = Self::entry_aabb(&entry.obstacle);
        let cells = self.cells_for_aabb(min, max);
        for c in &cells {
            self.cells.entry(*c).or_insert_with(Vec::new).push(entry.clone());
        }
        self.id_cells.insert(entry.id.clone(), cells);
    }

    fn remove_internal(&mut self, id: &str) -> bool {
        let cells = match self.id_cells.remove(id) {
            Some(c) => c,
            None => return false,
        };
        for c in cells {
            if let Some(bucket) = self.cells.get_mut(&c) {
                bucket.retain(|e| e.id != id);
                if bucket.is_empty() {
                    self.cells.remove(&c);
                }
            }
        }
        true
    }
}

impl SpatialIndex for SpatialHashGrid {
    fn initialize(&mut self, _bounds: Bounds, entries: Vec<TreeEntry>) {
        // SHG 不需要世界邊界（hash key 直接用 floor div 算），bounds 忽略
        self.cells.clear();
        self.id_cells.clear();
        for entry in entries {
            self.insert_internal(entry);
        }
    }

    fn insert(&mut self, id: String, obstacle: ObstacleInfo) {
        // 同 id 重插：先清舊的（以 update 語意為準）
        self.remove_internal(&id);
        self.insert_internal(TreeEntry { id, obstacle });
    }

    fn remove(&mut self, id: &str) -> bool {
        self.remove_internal(id)
    }

    fn update(&mut self, id: &str, obstacle: ObstacleInfo) {
        self.remove_internal(id);
        self.insert_internal(TreeEntry { id: id.to_string(), obstacle });
    }

    fn query_entries_in_range(&self, center: Vec2<f32>, radius: f32) -> Vec<(String, ObstacleInfo)> {
        let qmin = Vec2::new(center.x - radius, center.y - radius);
        let qmax = Vec2::new(center.x + radius, center.y + radius);
        let cells = self.cells_for_aabb(qmin, qmax);

        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut results: Vec<(String, ObstacleInfo)> = Vec::new();

        for c in cells {
            let bucket = match self.cells.get(&c) {
                Some(b) => b,
                None => continue,
            };
            for entry in bucket {
                if seen.contains(&entry.id) {
                    continue;
                }
                let extended = radius + obstacle_bounding_radius(&entry.obstacle);
                if entry.obstacle.position.distance(center) <= extended {
                    seen.insert(entry.id.clone());
                    results.push((entry.id.clone(), entry.obstacle.clone()));
                }
            }
        }
        results
    }

    fn count_nodes(&self) -> usize {
        self.cells.len()
    }

    fn name(&self) -> &'static str { "hash_grid" }
}

/// 障礙物的外接半徑，用於範圍判斷與 AABB 覆蓋。
/// 與 QuadTree::obstacle_intersects_bounds 的 extended_range 邏輯保持一致。
fn obstacle_bounding_radius(obstacle: &ObstacleInfo) -> f32 {
    match &obstacle.obstacle_type {
        ObstacleType::Circular { radius } => *radius,
        ObstacleType::Rectangle { width, height, .. } => {
            (width * width + height * height).sqrt() * 0.5
        }
        ObstacleType::Terrain { .. } => 50.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comp::circular_vision::ObstacleProperties;

    fn obs(x: f32, y: f32, r: f32) -> ObstacleInfo {
        ObstacleInfo {
            position: Vec2::new(x, y),
            obstacle_type: ObstacleType::Circular { radius: r },
            height: 10.0,
            properties: ObstacleProperties {
                blocks_completely: true,
                opacity: 1.0,
                shadow_multiplier: 1.0,
            },
        }
    }

    fn world_bounds() -> Bounds {
        Bounds::new(Vec2::new(0.0, 0.0), Vec2::new(1000.0, 1000.0))
    }

    fn ids_of(results: &[(String, ObstacleInfo)]) -> Vec<String> {
        let mut v: Vec<String> = results.iter().map(|(id, _)| id.clone()).collect();
        v.sort();
        v
    }

    #[test]
    fn insert_into_initialized_then_query() {
        let mut g = SpatialHashGrid::new(128.0);
        g.initialize(world_bounds(), vec![]);
        g.insert("a".into(), obs(100.0, 100.0, 10.0));
        g.insert("b".into(), obs(800.0, 800.0, 10.0));

        assert_eq!(ids_of(&g.query_entries_in_range(Vec2::new(100.0, 100.0), 50.0)), vec!["a"]);
        assert_eq!(ids_of(&g.query_entries_in_range(Vec2::new(800.0, 800.0), 50.0)), vec!["b"]);
    }

    #[test]
    fn remove_drops_entry_from_subsequent_queries() {
        let mut g = SpatialHashGrid::new(128.0);
        g.initialize(world_bounds(), vec![
            TreeEntry { id: "a".into(), obstacle: obs(100.0, 100.0, 10.0) },
            TreeEntry { id: "b".into(), obstacle: obs(120.0, 110.0, 10.0) },
        ]);

        assert_eq!(ids_of(&g.query_entries_in_range(Vec2::new(110.0, 105.0), 100.0)), vec!["a", "b"]);
        assert!(g.remove("a"));
        assert_eq!(ids_of(&g.query_entries_in_range(Vec2::new(110.0, 105.0), 100.0)), vec!["b"]);
        assert!(!g.remove("a"));
    }

    #[test]
    fn update_moves_entry_in_query_results() {
        let mut g = SpatialHashGrid::new(128.0);
        g.initialize(world_bounds(), vec![
            TreeEntry { id: "mover".into(), obstacle: obs(100.0, 100.0, 5.0) },
        ]);
        assert_eq!(ids_of(&g.query_entries_in_range(Vec2::new(100.0, 100.0), 20.0)), vec!["mover"]);

        g.update("mover", obs(900.0, 900.0, 5.0));
        assert!(ids_of(&g.query_entries_in_range(Vec2::new(100.0, 100.0), 20.0)).is_empty());
        assert_eq!(ids_of(&g.query_entries_in_range(Vec2::new(900.0, 900.0), 20.0)), vec!["mover"]);
    }

    #[test]
    fn insert_distributes_to_multiple_cells_for_large_obstacle() {
        let mut g = SpatialHashGrid::new(64.0);
        g.initialize(world_bounds(), vec![]);
        // 半徑 100 的 obstacle 在 cell_size=64 下會橫跨 4 cells（中心 (200, 200)，AABB 100..300 → cell 1..4）
        g.insert("big".into(), obs(200.0, 200.0, 100.0));
        // 至少跨 2 cell（不限定確切數值，只驗 invariant：跨 cell）
        assert!(g.id_cells.get("big").map(|v| v.len()).unwrap_or(0) >= 4,
                "large obstacle should occupy multiple cells, got {:?}",
                g.id_cells.get("big").map(|v| v.len()));

        // 而且查詢時 dedup 不會回傳同一 id 多次
        let q = g.query_entries_in_range(Vec2::new(200.0, 200.0), 50.0);
        assert_eq!(ids_of(&q), vec!["big"]);
    }

    #[test]
    fn remove_dedupes_across_overlapping_cells() {
        let mut g = SpatialHashGrid::new(64.0);
        g.initialize(world_bounds(), vec![
            TreeEntry { id: "spanner".into(), obstacle: obs(500.0, 500.0, 200.0) },
            TreeEntry { id: "filler1".into(), obstacle: obs(100.0, 100.0, 5.0) },
            TreeEntry { id: "filler2".into(), obstacle: obs(900.0, 900.0, 5.0) },
        ]);

        let q = g.query_entries_in_range(Vec2::new(500.0, 500.0), 50.0);
        assert_eq!(ids_of(&q), vec!["spanner"]);

        assert!(g.remove("spanner"));
        let q2 = g.query_entries_in_range(Vec2::new(500.0, 500.0), 50.0);
        assert!(ids_of(&q2).is_empty());

        let q3 = g.query_entries_in_range(Vec2::new(100.0, 100.0), 20.0);
        assert_eq!(ids_of(&q3), vec!["filler1"]);
    }
}
