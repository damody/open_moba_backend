//! Generic QuadTree spatial index。對 (Id, Item) 為 generic，由 spatial_index trait 統一介面。
//!
//! 演算法：top-down 自分裂，葉節點 entry 數超過 `max_obstacles_per_node` 即 subdivide
//! 成 NW/NE/SW/SE 四子節點，把 entries 全部下推。一個 entry 若跨多個子節點 bounds
//! 會在每個相交子節點各 clone 一份；query 時用 BTreeSet<Id> 去重。
//!
//! 移除策略：retain by id 全樹掃過；不 collapse 子節點以避免 churn。

use std::collections::BTreeSet;
use std::hash::Hash;
use vek::Vec2;

use super::spatial_index::{Bounds, Entry, SpatialIndex};

/// 四叉樹節點
#[derive(Debug, Clone)]
pub struct QuadTreeNode<Id, Item> {
    pub bounds: Bounds,
    pub children: Option<Box<[QuadTreeNode<Id, Item>; 4]>>,
    pub entries: Vec<Entry<Id, Item>>,
    pub depth: usize,
}

pub struct QuadTree<Id, Item> {
    pub root: Option<QuadTreeNode<Id, Item>>,
    pub max_tree_depth: usize,
    pub max_obstacles_per_node: usize,
}

impl<Id, Item> QuadTree<Id, Item>
where
    Id: Clone + Eq + Hash + Ord + Send + Sync + 'static,
    Item: Clone + Send + Sync + 'static,
{
    pub fn new(max_tree_depth: usize, max_obstacles_per_node: usize) -> Self {
        Self {
            root: None,
            max_tree_depth,
            max_obstacles_per_node,
        }
    }

    fn insert_into(
        node: &mut QuadTreeNode<Id, Item>,
        entry: Entry<Id, Item>,
        max_depth: usize,
        max_per_node: usize,
    ) {
        if node.children.is_some() {
            if let Some(children) = node.children.as_mut() {
                for child in children.iter_mut() {
                    if Self::entry_intersects_bounds(&entry, &child.bounds) {
                        Self::insert_into(child, entry.clone(), max_depth, max_per_node);
                    }
                }
            }
            return;
        }

        node.entries.push(entry);
        if node.entries.len() > max_per_node && node.depth < max_depth {
            Self::subdivide_node(node, max_depth, max_per_node);
        }
    }

    fn remove_from(node: &mut QuadTreeNode<Id, Item>, id: &Id) -> bool {
        let before = node.entries.len();
        node.entries.retain(|e| e.id != *id);
        let mut removed = node.entries.len() != before;

        if let Some(children) = node.children.as_mut() {
            for child in children.iter_mut() {
                if Self::remove_from(child, id) {
                    removed = true;
                }
            }
        }
        removed
    }

    fn subdivide_node(node: &mut QuadTreeNode<Id, Item>, max_depth: usize, max_per_node: usize) {
        if node.entries.len() <= max_per_node || node.depth >= max_depth {
            return;
        }

        let bounds = &node.bounds;
        let mid_x = (bounds.min.x + bounds.max.x) * 0.5;
        let mid_y = (bounds.min.y + bounds.max.y) * 0.5;

        let mut children = Box::new([
            QuadTreeNode {
                bounds: Bounds {
                    min: Vec2::new(bounds.min.x, mid_y),
                    max: Vec2::new(mid_x, bounds.max.y),
                },
                children: None, entries: Vec::new(), depth: node.depth + 1,
            },
            QuadTreeNode {
                bounds: Bounds {
                    min: Vec2::new(mid_x, mid_y),
                    max: bounds.max.clone(),
                },
                children: None, entries: Vec::new(), depth: node.depth + 1,
            },
            QuadTreeNode {
                bounds: Bounds {
                    min: bounds.min.clone(),
                    max: Vec2::new(mid_x, mid_y),
                },
                children: None, entries: Vec::new(), depth: node.depth + 1,
            },
            QuadTreeNode {
                bounds: Bounds {
                    min: Vec2::new(mid_x, bounds.min.y),
                    max: Vec2::new(bounds.max.x, mid_y),
                },
                children: None, entries: Vec::new(), depth: node.depth + 1,
            },
        ]);

        for entry in &node.entries {
            for child in children.iter_mut() {
                if Self::entry_intersects_bounds(entry, &child.bounds) {
                    child.entries.push(entry.clone());
                }
            }
        }

        node.children = Some(children);
        node.entries.clear();

        if let Some(ref mut children) = node.children {
            for child in children.iter_mut() {
                Self::subdivide_node(child, max_depth, max_per_node);
            }
        }
    }

    /// 用 entry.position + bounding_radius 算外接圓 vs AABB
    fn entry_intersects_bounds(entry: &Entry<Id, Item>, bounds: &Bounds) -> bool {
        let pos = entry.position;
        let r = entry.bounding_radius.max(0.0);
        let closest_x = pos.x.max(bounds.min.x).min(bounds.max.x);
        let closest_y = pos.y.max(bounds.min.y).min(bounds.max.y);
        let distance = pos.distance(Vec2::new(closest_x, closest_y));
        distance <= r || bounds.contains_point(pos)
    }

    fn bounds_intersect(b1: &Bounds, b2: &Bounds) -> bool {
        b1.min.x <= b2.max.x && b1.max.x >= b2.min.x &&
        b1.min.y <= b2.max.y && b1.max.y >= b2.min.y
    }

    fn count_nodes_recursive(node: &QuadTreeNode<Id, Item>) -> usize {
        let mut count = 1;
        if let Some(ref children) = node.children {
            for child in children.iter() {
                count += Self::count_nodes_recursive(child);
            }
        }
        count
    }

    fn query_node_recursive(
        node: &QuadTreeNode<Id, Item>,
        query_bounds: &Bounds,
        center: Vec2<f32>,
        radius: f32,
        results: &mut Vec<Entry<Id, Item>>,
        seen: &mut BTreeSet<Id>,
    ) {
        if !Self::bounds_intersect(&node.bounds, query_bounds) {
            return;
        }

        for entry in &node.entries {
            if seen.contains(&entry.id) {
                continue;
            }
            let extended = radius + entry.bounding_radius.max(0.0);
            if entry.position.distance(center) <= extended {
                seen.insert(entry.id.clone());
                results.push(entry.clone());
            }
        }

        if let Some(ref children) = node.children {
            for child in children.iter() {
                Self::query_node_recursive(child, query_bounds, center, radius, results, seen);
            }
        }
    }
}

