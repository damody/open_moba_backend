use vek::Vec2;
use crate::comp::circular_vision::{ObstacleInfo, ObstacleType};

/// 四叉樹節點
#[derive(Debug, Clone)]
pub struct QuadTreeNode {
    /// 節點邊界
    pub bounds: Bounds,
    /// 子節點（NW, NE, SW, SE）
    pub children: Option<Box<[QuadTreeNode; 4]>>,
    /// 存儲的障礙物
    pub obstacles: Vec<ObstacleInfo>,
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
    pub fn initialize(&mut self, world_bounds: Bounds, obstacles: Vec<ObstacleInfo>) {
        let mut root = QuadTreeNode {
            bounds: world_bounds,
            children: None,
            obstacles: obstacles,
            depth: 0,
        };

        self.subdivide_node(&mut root);
        self.root = Some(root);
    }

    /// 遞歸細分節點
    fn subdivide_node(&self, node: &mut QuadTreeNode) {
        if node.obstacles.len() <= self.max_obstacles_per_node || 
           node.depth >= self.max_tree_depth {
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
                obstacles: Vec::new(),
                depth: node.depth + 1,
            },
            // 東北
            QuadTreeNode {
                bounds: Bounds {
                    min: Vec2::new(mid_x, mid_y),
                    max: bounds.max,
                },
                children: None,
                obstacles: Vec::new(),
                depth: node.depth + 1,
            },
            // 西南
            QuadTreeNode {
                bounds: Bounds {
                    min: bounds.min,
                    max: Vec2::new(mid_x, mid_y),
                },
                children: None,
                obstacles: Vec::new(),
                depth: node.depth + 1,
            },
            // 東南
            QuadTreeNode {
                bounds: Bounds {
                    min: Vec2::new(mid_x, bounds.min.y),
                    max: Vec2::new(bounds.max.x, mid_y),
                },
                children: None,
                obstacles: Vec::new(),
                depth: node.depth + 1,
            },
        ]);

        // 將障礙物分配到子節點
        for obstacle in &node.obstacles {
            for child in children.iter_mut() {
                if self.obstacle_intersects_bounds(obstacle, &child.bounds) {
                    child.obstacles.push(obstacle.clone());
                }
            }
        }

        node.children = Some(children);
        node.obstacles.clear();

        // 遞歸細分子節點
        if let Some(ref mut children) = node.children {
            for child in children.iter_mut() {
                self.subdivide_node(child);
            }
        }
    }

    /// 檢查障礙物是否與邊界相交
    fn obstacle_intersects_bounds(&self, obstacle: &ObstacleInfo, bounds: &Bounds) -> bool {
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

    /// 查詢範圍內的障礙物
    pub fn query_obstacles_in_range(&self, center: Vec2<f32>, range: f32) -> Vec<ObstacleInfo> {
        let mut obstacles = Vec::new();
        
        if let Some(ref tree) = self.root {
            let query_bounds = Bounds {
                min: center - Vec2::new(range, range),
                max: center + Vec2::new(range, range),
            };
            
            self.query_node_recursive(tree, &query_bounds, center, range, &mut obstacles);
        }

        obstacles
    }

    /// 遞歸查詢節點
    fn query_node_recursive(
        &self,
        node: &QuadTreeNode,
        query_bounds: &Bounds,
        center: Vec2<f32>,
        range: f32,
        results: &mut Vec<ObstacleInfo>,
    ) {
        if !self.bounds_intersect(&node.bounds, query_bounds) {
            return;
        }

        for obstacle in &node.obstacles {
            let distance = obstacle.position.distance(center);
            
            let extended_range = range + match &obstacle.obstacle_type {
                ObstacleType::Circular { radius } => *radius,
                ObstacleType::Rectangle { width, height, .. } => {
                    (width * width + height * height).sqrt() * 0.5
                },
                ObstacleType::Terrain { .. } => 50.0,
            };

            if distance <= extended_range {
                results.push(obstacle.clone());
            }
        }

        if let Some(ref children) = node.children {
            for child in children.iter() {
                self.query_node_recursive(child, query_bounds, center, range, results);
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