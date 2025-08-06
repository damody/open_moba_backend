/// 360度圓形視野系統
/// 
/// 實現真實的圓形視野和精確的陰影投射
use specs::prelude::*;
use specs_derive::Component;
use vek::Vec2;
use std::collections::BTreeMap;
use serde::{Deserialize, Serialize};

/// 圓形視野組件
#[derive(Component, Debug, Clone)]
#[storage(VecStorage)]
pub struct CircularVision {
    /// 視野半徑
    pub range: f32,
    /// 觀察者高度
    pub height: f32,
    /// 視野精度（射線數量，影響性能）
    pub precision: u32,
    /// 是否為真實視野（無視隱身）
    pub true_sight: bool,
    /// 最後計算的視野結果
    pub vision_result: Option<VisionResult>,
}

/// 視野計算結果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisionResult {
    /// 觀察者位置
    pub observer_pos: Vec2<f32>,
    /// 視野半徑
    pub range: f32,
    /// 可見區域（多邊形頂點）
    pub visible_area: Vec<Vec2<f32>>,
    /// 陰影區域列表
    pub shadows: Vec<ShadowArea>,
    /// 計算時間戳
    pub timestamp: f64,
}

/// 陰影區域
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowArea {
    /// 陰影類型
    pub shadow_type: ShadowType,
    /// 造成陰影的遮擋物ID
    pub blocker_id: Option<String>,
    /// 陰影幾何形狀
    pub geometry: ShadowGeometry,
    /// 陰影深度（從遮擋物到視野邊緣的距離）
    pub depth: f32,
}

/// 陰影類型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ShadowType {
    /// 扇形陰影（樹木、柱子等圓形遮擋物）
    Sector,
    /// 梯形陰影（牆壁、建築等矩形遮擋物）
    Trapezoid,
    /// 地形陰影（高低起伏造成的複雜陰影）
    Terrain,
}

/// 陰影幾何形狀
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ShadowGeometry {
    /// 扇形：中心點、起始角度、結束角度、半徑
    Sector {
        center: Vec2<f32>,
        start_angle: f32,
        end_angle: f32,
        radius: f32,
    },
    /// 梯形：四個頂點
    Trapezoid {
        vertices: [Vec2<f32>; 4],
    },
    /// 多邊形：任意頂點數
    Polygon {
        vertices: Vec<Vec2<f32>>,
    },
}

/// 遮擋物信息
#[derive(Debug, Clone)]
pub struct ObstacleInfo {
    /// 位置
    pub position: Vec2<f32>,
    /// 遮擋物類型
    pub obstacle_type: ObstacleType,
    /// 高度
    pub height: f32,
    /// 其他屬性
    pub properties: ObstacleProperties,
}

/// 遮擋物類型
#[derive(Debug, Clone, PartialEq)]
pub enum ObstacleType {
    /// 圓形遮擋物（樹木、柱子）
    Circular { radius: f32 },
    /// 矩形遮擋物（建築、牆壁）
    Rectangle { width: f32, height: f32, rotation: f32 },
    /// 地形高度
    Terrain { elevation: f32 },
}

/// 遮擋物屬性
#[derive(Debug, Clone)]
pub struct ObstacleProperties {
    /// 是否完全遮擋
    pub blocks_completely: bool,
    /// 遮擋程度 (0.0-1.0)
    pub opacity: f32,
    /// 投射陰影的距離倍數
    pub shadow_multiplier: f32,
}

impl CircularVision {
    /// 創建新的圓形視野
    pub fn new(range: f32, height: f32) -> Self {
        Self {
            range,
            height,
            precision: 360, // 每度一條射線
            true_sight: false,
            vision_result: None,
        }
    }
    
    /// 設置視野精度
    pub fn with_precision(mut self, precision: u32) -> Self {
        self.precision = precision;
        self
    }
    
    /// 設置真實視野
    pub fn with_true_sight(mut self) -> Self {
        self.true_sight = true;
        self
    }
}

/// 陰影投射計算器
pub struct ShadowCaster {
    /// 射線精度
    ray_precision: u32,
    /// 最小陰影角度（度）
    min_shadow_angle: f32,
}

impl ShadowCaster {
    /// 創建陰影投射器
    pub fn new() -> Self {
        Self {
            ray_precision: 360,
            min_shadow_angle: 0.5, // 最小0.5度的陰影才會被計算
        }
    }
    
    /// 計算360度圓形視野
    pub fn calculate_circular_vision(
        &self,
        observer_pos: Vec2<f32>,
        observer_height: f32,
        vision_range: f32,
        obstacles: &[ObstacleInfo],
    ) -> VisionResult {
        let mut shadows = Vec::new();
        let angle_step = 360.0 / self.ray_precision as f32;
        
        // 為每個遮擋物計算陰影
        for obstacle in obstacles {
            if let Some(shadow) = self.calculate_obstacle_shadow(
                observer_pos,
                observer_height,
                vision_range,
                obstacle,
            ) {
                shadows.push(shadow);
            }
        }
        
        // 合併重疊的陰影
        shadows = self.merge_overlapping_shadows(shadows);
        
        // 計算可見區域（去除陰影後的區域）
        let visible_area = self.calculate_visible_area(
            observer_pos,
            vision_range,
            &shadows,
        );
        
        VisionResult {
            observer_pos,
            range: vision_range,
            visible_area,
            shadows,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64(),
        }
    }
    
