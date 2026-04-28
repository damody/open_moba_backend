//! Generic Spatial Hash Grid。對 (Id, Item) 為 generic。
//!
//! 設計：固定 cell_size，每個 entry 把它的 AABB（外接 bounding_radius）
//! 覆蓋的所有 cells 都 push 進去；同步維護 `id → cells` 反向映射讓 remove 不必掃整張表。

use std::collections::{BTreeSet, HashMap};
use std::hash::Hash;
use vek::Vec2;

use super::spatial_index::{Bounds, Entry, SpatialIndex};

type Cell = (i32, i32);

pub struct SpatialHashGrid<Id, Item> {
    cell_size: f32,
    cells: HashMap<Cell, Vec<Entry<Id, Item>>>,
    id_cells: HashMap<Id, Vec<Cell>>,
}

impl<Id, Item> SpatialHashGrid<Id, Item>
where
    Id: Clone + Eq + Hash + Ord + Send + Sync + 'static,
    Item: Clone + Send + Sync + 'static,
{
    pub fn new(cell_size: f32) -> Self {
        let cs = if cell_size > 0.0 { cell_size } else { 128.0 };
        Self {
            cell_size: cs,
            cells: HashMap::new(),
            id_cells: HashMap::new(),
        }
    }

    fn entry_aabb(entry: &Entry<Id, Item>) -> (Vec2<f32>, Vec2<f32>) {
        let pos = entry.position;
        let r = entry.bounding_radius.max(0.0);
        (Vec2::new(pos.x - r, pos.y - r), Vec2::new(pos.x + r, pos.y + r))
    }

    fn world_to_cell(&self, p: Vec2<f32>) -> Cell {
        ((p.x / self.cell_size).floor() as i32,
         (p.y / self.cell_size).floor() as i32)
    }

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

    fn insert_internal(&mut self, entry: Entry<Id, Item>) {
        let (min, max) = Self::entry_aabb(&entry);
        let cells = self.cells_for_aabb(min, max);
        for c in &cells {
            self.cells.entry(*c).or_insert_with(Vec::new).push(entry.clone());
        }
        self.id_cells.insert(entry.id.clone(), cells);
    }

    fn remove_internal(&mut self, id: &Id) -> bool {
        let cells = match self.id_cells.remove(id) {
            Some(c) => c,
            None => return false,
        };
        for c in cells {
            if let Some(bucket) = self.cells.get_mut(&c) {
                bucket.retain(|e| e.id != *id);
                if bucket.is_empty() {
                    self.cells.remove(&c);
                }
            }
        }
        true
    }
}

