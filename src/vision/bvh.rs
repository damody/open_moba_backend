//! Bounding Volume Hierarchy（SAH split）實作。
//!
//! Build 策略：top-down，每個 internal node 嘗試在 X / Y 兩軸上做 sort-then-sweep
//! 找最佳 split index（依 SAH cost 函數）；若所有 split 的 cost 都大於 leaf cost
//! 則直接停為 leaf。
//!
//! 動態更新策略：rebuild on mutation。維護 `id_index: HashMap<String, ObstacleInfo>`
//! 為 source of truth；insert/remove/update 改 id_index 之後 rebuild 整棵樹。
//! 對 vision 場景（塔毀 ~1Hz、obstacles hundreds 量級）夠快，比 refit / lazy 更穩定。

use std::collections::{BTreeSet, HashMap};
use vek::Vec2;

use crate::comp::circular_vision::{ObstacleInfo, ObstacleType};
use super::spatial_index::{Bounds, SpatialIndex, TreeEntry};

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

    fn from_obstacle(o: &ObstacleInfo) -> Self {
        let r = obstacle_bounding_radius(o);
        Self {
            min: Vec2::new(o.position.x - r, o.position.y - r),
            max: Vec2::new(o.position.x + r, o.position.y + r),
        }
    }

    fn union(&self, other: &Aabb) -> Aabb {
        Aabb {
            min: Vec2::new(self.min.x.min(other.min.x), self.min.y.min(other.min.y)),
            max: Vec2::new(self.max.x.max(other.max.x), self.max.y.max(other.max.y)),
        }
    }

    /// 2D 用 perimeter 代替 surface area（SAH 的 SA 在 2D 等同周長）
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
struct BvhNode {
    bounds: Aabb,
    /// leaf 時非空、internal 時為空
    entries: Vec<TreeEntry>,
    /// internal 時指向 nodes vec 的 index；leaf 時 = NIL
    left: u32,
    right: u32,
}

impl BvhNode {
    fn is_leaf(&self) -> bool {
        self.left == NIL && self.right == NIL
    }
}

pub struct Bvh {
    nodes: Vec<BvhNode>,
    /// source of truth：rebuild 從這裡讀
    id_index: HashMap<String, ObstacleInfo>,
    bounds: Option<Bounds>,
    max_leaf: usize,
}

impl Bvh {
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
            // 空樹：放一個 empty leaf 當 root，query 直接 miss
            self.nodes.push(BvhNode {
                bounds: Aabb::empty(),
                entries: Vec::new(),
                left: NIL,
                right: NIL,
            });
            return;
        }

        let entries: Vec<TreeEntry> = self.id_index.iter()
            .map(|(id, ob)| TreeEntry { id: id.clone(), obstacle: ob.clone() })
            .collect();

        // 預留 root index
        self.nodes.push(BvhNode {
            bounds: Aabb::empty(),
            entries: Vec::new(),
            left: NIL,
            right: NIL,
        });
        let max_leaf = self.max_leaf;
        Self::build_recursive(&mut self.nodes, 0, entries, max_leaf);
    }

    /// 在 nodes[node_idx] 處建立樹，遞迴 split entries。
    fn build_recursive(
        nodes: &mut Vec<BvhNode>,
        node_idx: usize,
        mut entries: Vec<TreeEntry>,
        max_leaf: usize,
    ) {
        // 計算 parent AABB
        let parent_aabb = entries.iter()
            .map(|e| Aabb::from_obstacle(&e.obstacle))
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
            // 按該軸的 obstacle 中心排序（穩定快排）
            entries.sort_by(|a, b| {
                let av = if axis == 0 { a.obstacle.position.x } else { a.obstacle.position.y };
                let bv = if axis == 0 { b.obstacle.position.x } else { b.obstacle.position.y };
                av.partial_cmp(&bv).unwrap_or(std::cmp::Ordering::Equal)
            });

            // 預先算 prefix / suffix AABB
            let aabbs: Vec<Aabb> = entries.iter()
                .map(|e| Aabb::from_obstacle(&e.obstacle))
                .collect();
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
            // SAH 找不到比 leaf 好的切法
            nodes[node_idx].entries = entries;
            return;
        }

        // 用最佳軸做最終排序、切割
        entries.sort_by(|a, b| {
            let av = if best_axis == 0 { a.obstacle.position.x } else { a.obstacle.position.y };
            let bv = if best_axis == 0 { b.obstacle.position.x } else { b.obstacle.position.y };
            av.partial_cmp(&bv).unwrap_or(std::cmp::Ordering::Equal)
        });

        let right_entries = entries.split_off(best_split);
        let left_entries = entries;

        // 配置 left/right child idx 後再遞迴
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

impl SpatialIndex for Bvh {
    fn initialize(&mut self, bounds: Bounds, entries: Vec<TreeEntry>) {
        self.id_index.clear();
        for e in entries {
            self.id_index.insert(e.id, e.obstacle);
        }
        self.bounds = Some(bounds);
        self.rebuild();
    }

    fn insert(&mut self, id: String, obstacle: ObstacleInfo) {
        self.id_index.insert(id, obstacle);
        self.rebuild();
    }

    fn remove(&mut self, id: &str) -> bool {
        if self.id_index.remove(id).is_some() {
            self.rebuild();
            true
        } else {
            false
        }
    }

