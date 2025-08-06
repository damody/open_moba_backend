// 簡化的視野測試程序，避免其他模組的依賴問題
use vek::Vec2;
use std::f32::consts::PI;
use std::collections::{HashMap, BTreeMap};
use std::time::{SystemTime, UNIX_EPOCH};

// 簡化的組件和結構體定義
#[derive(Debug, Clone)]
pub struct CircularVision {
    pub range: f32,
    pub height: f32,
    pub precision: u32,
    pub true_sight: bool,
    pub vision_result: Option<VisionResult>,
}

#[derive(Debug, Clone)]
pub struct VisionResult {
    pub observer_pos: Vec2<f32>,
    pub range: f32,
    pub visible_area: Vec<Vec2<f32>>,
    pub shadows: Vec<ShadowArea>,
    pub timestamp: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ShadowType {
    Sector,
    Polygon,
    Terrain,
}

#[derive(Debug, Clone)]
pub enum ShadowGeometry {
    Sector {
        center: Vec2<f32>,
        start_angle: f32,
        end_angle: f32,
        radius: f32,
    },
    Polygon {
        vertices: Vec<Vec2<f32>>,
    },
    Trapezoid {
        vertices: [Vec2<f32>; 4],
    },
}

#[derive(Debug, Clone)]
pub struct ShadowArea {
    pub shadow_type: ShadowType,
    pub blocker_id: Option<String>,
    pub geometry: ShadowGeometry,
    pub depth: f32,
}

#[derive(Debug, Clone)]
pub struct ObstacleInfo {
    pub position: Vec2<f32>,
    pub obstacle_type: ObstacleType,
    pub height: f32,
    pub properties: ObstacleProperties,
}

#[derive(Debug, Clone)]
pub enum ObstacleType {
    Circular { radius: f32 },
    Rectangle { width: f32, height: f32, rotation: f32 },
    Terrain { elevation: f32 },
}

#[derive(Debug, Clone)]
pub struct ObstacleProperties {
    pub blocks_completely: bool,
    pub opacity: f32,
    pub shadow_multiplier: f32,
}

#[derive(Debug, Clone)]
pub struct Bounds {
    pub min: Vec2<f32>,
    pub max: Vec2<f32>,
}

impl Bounds {
    pub fn new(min: Vec2<f32>, max: Vec2<f32>) -> Self {
        Self { min, max }
    }
}

// 簡化的陰影計算器
pub struct ShadowCalculator {
    obstacles: Vec<ObstacleInfo>,
}

impl ShadowCalculator {
    pub fn new() -> Self {
        Self {
            obstacles: Vec::new(),
        }
    }

    pub fn initialize_quadtree(&mut self, _world_bounds: Bounds, obstacles: Vec<ObstacleInfo>) {
        self.obstacles = obstacles;
    }

    pub fn calculate_optimized_vision(
        &mut self,
        observer_pos: Vec2<f32>,
        observer_height: f32,
        vision_range: f32,
    ) -> VisionResult {
        let mut shadows = Vec::new();
        
        for obstacle in &self.obstacles {
            if let Some(shadow) = self.calculate_obstacle_shadow(
                observer_pos, 
                observer_height, 
                vision_range, 
                obstacle
            ) {
                shadows.push(shadow);
            }
        }

        let visible_area = self.calculate_visible_area(observer_pos, vision_range, &shadows);

        VisionResult {
            observer_pos,
            range: vision_range,
            visible_area,
            shadows,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64(),
        }
    }

    fn calculate_obstacle_shadow(
        &self,
        observer_pos: Vec2<f32>,
        observer_height: f32,
        vision_range: f32,
        obstacle: &ObstacleInfo,
    ) -> Option<ShadowArea> {
        let distance = observer_pos.distance(obstacle.position);
        
        if distance > vision_range {
            return None;
        }

        match &obstacle.obstacle_type {
            ObstacleType::Circular { radius } => {
                if obstacle.height <= observer_height * 0.5 {
                    return None;
                }
                
                let to_obstacle = obstacle.position - observer_pos;
                let center_angle = to_obstacle.y.atan2(to_obstacle.x);
                let angle_offset = (*radius / distance).asin();
                
                if angle_offset.to_degrees() < 0.5 {
                    return None;
                }
                
                Some(ShadowArea {
                    shadow_type: ShadowType::Sector,
                    blocker_id: None,
                    geometry: ShadowGeometry::Sector {
                        center: observer_pos,
                        start_angle: center_angle - angle_offset,
                        end_angle: center_angle + angle_offset,
                        radius: vision_range,
                    },
                    depth: vision_range - distance,
                })
            },
            _ => None, // 簡化：只處理圓形障礙物
        }
    }