impl<Id, Item> SpatialIndex<Id, Item> for SpatialHashGrid<Id, Item>
where
    Id: Clone + Eq + Hash + Ord + Send + Sync + 'static,
    Item: Clone + Send + Sync + 'static,
{
    fn initialize(&mut self, _bounds: Bounds, entries: Vec<Entry<Id, Item>>) {
        self.cells.clear();
        self.id_cells.clear();
        for entry in entries {
            self.insert_internal(entry);
        }
    }

    fn insert(&mut self, entry: Entry<Id, Item>) {
        self.remove_internal(&entry.id);
        self.insert_internal(entry);
    }

    fn remove(&mut self, id: &Id) -> bool {
        self.remove_internal(id)
    }

    fn update(&mut self, entry: Entry<Id, Item>) {
        self.remove_internal(&entry.id);
        self.insert_internal(entry);
    }

    fn query_in_range(&self, center: Vec2<f32>, radius: f32) -> Vec<Entry<Id, Item>> {
        let qmin = Vec2::new(center.x - radius, center.y - radius);
        let qmax = Vec2::new(center.x + radius, center.y + radius);
        let cells = self.cells_for_aabb(qmin, qmax);

        let mut seen: BTreeSet<Id> = BTreeSet::new();
        let mut results: Vec<Entry<Id, Item>> = Vec::new();

        for c in cells {
            let bucket = match self.cells.get(&c) {
                Some(b) => b,
                None => continue,
            };
            for entry in bucket {
                if seen.contains(&entry.id) {
                    continue;
                }
                let extended = radius + entry.bounding_radius.max(0.0);
                if entry.position.distance(center) <= extended {
                    seen.insert(entry.id.clone());
                    results.push(entry.clone());
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
        let mut g: SpatialHashGrid<String, ()> = SpatialHashGrid::new(128.0);
        g.initialize(world_bounds(), vec![]);
        g.insert(pt("a", 100.0, 100.0, 10.0));
        g.insert(pt("b", 800.0, 800.0, 10.0));

        assert_eq!(ids_of(&g.query_in_range(Vec2::new(100.0, 100.0), 50.0)), vec!["a"]);
        assert_eq!(ids_of(&g.query_in_range(Vec2::new(800.0, 800.0), 50.0)), vec!["b"]);
    }

    #[test]
    fn remove_drops_entry_from_subsequent_queries() {
        let mut g: SpatialHashGrid<String, ()> = SpatialHashGrid::new(128.0);
        g.initialize(world_bounds(), vec![
            pt("a", 100.0, 100.0, 10.0),
            pt("b", 120.0, 110.0, 10.0),
        ]);
        assert_eq!(ids_of(&g.query_in_range(Vec2::new(110.0, 105.0), 100.0)), vec!["a", "b"]);
        assert!(g.remove(&"a".to_string()));
        assert_eq!(ids_of(&g.query_in_range(Vec2::new(110.0, 105.0), 100.0)), vec!["b"]);
        assert!(!g.remove(&"a".to_string()));
    }

    #[test]
    fn update_moves_entry_in_query_results() {
        let mut g: SpatialHashGrid<String, ()> = SpatialHashGrid::new(128.0);
        g.initialize(world_bounds(), vec![pt("mover", 100.0, 100.0, 5.0)]);
        assert_eq!(ids_of(&g.query_in_range(Vec2::new(100.0, 100.0), 20.0)), vec!["mover"]);

        g.update(pt("mover", 900.0, 900.0, 5.0));
        assert!(ids_of(&g.query_in_range(Vec2::new(100.0, 100.0), 20.0)).is_empty());
        assert_eq!(ids_of(&g.query_in_range(Vec2::new(900.0, 900.0), 20.0)), vec!["mover"]);
    }

    #[test]
    fn insert_distributes_to_multiple_cells_for_large_obstacle() {
        let mut g: SpatialHashGrid<String, ()> = SpatialHashGrid::new(64.0);
        g.initialize(world_bounds(), vec![]);
        g.insert(pt("big", 200.0, 200.0, 100.0));
        assert!(g.id_cells.get(&"big".to_string()).map(|v| v.len()).unwrap_or(0) >= 4,
                "large entry should occupy multiple cells");

        let q = g.query_in_range(Vec2::new(200.0, 200.0), 50.0);
        assert_eq!(ids_of(&q), vec!["big"]);
    }

    #[test]
    fn remove_dedupes_across_overlapping_cells() {
        let mut g: SpatialHashGrid<String, ()> = SpatialHashGrid::new(64.0);
        g.initialize(world_bounds(), vec![
            pt("spanner", 500.0, 500.0, 200.0),
            pt("filler1", 100.0, 100.0, 5.0),
            pt("filler2", 900.0, 900.0, 5.0),
        ]);

        let q = g.query_in_range(Vec2::new(500.0, 500.0), 50.0);
        assert_eq!(ids_of(&q), vec!["spanner"]);

        assert!(g.remove(&"spanner".to_string()));
        let q2 = g.query_in_range(Vec2::new(500.0, 500.0), 50.0);
        assert!(ids_of(&q2).is_empty());

        let q3 = g.query_in_range(Vec2::new(100.0, 100.0), 20.0);
        assert_eq!(ids_of(&q3), vec!["filler1"]);
    }
}