    /// 計算單個遮擋物的陰影
    fn calculate_obstacle_shadow(
        &self,
        observer_pos: Vec2<f32>,
        observer_height: f32,
        vision_range: f32,
        obstacle: &ObstacleInfo,
    ) -> Option<ShadowArea> {
        let distance = observer_pos.distance(obstacle.position);
        
        // 如果遮擋物在視野範圍外，不產生陰影
        if distance > vision_range {
            return None;
        }
        
        match obstacle.obstacle_type {
            ObstacleType::Circular { radius } => {
                self.calculate_circular_shadow(
                    observer_pos,
                    vision_range,
                    obstacle.position,
                    radius,
                    obstacle.height,
                    observer_height,
                )
            },
            ObstacleType::Rectangle { width, height, rotation } => {
                self.calculate_rectangular_shadow(
                    observer_pos,
                    vision_range,
                    obstacle.position,
                    width,
                    height,
                    rotation,
                    obstacle.height,
                    observer_height,
                )
            },
            ObstacleType::Terrain { elevation } => {
                self.calculate_terrain_shadow(
                    observer_pos,
                    observer_height,
                    vision_range,
                    obstacle.position,
                    elevation,
                )
            },
        }
    }
    
    /// 計算圓形遮擋物的扇形陰影
    fn calculate_circular_shadow(
        &self,
        observer_pos: Vec2<f32>,
        vision_range: f32,
        obstacle_pos: Vec2<f32>,
        obstacle_radius: f32,
        obstacle_height: f32,
        observer_height: f32,
    ) -> Option<ShadowArea> {
        let to_obstacle = obstacle_pos - observer_pos;
        let distance = to_obstacle.magnitude();
        
        // 檢查高度是否會遮擋
        if !self.blocks_line_of_sight(distance, obstacle_height, observer_height) {
            return None;
        }
        
        // 計算切線角度
        let center_angle = to_obstacle.y.atan2(to_obstacle.x);
        let angle_offset = (obstacle_radius / distance).asin();
        
        // 檢查角度是否足夠大
        if angle_offset.to_degrees() < self.min_shadow_angle {
            return None;
        }
        
        let start_angle = center_angle - angle_offset;
        let end_angle = center_angle + angle_offset;
        
        // 計算陰影深度
        let shadow_depth = vision_range - distance;
        
        Some(ShadowArea {
            shadow_type: ShadowType::Sector,
            blocker_id: None,
            geometry: ShadowGeometry::Sector {
                center: observer_pos,
                start_angle,
                end_angle,
                radius: vision_range,
            },
            depth: shadow_depth,
        })
    }
    
    /// 計算矩形遮擋物的梯形陰影
    fn calculate_rectangular_shadow(
        &self,
        observer_pos: Vec2<f32>,
        vision_range: f32,
        obstacle_pos: Vec2<f32>,
        width: f32,
        height: f32,
        rotation: f32,
        obstacle_height: f32,
        observer_height: f32,
    ) -> Option<ShadowArea> {
        // 計算旋轉後的矩形四個頂點
        let cos_r = rotation.cos();
        let sin_r = rotation.sin();
        let half_w = width * 0.5;
        let half_h = height * 0.5;
        
        let corners = [
            Vec2::new(-half_w, -half_h),
            Vec2::new(half_w, -half_h),
            Vec2::new(half_w, half_h),
            Vec2::new(-half_w, half_h),
        ];
        
        let rotated_corners: Vec<Vec2<f32>> = corners
            .iter()
            .map(|&corner| {
                let x = corner.x * cos_r - corner.y * sin_r;
                let y = corner.x * sin_r + corner.y * cos_r;
                obstacle_pos + Vec2::new(x, y)
            })
            .collect();
        
        // 找到從觀察者看來的遮擋邊緣
        let visible_edges = self.find_visible_edges(observer_pos, &rotated_corners);
        
        if visible_edges.is_empty() {
            return None;
        }
        
        // 投射邊緣到視野圓周
        let projected_vertices = self.project_edges_to_circle(
            observer_pos,
            vision_range,
            &visible_edges,
        );
        
        Some(ShadowArea {
            shadow_type: ShadowType::Trapezoid,
            blocker_id: None,
            geometry: ShadowGeometry::Polygon {
                vertices: projected_vertices,
            },
            depth: vision_range - observer_pos.distance(obstacle_pos),
        })
    }
    
