//! Generic Bounding Volume Hierarchy（SAH split）。對 (Id, Item) 為 generic。
//!
//! Build 策略：top-down，每個 internal node 嘗試在 X / Y 兩軸上做 sort-then-sweep
//! 找最佳 split index（依 SAH cost 函數）；若所有 split 的 cost 都大於 leaf cost
//! 則直接停為 leaf。
//!
//! 動態更新策略：rebuild on mutation。維護 `id_index: HashMap<Id, (Item, position, radius)>`
//! 為 source of truth；insert/remove/update 改 id_index 之後 rebuild 整棵樹。

use std::collections::{BTreeSet, HashMap};
use std::hash::Hash;
use vek::Vec2;

use super::spatial_index::{Bounds, Entry, SpatialIndex};

const NIL: u32 = u32::MAX;
const T_TRAVERSE: f32 = 1.0;
const T_INTERSECT: f32 = 2.0;

#[derive(Debug, Clone)]
struct Aabb {
    min: Vec2<f32>,
    max: Vec2<f32>,
}

impl Aabb {
    fn empty() -> Self {
        Self {
            min: Vec2::new(f32::INFINITY, f32::INFINITY),
            max: Vec2::new(f32::NEG_INFINITY, f32::NEG_INFINITY),
        }
    }

    fn from_entry<Id, Item>(e: &Entry<Id, Item>) -> Self {
        let r = e.bounding_radius.max(0.0);
        Self {
            min: Vec2::new(e.position.x - r, e.position.y - r),
            max: Vec2::new(e.position.x + r, e.position.y + r),
        }
    }

    fn union(&self, other: &Aabb) -> Aabb {
        Aabb {
            min: Vec2::new(self.min.x.min(other.min.x), self.min.y.min(other.min.y)),
            max: Vec2::new(self.max.x.max(other.max.x), self.max.y.max(other.max.y)),
        }
    }

    /// 2D 用 perimeter 代替 surface area
    fn perimeter(&self) -> f32 {
        let w = (self.max.x - self.min.x).max(0.0);
        let h = (self.max.y - self.min.y).max(0.0);
        2.0 * (w + h)
    }

    fn intersects(&self, other: &Aabb) -> bool {
        self.min.x <= other.max.x && self.max.x >= other.min.x &&
        self.min.y <= other.max.y && self.max.y >= other.min.y
    }

    fn from_query(center: Vec2<f32>, radius: f32) -> Self {
        Self {
            min: Vec2::new(center.x - radius, center.y - radius),
            max: Vec2::new(center.x + radius, center.y + radius),
        }
    }
}

#[derive(Debug, Clone)]
struct BvhNode<Id, Item> {
    bounds: Aabb,
    entries: Vec<Entry<Id, Item>>,
    left: u32,
    right: u32,
}

impl<Id, Item> BvhNode<Id, Item> {
    fn is_leaf(&self) -> bool {
        self.left == NIL && self.right == NIL
    }
}

pub struct Bvh<Id, Item> {
    nodes: Vec<BvhNode<Id, Item>>,
    /// source of truth: rebuild 從這裡讀
    id_index: HashMap<Id, (Item, Vec2<f32>, f32)>,
    bounds: Option<Bounds>,
    max_leaf: usize,
}

