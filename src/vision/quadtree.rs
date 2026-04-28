use vek::Vec2;
use crate::comp::circular_vision::{ObstacleInfo, ObstacleType};

/// 一筆儲存於 QuadTree 內的障礙物連同它的字串 id。
/// id 由呼叫端決定，用於 `update_obstacle` / `remove_obstacle` 的增量定位
/// 以及 cache 失效追蹤。
#[derive(Debug, Clone)]
pub struct TreeEntry {
    pub id: String,
    pub obstacle: ObstacleInfo,
}

/// 四叉樹節點
#[derive(Debug, Clone)]
pub struct QuadTreeNode {
    /// 節點邊界
    pub bounds: Bounds,
    /// 子節點（NW, NE, SW, SE）
    pub children: Option<Box<[QuadTreeNode; 4]>>,
    /// 存儲的障礙物（含 id）；只有葉節點會有 entries，分裂後會 clear
    pub entries: Vec<TreeEntry>,
    /// 節點深度
    pub depth: usize,
}

/// 邊界矩形
#[derive(Debug, Clone)]
pub struct Bounds {
    pub min: Vec2<f32>,
    pub max: Vec2<f32>,
}

impl Bounds {
    pub fn new(min: Vec2<f32>, max: Vec2<f32>) -> Self {
        Self { min, max }
    }

    pub fn contains_point(&self, point: Vec2<f32>) -> bool {
        point.x >= self.min.x && point.x <= self.max.x &&
        point.y >= self.min.y && point.y <= self.max.y
    }

    pub fn width(&self) -> f32 {
        self.max.x - self.min.x
    }

    pub fn height(&self) -> f32 {
        self.max.y - self.min.y
    }
}

pub struct QuadTree {
    pub root: Option<QuadTreeNode>,
    pub max_tree_depth: usize,
    pub max_obstacles_per_node: usize,
}

impl QuadTree {
    pub fn new(max_tree_depth: usize, max_obstacles_per_node: usize) -> Self {
        Self {
            root: None,
            max_tree_depth,
            max_obstacles_per_node,
        }
    }

    /// 初始化四叉樹
    pub fn initialize(&mut self, world_bounds: Bounds, entries: Vec<TreeEntry>) {
        let mut root = QuadTreeNode {
            bounds: world_bounds,
            children: None,
            entries,
            depth: 0,
        };

        Self::subdivide_node(&mut root, self.max_tree_depth, self.max_obstacles_per_node);
        self.root = Some(root);
    }

    /// 增量插入：把單一 entry 加入既有樹。
    /// 若樹尚未 initialize（root 為 None）則 no-op，呼叫端有義務先 initialize。
    /// 一個 entry 若橫跨多個子節點 bounds 會被 clone 進每個相交子節點 — 與 initialize
    /// 後 subdivide 的分配方式一致，移除時依 id 去重清掉。
    pub fn insert_obstacle(&mut self, id: String, obstacle: ObstacleInfo) {
        let max_depth = self.max_tree_depth;
        let max_per_node = self.max_obstacles_per_node;
        if let Some(root) = self.root.as_mut() {
            Self::insert_into(root, TreeEntry { id, obstacle }, max_depth, max_per_node);
        }
    }

    /// 增量移除：把所有具備此 id 的 entry 從整棵樹刪除。
    /// 回傳是否真的刪到任何 entry（false = id 不存在）。
    /// 不會 collapse 子節點 — 即使整個分支變空也保留結構，避免 churn；
    /// 若需 compact 是另一個獨立優化。
    pub fn remove_obstacle(&mut self, id: &str) -> bool {
        if let Some(root) = self.root.as_mut() {
            Self::remove_from(root, id)
        } else {
            false
        }
    }

    /// 增量更新：等同 `remove_obstacle(id)` + `insert_obstacle(id, obstacle)`。
    /// 對「障礙物移動」或「屬性改變」的情境最直接。
    pub fn update_obstacle(&mut self, id: &str, obstacle: ObstacleInfo) {
        self.remove_obstacle(id);
        self.insert_obstacle(id.to_string(), obstacle);
    }