    /// 計算地形陰影
    fn calculate_terrain_shadow(
        &self,
        observer_pos: Vec2<f32>,
        observer_height: f32,
        vision_range: f32,
        terrain_pos: Vec2<f32>,
        terrain_elevation: f32,
    ) -> Option<ShadowArea> {
        // 地形陰影計算（簡化版本）
        if terrain_elevation <= observer_height {
            return None;
        }
        
        let distance = observer_pos.distance(terrain_pos);
        let height_diff = terrain_elevation - observer_height;
        
        // 計算陰影投射距離
        let shadow_distance = (height_diff * distance) / observer_height.max(1.0);
        let shadow_end_distance = (distance + shadow_distance).min(vision_range);
        
        if shadow_end_distance <= distance {
            return None;
        }
        
        // 簡化：創建一個小的扇形陰影
        let to_terrain = terrain_pos - observer_pos;
        let center_angle = to_terrain.y.atan2(to_terrain.x);
        let angle_width = 0.1; // 10度寬的陰影
        
        Some(ShadowArea {
            shadow_type: ShadowType::Terrain,
            blocker_id: None,
            geometry: ShadowGeometry::Sector {
                center: observer_pos,
                start_angle: center_angle - angle_width,
                end_angle: center_angle + angle_width,
                radius: shadow_end_distance,
            },
            depth: shadow_end_distance - distance,
        })
    }
    
    /// 檢查高度是否遮擋視線
    fn blocks_line_of_sight(
        &self,
        distance: f32,
        obstacle_height: f32,
        observer_height: f32,
    ) -> bool {
        // 簡化的視線遮擋檢查
        // 實際應該考慮角度、距離等因素
        obstacle_height > observer_height * 0.5
    }
    
    /// 找到矩形的可見邊緣
    fn find_visible_edges(
        &self,
        observer_pos: Vec2<f32>,
        corners: &[Vec2<f32>],
    ) -> Vec<Vec2<f32>> {
        let mut visible_points = Vec::new();
        
        for &corner in corners {
            let to_corner = corner - observer_pos;
            // 簡化：假設所有角點都可見
            visible_points.push(corner);
        }
        
        // 按角度排序
        visible_points.sort_by(|a, b| {
            let angle_a = (a - observer_pos).y.atan2((a - observer_pos).x);
            let angle_b = (b - observer_pos).y.atan2((b - observer_pos).x);
            angle_a.partial_cmp(&angle_b).unwrap()
        });
        
        visible_points
    }
    
    /// 將邊緣投射到視野圓周
    fn project_edges_to_circle(
        &self,
        observer_pos: Vec2<f32>,
        radius: f32,
        edges: &[Vec2<f32>],
    ) -> Vec<Vec2<f32>> {
        let mut projected = Vec::new();
        
        for &edge in edges {
            let direction = (edge - observer_pos).normalized();
            let projected_point = observer_pos + direction * radius;
            projected.push(projected_point);
        }
        
        projected
    }
    
    /// 合併重疊的陰影區域
    fn merge_overlapping_shadows(&self, shadows: Vec<ShadowArea>) -> Vec<ShadowArea> {
        // 簡化實現：直接返回所有陰影
        // 實際應該合併重疊的扇形和多邊形
        shadows
    }
    
    /// 計算可見區域（360度圓減去陰影區域）
    fn calculate_visible_area(
        &self,
        observer_pos: Vec2<f32>,
        radius: f32,
        shadows: &[ShadowArea],
    ) -> Vec<Vec2<f32>> {
        let mut visible_area = Vec::new();
        let angle_step = 2.0 * std::f32::consts::PI / self.ray_precision as f32;
        
        for i in 0..self.ray_precision {
            let angle = i as f32 * angle_step;
            let direction = Vec2::new(angle.cos(), angle.sin());
            
            // 檢查這個方向是否被陰影遮擋
            let mut is_visible = true;
            let mut max_distance = radius;
            
            for shadow in shadows {
                if let Some(intersection_distance) = self.ray_intersects_shadow(
                    observer_pos,
                    direction,
                    shadow,
                ) {
                    if intersection_distance < max_distance {
                        max_distance = intersection_distance;
                        is_visible = false;
                    }
                }
            }
            
            if is_visible || max_distance > 0.0 {
                visible_area.push(observer_pos + direction * max_distance);
            }
        }
        
        visible_area
    }
    
    /// 檢查射線是否與陰影相交
    fn ray_intersects_shadow(
        &self,
        origin: Vec2<f32>,
        direction: Vec2<f32>,
        shadow: &ShadowArea,
    ) -> Option<f32> {
        match &shadow.geometry {
            ShadowGeometry::Sector { center, start_angle, end_angle, radius } => {
                let ray_angle = direction.y.atan2(direction.x);
                
                // 正規化角度到 [0, 2π]
                let normalize_angle = |angle: f32| {
                    let mut a = angle;
                    while a < 0.0 { a += 2.0 * std::f32::consts::PI; }
                    while a >= 2.0 * std::f32::consts::PI { a -= 2.0 * std::f32::consts::PI; }
                    a
                };
                
                let norm_ray = normalize_angle(ray_angle);
                let norm_start = normalize_angle(*start_angle);
                let norm_end = normalize_angle(*end_angle);
                
                // 檢查射線是否在扇形角度範圍內
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
            ShadowGeometry::Polygon { vertices } => {
                // 簡化的多邊形相交檢測
                // 實際應該使用更精確的射線-多邊形相交算法
                None
            },
            _ => None,
        }
    }
}

impl Default for ShadowCaster {
    fn default() -> Self {
        Self::new()
    }
}