impl<Id, Item> Bvh<Id, Item>
where
    Id: Clone + Eq + Hash + Ord + Send + Sync + 'static,
    Item: Clone + Send + Sync + 'static,
{
    pub fn new(max_leaf: usize) -> Self {
        Self {
            nodes: Vec::new(),
            id_index: HashMap::new(),
            bounds: None,
            max_leaf: max_leaf.max(1),
        }
    }

    fn rebuild(&mut self) {
        self.nodes.clear();
        if self.id_index.is_empty() {
            self.nodes.push(BvhNode {
                bounds: Aabb::empty(),
                entries: Vec::new(),
                left: NIL,
                right: NIL,
            });
            return;
        }

        let entries: Vec<Entry<Id, Item>> = self.id_index.iter()
            .map(|(id, (item, pos, r))| Entry {
                id: id.clone(),
                item: item.clone(),
                position: *pos,
                bounding_radius: *r,
            })
            .collect();

        self.nodes.push(BvhNode {
            bounds: Aabb::empty(),
            entries: Vec::new(),
            left: NIL,
            right: NIL,
        });
        let max_leaf = self.max_leaf;
        Self::build_recursive(&mut self.nodes, 0, entries, max_leaf);
    }

    fn build_recursive(
        nodes: &mut Vec<BvhNode<Id, Item>>,
        node_idx: usize,
        mut entries: Vec<Entry<Id, Item>>,
        max_leaf: usize,
    ) {
        let parent_aabb = entries.iter()
            .map(|e| Aabb::from_entry(e))
            .fold(Aabb::empty(), |acc, a| acc.union(&a));

        nodes[node_idx].bounds = parent_aabb.clone();

        if entries.len() <= max_leaf {
            nodes[node_idx].entries = entries;
            return;
        }

        let parent_sa = parent_aabb.perimeter().max(1e-6);
        let leaf_cost = entries.len() as f32 * T_INTERSECT;

        let n = entries.len();
        let mut best_cost = f32::INFINITY;
        let mut best_axis = 0usize;
        let mut best_split = 1usize;

        for axis in 0..2 {
            entries.sort_by(|a, b| {
                let av = if axis == 0 { a.position.x } else { a.position.y };
                let bv = if axis == 0 { b.position.x } else { b.position.y };
                av.partial_cmp(&bv).unwrap_or(std::cmp::Ordering::Equal)
            });

            let aabbs: Vec<Aabb> = entries.iter().map(|e| Aabb::from_entry(e)).collect();
            let mut prefix: Vec<Aabb> = Vec::with_capacity(n);
            let mut acc = Aabb::empty();
            for a in &aabbs {
                acc = acc.union(a);
                prefix.push(acc.clone());
            }
            let mut suffix: Vec<Aabb> = vec![Aabb::empty(); n];
            let mut acc2 = Aabb::empty();
            for i in (0..n).rev() {
                acc2 = acc2.union(&aabbs[i]);
                suffix[i] = acc2.clone();
            }

            for split in 1..n {
                let left_sa = prefix[split - 1].perimeter();
                let right_sa = suffix[split].perimeter();
                let cost = T_TRAVERSE
                    + (left_sa / parent_sa) * (split as f32) * T_INTERSECT
                    + (right_sa / parent_sa) * ((n - split) as f32) * T_INTERSECT;
                if cost < best_cost {
                    best_cost = cost;
                    best_axis = axis;
                    best_split = split;
                }
            }
        }

        if best_cost >= leaf_cost {
            nodes[node_idx].entries = entries;
            return;
        }

        entries.sort_by(|a, b| {
            let av = if best_axis == 0 { a.position.x } else { a.position.y };
            let bv = if best_axis == 0 { b.position.x } else { b.position.y };
            av.partial_cmp(&bv).unwrap_or(std::cmp::Ordering::Equal)
        });

        let right_entries = entries.split_off(best_split);
        let left_entries = entries;

        let left_idx = nodes.len() as u32;
        nodes.push(BvhNode {
            bounds: Aabb::empty(),
            entries: Vec::new(),
            left: NIL,
            right: NIL,
        });
        let right_idx = nodes.len() as u32;
        nodes.push(BvhNode {
            bounds: Aabb::empty(),
            entries: Vec::new(),
            left: NIL,
            right: NIL,
        });

        nodes[node_idx].left = left_idx;
        nodes[node_idx].right = right_idx;

        Self::build_recursive(nodes, left_idx as usize, left_entries, max_leaf);
        Self::build_recursive(nodes, right_idx as usize, right_entries, max_leaf);
    }
}