    fn calculate_visible_area(
        &self,
        observer_pos: Vec2<f32>,
        radius: f32,
        shadows: &[ShadowArea],
    ) -> Vec<Vec2<f32>> {
        let mut visible_points = Vec::new();
        let precision = 360;
        let angle_step = 2.0 * PI / precision as f32;
        
        for i in 0..precision {
            let angle = i as f32 * angle_step;
            let direction = Vec2::new(angle.cos(), angle.sin());
            
            let mut max_distance = radius;
            for shadow in shadows {
                if let Some(intersection) = self.ray_shadow_intersection(
                    observer_pos, direction, shadow
                ) {
                    max_distance = max_distance.min(intersection);
                }
            }
            
            visible_points.push(observer_pos + direction * max_distance);
        }
        
        visible_points
    }

    fn ray_shadow_intersection(
        &self,
        _origin: Vec2<f32>,
        direction: Vec2<f32>,
        shadow: &ShadowArea,
    ) -> Option<f32> {
        match &shadow.geometry {
            ShadowGeometry::Sector { start_angle, end_angle, radius, .. } => {
                let ray_angle = direction.y.atan2(direction.x);
                
                let normalize = |mut angle: f32| {
                    while angle < 0.0 { angle += 2.0 * PI; }
                    while angle >= 2.0 * PI { angle -= 2.0 * PI; }
                    angle
                };
                
                let norm_ray = normalize(ray_angle);
                let norm_start = normalize(*start_angle);
                let norm_end = normalize(*end_angle);
                
                let in_sector = if norm_start <= norm_end {
                    norm_ray >= norm_start && norm_ray <= norm_end
                } else {
                    norm_ray >= norm_start || norm_ray <= norm_end
                };
                
                if in_sector {
                    Some(*radius)
                } else {
                    None
                }
            },
            _ => None,
        }
    }
}

// 測試函數
fn test_basic_vision_calculation() {
    println!("=== 測試基本視野計算 ===");
    
    let mut calculator = ShadowCalculator::new();
    
    // 創建一個簡單的圓形障礙物
    let obstacle = ObstacleInfo {
        position: Vec2::new(50.0, 0.0), // 在觀察者右側
        obstacle_type: ObstacleType::Circular { radius: 10.0 },
        height: 50.0,
        properties: ObstacleProperties {
            blocks_completely: true,
            opacity: 1.0,
            shadow_multiplier: 1.0,
        }
    };

    let obstacles = vec![obstacle];
    let world_bounds = Bounds::new(Vec2::new(-100.0, -100.0), Vec2::new(100.0, 100.0));
    calculator.initialize_quadtree(world_bounds, obstacles);

    let observer_pos = Vec2::new(0.0, 0.0);
    let result = calculator.calculate_optimized_vision(observer_pos, 20.0, 100.0);

    // 驗證結果的基本屬性
    assert_eq!(result.observer_pos, observer_pos);
    assert_eq!(result.range, 100.0);
    assert!(!result.visible_area.is_empty(), "應該有可見區域");
    
    println!("✅ 基本視野計算測試通過");
    println!("  觀察者位置: {:?}", result.observer_pos);
    println!("  視野範圍: {}", result.range);
    println!("  陰影數量: {}", result.shadows.len());
    println!("  可見區域點數: {}", result.visible_area.len());
}

fn test_unobstructed_vision() {
    println!("=== 測試無障礙物視野 ===");
    
    let mut calculator = ShadowCalculator::new();
    
    // 無障礙物
    let obstacles = vec![];
    let world_bounds = Bounds::new(Vec2::new(-50.0, -50.0), Vec2::new(50.0, 50.0));
    calculator.initialize_quadtree(world_bounds, obstacles);

    let result = calculator.calculate_optimized_vision(Vec2::new(0.0, 0.0), 10.0, 30.0);

    // 無障礙物時不應該有陰影
    assert_eq!(result.shadows.len(), 0, "無障礙物時不應該有陰影");
    assert!(!result.visible_area.is_empty(), "可見區域不應該為空");
    
    println!("✅ 無障礙物視野測試通過");
    println!("  陰影數量: {}", result.shadows.len());
    println!("  可見區域點數: {}", result.visible_area.len());
}

fn main() {
    println!("開始視野系統測試...");
    
    test_basic_vision_calculation();
    test_unobstructed_vision();
    
    println!("=== 所有測試完成 ===");
}