    fn update(&mut self, id: &str, obstacle: ObstacleInfo) {
        self.id_index.insert(id.to_string(), obstacle);
        self.rebuild();
    }

    fn query_entries_in_range(&self, center: Vec2<f32>, radius: f32) -> Vec<(String, ObstacleInfo)> {
        if self.nodes.is_empty() {
            return Vec::new();
        }
        let query = Aabb::from_query(center, radius);
        let mut results: Vec<(String, ObstacleInfo)> = Vec::new();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut stack: Vec<u32> = vec![0];

        while let Some(idx) = stack.pop() {
            let node = &self.nodes[idx as usize];
            if !node.bounds.intersects(&query) {
                continue;
            }
            if node.is_leaf() {
                for entry in &node.entries {
                    if seen.contains(&entry.id) { continue; }
                    let extended = radius + obstacle_bounding_radius(&entry.obstacle);
                    if entry.obstacle.position.distance(center) <= extended {
                        seen.insert(entry.id.clone());
                        results.push((entry.id.clone(), entry.obstacle.clone()));
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
        let mut t = Bvh::new(2);
        t.initialize(world_bounds(), vec![]);
        t.insert("a".into(), obs(100.0, 100.0, 10.0));
        t.insert("b".into(), obs(800.0, 800.0, 10.0));

        assert_eq!(ids_of(&t.query_entries_in_range(Vec2::new(100.0, 100.0), 50.0)), vec!["a"]);
        assert_eq!(ids_of(&t.query_entries_in_range(Vec2::new(800.0, 800.0), 50.0)), vec!["b"]);
    }

    #[test]
    fn remove_drops_entry_from_subsequent_queries() {
        let mut t = Bvh::new(2);
        t.initialize(world_bounds(), vec![
            TreeEntry { id: "a".into(), obstacle: obs(100.0, 100.0, 10.0) },
            TreeEntry { id: "b".into(), obstacle: obs(120.0, 110.0, 10.0) },
        ]);
        assert_eq!(ids_of(&t.query_entries_in_range(Vec2::new(110.0, 105.0), 100.0)), vec!["a", "b"]);
        assert!(t.remove("a"));
        assert_eq!(ids_of(&t.query_entries_in_range(Vec2::new(110.0, 105.0), 100.0)), vec!["b"]);
        assert!(!t.remove("a"));
    }

    #[test]
    fn update_moves_entry_in_query_results() {
        let mut t = Bvh::new(2);
        t.initialize(world_bounds(), vec![
            TreeEntry { id: "mover".into(), obstacle: obs(100.0, 100.0, 5.0) },
        ]);
        assert_eq!(ids_of(&t.query_entries_in_range(Vec2::new(100.0, 100.0), 20.0)), vec!["mover"]);

        t.update("mover", obs(900.0, 900.0, 5.0));
        assert!(ids_of(&t.query_entries_in_range(Vec2::new(100.0, 100.0), 20.0)).is_empty());
        assert_eq!(ids_of(&t.query_entries_in_range(Vec2::new(900.0, 900.0), 20.0)), vec!["mover"]);
    }

    #[test]
    fn build_creates_internal_nodes_when_above_leaf_capacity() {
        let mut t = Bvh::new(2);
        let entries = vec![
            TreeEntry { id: "o0".into(), obstacle: obs(50.0, 50.0, 5.0) },
            TreeEntry { id: "o1".into(), obstacle: obs(950.0, 50.0, 5.0) },
            TreeEntry { id: "o2".into(), obstacle: obs(50.0, 950.0, 5.0) },
            TreeEntry { id: "o3".into(), obstacle: obs(950.0, 950.0, 5.0) },
            TreeEntry { id: "o4".into(), obstacle: obs(500.0, 500.0, 5.0) },
        ];
        t.initialize(world_bounds(), entries);
        assert!(t.count_nodes() > 1, "BVH 應分裂成多個 nodes，實際 {}", t.count_nodes());

        // 全部 5 個都應該被索引到
        let all = t.query_entries_in_range(Vec2::new(500.0, 500.0), 800.0);
        assert_eq!(all.len(), 5);
    }

    #[test]
    fn rebuild_after_remove_keeps_consistency() {
        // 重複 insert/remove 確認 rebuild 不會留陳腐 entry
        let mut t = Bvh::new(2);
        t.initialize(world_bounds(), vec![
            TreeEntry { id: "a".into(), obstacle: obs(100.0, 100.0, 10.0) },
            TreeEntry { id: "b".into(), obstacle: obs(500.0, 500.0, 10.0) },
            TreeEntry { id: "c".into(), obstacle: obs(900.0, 900.0, 10.0) },
        ]);
        assert!(t.remove("b"));
        t.insert("d".into(), obs(500.0, 500.0, 10.0));
        t.update("a", obs(110.0, 110.0, 10.0));

        let q = t.query_entries_in_range(Vec2::new(500.0, 500.0), 50.0);
        assert_eq!(ids_of(&q), vec!["d"]);
        let q2 = t.query_entries_in_range(Vec2::new(110.0, 110.0), 30.0);
        assert_eq!(ids_of(&q2), vec!["a"]);
        let q3 = t.query_entries_in_range(Vec2::new(900.0, 900.0), 30.0);
        assert_eq!(ids_of(&q3), vec!["c"]);
    }
}