impl<Id, Item> SpatialIndex<Id, Item> for Bvh<Id, Item>
where
    Id: Clone + Eq + Hash + Ord + Send + Sync + 'static,
    Item: Clone + Send + Sync + 'static,
{
    fn initialize(&mut self, bounds: Bounds, entries: Vec<Entry<Id, Item>>) {
        self.id_index.clear();
        for e in entries {
            self.id_index.insert(e.id, (e.item, e.position, e.bounding_radius));
        }
        self.bounds = Some(bounds);
        self.rebuild();
    }

    fn insert(&mut self, entry: Entry<Id, Item>) {
        self.id_index.insert(entry.id, (entry.item, entry.position, entry.bounding_radius));
        self.rebuild();
    }

    fn remove(&mut self, id: &Id) -> bool {
        if self.id_index.remove(id).is_some() {
            self.rebuild();
            true
        } else {
            false
        }
    }

    fn update(&mut self, entry: Entry<Id, Item>) {
        self.id_index.insert(entry.id, (entry.item, entry.position, entry.bounding_radius));
        self.rebuild();
    }

    fn query_in_range(&self, center: Vec2<f32>, radius: f32) -> Vec<Entry<Id, Item>> {
        if self.nodes.is_empty() {
            return Vec::new();
        }
        let query = Aabb::from_query(center, radius);
        let mut results: Vec<Entry<Id, Item>> = Vec::new();
        let mut seen: BTreeSet<Id> = BTreeSet::new();
        let mut stack: Vec<u32> = vec![0];

        while let Some(idx) = stack.pop() {
            let node = &self.nodes[idx as usize];
            if !node.bounds.intersects(&query) {
                continue;
            }
            if node.is_leaf() {
                for entry in &node.entries {
                    if seen.contains(&entry.id) { continue; }
                    let extended = radius + entry.bounding_radius.max(0.0);
                    if entry.position.distance(center) <= extended {
                        seen.insert(entry.id.clone());
                        results.push(entry.clone());
                    }
                }
            } else {
                if node.left != NIL { stack.push(node.left); }
                if node.right != NIL { stack.push(node.right); }
            }
        }
        results
    }

    fn count_nodes(&self) -> usize {
        self.nodes.len()
    }

    fn name(&self) -> &'static str { "bvh" }
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
        let mut t: Bvh<String, ()> = Bvh::new(2);
        t.initialize(world_bounds(), vec![]);
        t.insert(pt("a", 100.0, 100.0, 10.0));
        t.insert(pt("b", 800.0, 800.0, 10.0));

        assert_eq!(ids_of(&t.query_in_range(Vec2::new(100.0, 100.0), 50.0)), vec!["a"]);
        assert_eq!(ids_of(&t.query_in_range(Vec2::new(800.0, 800.0), 50.0)), vec!["b"]);
    }

    #[test]
    fn remove_drops_entry_from_subsequent_queries() {
        let mut t: Bvh<String, ()> = Bvh::new(2);
        t.initialize(world_bounds(), vec![
            pt("a", 100.0, 100.0, 10.0),
            pt("b", 120.0, 110.0, 10.0),
        ]);
        assert_eq!(ids_of(&t.query_in_range(Vec2::new(110.0, 105.0), 100.0)), vec!["a", "b"]);
        assert!(t.remove(&"a".to_string()));
        assert_eq!(ids_of(&t.query_in_range(Vec2::new(110.0, 105.0), 100.0)), vec!["b"]);
        assert!(!t.remove(&"a".to_string()));
    }

    #[test]
    fn update_moves_entry_in_query_results() {
        let mut t: Bvh<String, ()> = Bvh::new(2);
        t.initialize(world_bounds(), vec![pt("mover", 100.0, 100.0, 5.0)]);
        assert_eq!(ids_of(&t.query_in_range(Vec2::new(100.0, 100.0), 20.0)), vec!["mover"]);

        t.update(pt("mover", 900.0, 900.0, 5.0));
        assert!(ids_of(&t.query_in_range(Vec2::new(100.0, 100.0), 20.0)).is_empty());
        assert_eq!(ids_of(&t.query_in_range(Vec2::new(900.0, 900.0), 20.0)), vec!["mover"]);
    }

    #[test]
    fn build_creates_internal_nodes_when_above_leaf_capacity() {
        let mut t: Bvh<String, ()> = Bvh::new(2);
        let entries = vec![
            pt("o0", 50.0, 50.0, 5.0),
            pt("o1", 950.0, 50.0, 5.0),
            pt("o2", 50.0, 950.0, 5.0),
            pt("o3", 950.0, 950.0, 5.0),
            pt("o4", 500.0, 500.0, 5.0),
        ];
        t.initialize(world_bounds(), entries);
        assert!(t.count_nodes() > 1);

        let all = t.query_in_range(Vec2::new(500.0, 500.0), 800.0);
        assert_eq!(all.len(), 5);
    }

    #[test]
    fn rebuild_after_remove_keeps_consistency() {
        let mut t: Bvh<String, ()> = Bvh::new(2);
        t.initialize(world_bounds(), vec![
            pt("a", 100.0, 100.0, 10.0),
            pt("b", 500.0, 500.0, 10.0),
            pt("c", 900.0, 900.0, 10.0),
        ]);
        assert!(t.remove(&"b".to_string()));
        t.insert(pt("d", 500.0, 500.0, 10.0));
        t.update(pt("a", 110.0, 110.0, 10.0));

        let q = t.query_in_range(Vec2::new(500.0, 500.0), 50.0);
        assert_eq!(ids_of(&q), vec!["d"]);
        let q2 = t.query_in_range(Vec2::new(110.0, 110.0), 30.0);
        assert_eq!(ids_of(&q2), vec!["a"]);
        let q3 = t.query_in_range(Vec2::new(900.0, 900.0), 30.0);
        assert_eq!(ids_of(&q3), vec!["c"]);
    }
}
