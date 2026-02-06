/// 視野系統組件定義

use specs::prelude::*;
use specs::Component;
use vek::Vec2;
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

impl CircularVision {
    /// 創建新的圓形視野
    pub fn new(range: f32, height: f32) -> Self {
        Self {
            range,
            height,
            precision: 360, // 默認精度
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
    pub fn with_true_sight(mut self, true_sight: bool) -> Self {
        self.true_sight = true_sight;
        self
    }

    /// 檢查是否需要重新計算視野
    pub fn needs_recalculation(&self, current_time: f64) -> bool {
        if let Some(ref result) = self.vision_result {
            current_time - result.timestamp > 0.1 // 100ms更新一次
        } else {
            true
        }
    }
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

impl VisionResult {
    /// 檢查點是否在可見區域內
    pub fn is_point_visible(&self, point: Vec2<f32>) -> bool {
        // 首先檢查是否在視野範圍內
        if (point - self.observer_pos).magnitude() > self.range {
            return false;
        }

        // 檢查是否被陰影遮擋
        for shadow in &self.shadows {
            if shadow.contains_point(point) {
                return false;
            }
        }

        true
    }

    /// 獲取可見區域面積
    pub fn get_visible_area(&self) -> f32 {
        if self.visible_area.len() < 3 {
            return 0.0;
        }

        let mut area = 0.0;
        let n = self.visible_area.len();
        
        for i in 0..n {
            let j = (i + 1) % n;
            area += self.visible_area[i].x * self.visible_area[j].y;
            area -= self.visible_area[j].x * self.visible_area[i].y;
        }
        
        area.abs() / 2.0
    }
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

impl ShadowArea {
    /// 檢查點是否在陰影內
    pub fn contains_point(&self, point: Vec2<f32>) -> bool {
        match &self.geometry {
            ShadowGeometry::Sector { center, start_angle, end_angle, radius } => {
                let distance = (*center - point).magnitude();
                if distance > *radius {
                    return false;
                }
                
                let angle = (point - *center).y.atan2((point - *center).x);
                self.angle_in_range(angle, *start_angle, *end_angle)
            }
            ShadowGeometry::Polygon { vertices } => {
                self.point_in_polygon(point, vertices)
            }
            ShadowGeometry::Trapezoid { vertices } => {
                self.point_in_polygon(point, &vertices.to_vec())
            }
        }
    }

    fn angle_in_range(&self, angle: f32, start: f32, end: f32) -> bool {
        let normalize = |mut a: f32| {
            while a < 0.0 { a += 2.0 * std::f32::consts::PI; }
            while a >= 2.0 * std::f32::consts::PI { a -= 2.0 * std::f32::consts::PI; }
            a
        };

        let norm_angle = normalize(angle);
        let norm_start = normalize(start);
        let norm_end = normalize(end);

        if norm_start <= norm_end {
            norm_angle >= norm_start && norm_angle <= norm_end
        } else {
            norm_angle >= norm_start || norm_angle <= norm_end
        }
    }

    fn point_in_polygon(&self, point: Vec2<f32>, vertices: &[Vec2<f32>]) -> bool {
        if vertices.len() < 3 {
            return false;
        }

        let mut inside = false;
        let n = vertices.len();
        
        for i in 0..n {
            let j = (i + 1) % n;
            
            if ((vertices[i].y > point.y) != (vertices[j].y > point.y)) &&
               (point.x < (vertices[j].x - vertices[i].x) * (point.y - vertices[i].y) / (vertices[j].y - vertices[i].y) + vertices[i].x) {
                inside = !inside;
            }
        }
        
        inside
    }
}

/// 陰影類型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ShadowType {
    /// 物體陰影
    Object,
    /// 建築陰影
    Building,
    /// 地形陰影
    Terrain,
    /// 扇形陰影
    Sector,
    /// 臨時陰影
    Temporary,
}

/// 陰影幾何形狀
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ShadowGeometry {
    /// 扇形陰影
    Sector {
        center: Vec2<f32>,
        start_angle: f32,
        end_angle: f32,
        radius: f32,
    },
    /// 多邊形陰影
    Polygon {
        vertices: Vec<Vec2<f32>>,
    },
    /// 梯形陰影
    Trapezoid {
        vertices: [Vec2<f32>; 4],
    },
}

/// 障礙物信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObstacleInfo {
    /// 障礙物位置
    pub position: Vec2<f32>,
    /// 障礙物類型
    pub obstacle_type: ObstacleType,
    /// 障礙物高度
    pub height: f32,
    /// 障礙物屬性
    pub properties: ObstacleProperties,
}

/// 障礙物類型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ObstacleType {
    /// 圓形障礙物
    Circular { radius: f32 },
    /// 矩形障礙物
    Rectangle { width: f32, height: f32, rotation: f32 },
    /// 地形障礙物
    Terrain { elevation: f32 },
}

/// 障礙物屬性
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObstacleProperties {
    /// 是否完全遮擋
    pub blocks_completely: bool,
    /// 透明度
    pub opacity: f32,
    /// 陰影倍數
    pub shadow_multiplier: f32,
}