    /// 將 entry 插入指定節點。
    /// - 若 node 已分裂：對所有相交子節點遞迴插入（不在 internal node 留 entry）。
    /// - 若 node 是葉：push entry，超過上限則分裂；分裂會把當前 entries 全部下推。
    fn insert_into(
        node: &mut QuadTreeNode,
        entry: TreeEntry,
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

    /// 從 node 與其所有後代節點移除符合 id 的 entry。
    fn remove_from(node: &mut QuadTreeNode, id: &str) -> bool {
        let before = node.entries.len();
        node.entries.retain(|e| e.id != id);
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

    /// 遞歸細分節點
    fn subdivide_node(node: &mut QuadTreeNode, max_depth: usize, max_per_node: usize) {
        if node.entries.len() <= max_per_node || node.depth >= max_depth {
            return;
        }

        let bounds = &node.bounds;
        let mid_x = (bounds.min.x + bounds.max.x) * 0.5;
        let mid_y = (bounds.min.y + bounds.max.y) * 0.5;

        let mut children = Box::new([
            // 西北
            QuadTreeNode {
                bounds: Bounds {
                    min: Vec2::new(bounds.min.x, mid_y),
                    max: Vec2::new(mid_x, bounds.max.y),
                },
                children: None,
                entries: Vec::new(),
                depth: node.depth + 1,
            },
            // 東北
            QuadTreeNode {
                bounds: Bounds {
                    min: Vec2::new(mid_x, mid_y),
                    max: bounds.max.clone(),
                },
                children: None,
                entries: Vec::new(),
                depth: node.depth + 1,
            },
            // 西南
            QuadTreeNode {
                bounds: Bounds {
                    min: bounds.min.clone(),
                    max: Vec2::new(mid_x, mid_y),
                },
                children: None,
                entries: Vec::new(),
                depth: node.depth + 1,
            },
            // 東南
            QuadTreeNode {
                bounds: Bounds {
                    min: Vec2::new(mid_x, bounds.min.y),
                    max: Vec2::new(bounds.max.x, mid_y),
                },
                children: None,
                entries: Vec::new(),
                depth: node.depth + 1,
            },
        ]);

        // 將 entry 分配到子節點（橫跨者複製進每個相交子節點）
        for entry in &node.entries {
            for child in children.iter_mut() {
                if Self::entry_intersects_bounds(entry, &child.bounds) {
                    child.entries.push(entry.clone());
                }
            }
        }

        node.children = Some(children);
        node.entries.clear();

        // 遞歸細分子節點
        if let Some(ref mut children) = node.children {
            for child in children.iter_mut() {
                Self::subdivide_node(child, max_depth, max_per_node);
            }
        }
    }

    /// 檢查 entry 的障礙物是否與邊界相交
    fn entry_intersects_bounds(entry: &TreeEntry, bounds: &Bounds) -> bool {
        Self::obstacle_intersects_bounds(&entry.obstacle, bounds)
    }

    /// 檢查障礙物是否與邊界相交
    fn obstacle_intersects_bounds(obstacle: &ObstacleInfo, bounds: &Bounds) -> bool {
        let pos = obstacle.position;

        match &obstacle.obstacle_type {
            ObstacleType::Circular { radius } => {
                let closest_x = pos.x.max(bounds.min.x).min(bounds.max.x);
                let closest_y = pos.y.max(bounds.min.y).min(bounds.max.y);
                let distance = pos.distance(Vec2::new(closest_x, closest_y));
                distance <= *radius
            },
            ObstacleType::Rectangle { width, height, rotation: _ } => {
                let diagonal = (width * width + height * height).sqrt() * 0.5;
                let closest_x = pos.x.max(bounds.min.x).min(bounds.max.x);
                let closest_y = pos.y.max(bounds.min.y).min(bounds.max.y);
                let distance = pos.distance(Vec2::new(closest_x, closest_y));
                distance <= diagonal
            },
            ObstacleType::Terrain { .. } => {
                pos.x >= bounds.min.x && pos.x <= bounds.max.x &&
                pos.y >= bounds.min.y && pos.y <= bounds.max.y
            },
        }
    }

    /// 查詢範圍內的障礙物（不帶 id；shadow casting 用）
    pub fn query_obstacles_in_range(&self, center: Vec2<f32>, range: f32) -> Vec<ObstacleInfo> {
        self.query_entries_in_range(center, range)
            .into_iter()
            .map(|(_id, ob)| ob)
            .collect()
    }

    /// 查詢範圍內的障礙物（帶 id；用於 cache dependency 追蹤）
    /// 跨多個 leaf 的 entry 可能出現多次 — 用 BTreeSet 去重 by id 後回傳。
    pub fn query_entries_in_range(&self, center: Vec2<f32>, range: f32) -> Vec<(String, ObstacleInfo)> {
        let mut results: Vec<(String, ObstacleInfo)> = Vec::new();
        let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

        if let Some(ref tree) = self.root {
            let query_bounds = Bounds {
                min: center - Vec2::new(range, range),
                max: center + Vec2::new(range, range),
            };

            self.query_node_recursive(tree, &query_bounds, center, range, &mut results, &mut seen);
        }

        results
    }

    /// 遞歸查詢節點
    fn query_node_recursive(
        &self,
        node: &QuadTreeNode,
        query_bounds: &Bounds,
        center: Vec2<f32>,
        range: f32,
        results: &mut Vec<(String, ObstacleInfo)>,
        seen: &mut std::collections::BTreeSet<String>,
    ) {
        if !self.bounds_intersect(&node.bounds, query_bounds) {
            return;
        }

        for entry in &node.entries {
            if seen.contains(&entry.id) {
                continue;
            }
            let distance = entry.obstacle.position.distance(center);

            let extended_range = range + match &entry.obstacle.obstacle_type {
                ObstacleType::Circular { radius } => *radius,
                ObstacleType::Rectangle { width, height, .. } => {
                    (width * width + height * height).sqrt() * 0.5
                },
                ObstacleType::Terrain { .. } => 50.0,
            };

            if distance <= extended_range {
                seen.insert(entry.id.clone());
                results.push((entry.id.clone(), entry.obstacle.clone()));
            }
        }

        if let Some(ref children) = node.children {
            for child in children.iter() {
                self.query_node_recursive(child, query_bounds, center, range, results, seen);
            }
        }
    }

    /// 檢查兩個邊界是否相交
    fn bounds_intersect(&self, bounds1: &Bounds, bounds2: &Bounds) -> bool {
        bounds1.min.x <= bounds2.max.x && bounds1.max.x >= bounds2.min.x &&
        bounds1.min.y <= bounds2.max.y && bounds1.max.y >= bounds2.min.y
    }

    /// 計算四叉樹節點數量
    pub fn count_nodes(&self) -> usize {
        if let Some(ref root) = self.root {
            self.count_nodes_recursive(root)
        } else {
            0
        }
    }

    /// 遞歸計算節點數量
    fn count_nodes_recursive(&self, node: &QuadTreeNode) -> usize {
        let mut count = 1;
        if let Some(ref children) = node.children {
            for child in children.iter() {
                count += self.count_nodes_recursive(child);
            }
        }
        count
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
    fn insert_into_initialized_tree_then_query() {
        let mut tree = QuadTree::new(4, 4);
        tree.initialize(world_bounds(), vec![]);

        tree.insert_obstacle("a".to_string(), obs(100.0, 100.0, 10.0));
        tree.insert_obstacle("b".to_string(), obs(800.0, 800.0, 10.0));

        let near_a = tree.query_entries_in_range(Vec2::new(100.0, 100.0), 50.0);
        assert_eq!(ids_of(&near_a), vec!["a"]);

        let near_b = tree.query_entries_in_range(Vec2::new(800.0, 800.0), 50.0);
        assert_eq!(ids_of(&near_b), vec!["b"]);
    }

    #[test]
    fn remove_drops_entry_from_subsequent_queries() {
        let mut tree = QuadTree::new(4, 4);
        tree.initialize(world_bounds(), vec![
            TreeEntry { id: "a".into(), obstacle: obs(100.0, 100.0, 10.0) },
            TreeEntry { id: "b".into(), obstacle: obs(120.0, 110.0, 10.0) },
        ]);

        let before = tree.query_entries_in_range(Vec2::new(110.0, 105.0), 100.0);
        assert_eq!(ids_of(&before), vec!["a", "b"]);

        let removed = tree.remove_obstacle("a");
        assert!(removed, "remove_obstacle should report true when id existed");

        let after = tree.query_entries_in_range(Vec2::new(110.0, 105.0), 100.0);
        assert_eq!(ids_of(&after), vec!["b"]);
        assert!(!tree.remove_obstacle("a"), "second remove should report false");
    }

    #[test]
    fn update_moves_entry_in_query_results() {
        let mut tree = QuadTree::new(4, 4);
        tree.initialize(world_bounds(), vec![
            TreeEntry { id: "mover".into(), obstacle: obs(100.0, 100.0, 5.0) },
        ]);

        let near_old = tree.query_entries_in_range(Vec2::new(100.0, 100.0), 20.0);
        assert_eq!(ids_of(&near_old), vec!["mover"]);

        tree.update_obstacle("mover", obs(900.0, 900.0, 5.0));

        let near_old_after = tree.query_entries_in_range(Vec2::new(100.0, 100.0), 20.0);
        assert!(ids_of(&near_old_after).is_empty(), "old position should no longer match");

        let near_new = tree.query_entries_in_range(Vec2::new(900.0, 900.0), 20.0);
        assert_eq!(ids_of(&near_new), vec!["mover"]);
    }

    #[test]
    fn insert_beyond_capacity_triggers_subdivide() {
        let mut tree = QuadTree::new(6, 2);
        tree.initialize(world_bounds(), vec![]);

        let nodes_empty = tree.count_nodes();
        assert_eq!(nodes_empty, 1, "empty initialized tree should have only the root");

        // Insert 5 widely spread obstacles — > max_per_node=2 → subdivide
        for (i, (x, y)) in [(50.0, 50.0), (950.0, 50.0), (50.0, 950.0), (950.0, 950.0), (500.0, 500.0)]
            .iter().enumerate()
        {
            tree.insert_obstacle(format!("o{}", i), obs(*x, *y, 5.0));
        }

        let nodes_after = tree.count_nodes();
        assert!(nodes_after > 1, "tree should have subdivided after exceeding capacity (got {} node)", nodes_after);

        // All five should still be queryable
        let all = tree.query_entries_in_range(Vec2::new(500.0, 500.0), 800.0);
        assert_eq!(all.len(), 5, "all 5 obstacles should be findable, got {}", all.len());
    }

    #[test]
    fn remove_dedupes_across_overlapping_leaves() {
        // 障礙物半徑大到必然跨子節點時，會在多個 leaf 留 clone；
        // remove by id 應一次清乾淨。
        let mut tree = QuadTree::new(4, 1);
        tree.initialize(
            world_bounds(),
            vec![
                TreeEntry { id: "spanner".into(), obstacle: obs(500.0, 500.0, 200.0) },
                TreeEntry { id: "filler1".into(), obstacle: obs(100.0, 100.0, 5.0) },
                TreeEntry { id: "filler2".into(), obstacle: obs(900.0, 900.0, 5.0) },
            ],
        );

        // After subdivide, spanner should be in multiple leaves; query 應仍只回 1 個（去重）
        let q = tree.query_entries_in_range(Vec2::new(500.0, 500.0), 50.0);
        assert_eq!(ids_of(&q), vec!["spanner"]);

        assert!(tree.remove_obstacle("spanner"));
        let q2 = tree.query_entries_in_range(Vec2::new(500.0, 500.0), 50.0);
        assert!(ids_of(&q2).is_empty(), "spanner should be fully removed across all leaves");

        // 其他 id 不受影響
        let q3 = tree.query_entries_in_range(Vec2::new(100.0, 100.0), 20.0);
        assert_eq!(ids_of(&q3), vec!["filler1"]);
    }
}