impl<Id, Item> SpatialIndex<Id, Item> for QuadTree<Id, Item>
where
    Id: Clone + Eq + Hash + Ord + Send + Sync + 'static,
    Item: Clone + Send + Sync + 'static,
{
    fn initialize(&mut self, world_bounds: Bounds, entries: Vec<Entry<Id, Item>>) {
        let mut root = QuadTreeNode {
            bounds: world_bounds,
            children: None,
            entries,
            depth: 0,
        };
        Self::subdivide_node(&mut root, self.max_tree_depth, self.max_obstacles_per_node);
        self.root = Some(root);
    }

    fn insert(&mut self, entry: Entry<Id, Item>) {
        let max_depth = self.max_tree_depth;
        let max_per_node = self.max_obstacles_per_node;
        if let Some(root) = self.root.as_mut() {
            Self::insert_into(root, entry, max_depth, max_per_node);
        }
    }

    fn remove(&mut self, id: &Id) -> bool {
        if let Some(root) = self.root.as_mut() {
            Self::remove_from(root, id)
        } else {
            false
        }
    }

    fn update(&mut self, entry: Entry<Id, Item>) {
        self.remove(&entry.id);
        self.insert(entry);
    }

    fn query_in_range(&self, center: Vec2<f32>, radius: f32) -> Vec<Entry<Id, Item>> {
        let mut results = Vec::new();
        let mut seen: BTreeSet<Id> = BTreeSet::new();
        if let Some(ref tree) = self.root {
            let query_bounds = Bounds {
                min: center - Vec2::new(radius, radius),
                max: center + Vec2::new(radius, radius),
            };
            Self::query_node_recursive(tree, &query_bounds, center, radius, &mut results, &mut seen);
        }
        results
    }

    fn count_nodes(&self) -> usize {
        if let Some(ref root) = self.root {
            Self::count_nodes_recursive(root)
        } else {
            0
        }
    }

    fn name(&self) -> &'static str { "quadtree" }
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
    fn insert_into_initialized_tree_then_query() {
        let mut tree: QuadTree<String, ()> = QuadTree::new(4, 4);
        tree.initialize(world_bounds(), vec![]);

        tree.insert(pt("a", 100.0, 100.0, 10.0));
        tree.insert(pt("b", 800.0, 800.0, 10.0));

        assert_eq!(ids_of(&tree.query_in_range(Vec2::new(100.0, 100.0), 50.0)), vec!["a"]);
        assert_eq!(ids_of(&tree.query_in_range(Vec2::new(800.0, 800.0), 50.0)), vec!["b"]);
    }

    #[test]
    fn remove_drops_entry_from_subsequent_queries() {
        let mut tree: QuadTree<String, ()> = QuadTree::new(4, 4);
        tree.initialize(world_bounds(), vec![
            pt("a", 100.0, 100.0, 10.0),
            pt("b", 120.0, 110.0, 10.0),
        ]);
        assert_eq!(ids_of(&tree.query_in_range(Vec2::new(110.0, 105.0), 100.0)), vec!["a", "b"]);
        assert!(tree.remove(&"a".to_string()));
        assert_eq!(ids_of(&tree.query_in_range(Vec2::new(110.0, 105.0), 100.0)), vec!["b"]);
        assert!(!tree.remove(&"a".to_string()));
    }

    #[test]
    fn update_moves_entry_in_query_results() {
        let mut tree: QuadTree<String, ()> = QuadTree::new(4, 4);
        tree.initialize(world_bounds(), vec![pt("mover", 100.0, 100.0, 5.0)]);
        assert_eq!(ids_of(&tree.query_in_range(Vec2::new(100.0, 100.0), 20.0)), vec!["mover"]);

        tree.update(pt("mover", 900.0, 900.0, 5.0));
        assert!(ids_of(&tree.query_in_range(Vec2::new(100.0, 100.0), 20.0)).is_empty());
        assert_eq!(ids_of(&tree.query_in_range(Vec2::new(900.0, 900.0), 20.0)), vec!["mover"]);
    }

    #[test]
    fn insert_beyond_capacity_triggers_subdivide() {
        let mut tree: QuadTree<String, ()> = QuadTree::new(6, 2);
        tree.initialize(world_bounds(), vec![]);

        assert_eq!(tree.count_nodes(), 1);

        for (i, (x, y)) in [(50.0, 50.0), (950.0, 50.0), (50.0, 950.0), (950.0, 950.0), (500.0, 500.0)]
            .iter().enumerate()
        {
            tree.insert(pt(&format!("o{}", i), *x, *y, 5.0));
        }
        assert!(tree.count_nodes() > 1);

        let all = tree.query_in_range(Vec2::new(500.0, 500.0), 800.0);
        assert_eq!(all.len(), 5);
    }

    #[test]
    fn remove_dedupes_across_overlapping_leaves() {
        let mut tree: QuadTree<String, ()> = QuadTree::new(4, 1);
        tree.initialize(world_bounds(), vec![
            pt("spanner", 500.0, 500.0, 200.0),
            pt("filler1", 100.0, 100.0, 5.0),
            pt("filler2", 900.0, 900.0, 5.0),
        ]);

        let q = tree.query_in_range(Vec2::new(500.0, 500.0), 50.0);
        assert_eq!(ids_of(&q), vec!["spanner"]);

        assert!(tree.remove(&"spanner".to_string()));
        let q2 = tree.query_in_range(Vec2::new(500.0, 500.0), 50.0);
        assert!(ids_of(&q2).is_empty());

        let q3 = tree.query_in_range(Vec2::new(100.0, 100.0), 20.0);
        assert_eq!(ids_of(&q3), vec!["filler1"]);
    }
}
