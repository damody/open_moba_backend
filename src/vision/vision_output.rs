/// 視野輸出系統
/// 
/// 提供點陣（Grid）和向量（Vector）兩種輸出格式
use vek::Vec2;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::comp::circular_vision::{VisionResult, ShadowArea, ShadowType, ShadowGeometry};

/// 視野輸出格式類型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum OutputFormat {
    /// 點陣格式（適合前端小地圖）
    Grid,
    /// 向量格式（適合SVG渲染）
    Vector,
    /// 混合格式（同時提供兩種）
    Mixed,
}

/// 點陣視野輸出
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GridVisionOutput {
    /// 輸出格式標識
    pub format_type: String,
    /// 網格大小（每格代表多少米）
    pub grid_size: f32,
    /// 觀察者位置
    pub observer_pos: Vec2<f32>,
    /// 視野半徑
    pub vision_range: f32,
    /// 網格寬度（格數）
    pub width: usize,
    /// 網格高度（格數）
    pub height: usize,
    /// 可見性網格
    pub visibility_grid: Vec<Vec<VisibilityLevel>>,
    /// 計算時間戳
    pub timestamp: f64,
}

/// 向量視野輸出
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorVisionOutput {
    /// 輸出格式標識
    pub format_type: String,
    /// 觀察者位置
    pub observer_pos: Vec2<f32>,
    /// 視野半徑
    pub vision_range: f32,
    /// 可見區域多邊形頂點
    pub visible_area: Vec<Vec2<f32>>,
    /// 陰影多邊形列表
    pub shadow_polygons: Vec<ShadowPolygon>,
    /// 計算時間戳
    pub timestamp: f64,
}

/// 可見性等級
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum VisibilityLevel {
    /// 完全不可見
    Invisible,
    /// 在陰影中（戰爭迷霧）
    Shadowed,
    /// 完全可見
    Visible,
    /// 部分可見（透明度0.0-1.0）
    Partial(f32),
}

/// 陰影多邊形（向量格式）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowPolygon {
    /// 陰影類型
    pub shadow_type: ShadowType,
    /// 多邊形頂點
    pub vertices: Vec<Vec2<f32>>,
    /// 透明度
    pub opacity: f32,
    /// 陰影中心（適用於扇形）
    pub center: Option<Vec2<f32>>,
    /// 扇形參數（適用於扇形陰影）
    pub sector_params: Option<SectorParams>,
}

/// 扇形參數
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectorParams {
    /// 起始角度（弧度）
    pub start_angle: f32,
    /// 結束角度（弧度）
    pub end_angle: f32,
    /// 半徑
    pub radius: f32,
}

/// 視野輸出生成器
pub struct VisionOutputGenerator {
    /// 默認網格大小
    default_grid_size: f32,
    /// 網格緩存
    grid_cache: HashMap<String, GridVisionOutput>,
    /// 向量緩存
    vector_cache: HashMap<String, VectorVisionOutput>,
}

impl VisionOutputGenerator {
    /// 創建新的輸出生成器
    pub fn new(default_grid_size: f32) -> Self {
        Self {
            default_grid_size,
            grid_cache: HashMap::new(),
            vector_cache: HashMap::new(),
        }
    }

    /// 生成點陣格式輸出
    pub fn generate_grid_output(
        &mut self,
        vision_result: &VisionResult,
        grid_size: Option<f32>,
    ) -> GridVisionOutput {
        let grid_size = grid_size.unwrap_or(self.default_grid_size);
        
        // 創建緩存鍵
        let cache_key = format!("{:.1}_{:.1}_{}", 
            vision_result.observer_pos.x, 
            vision_result.observer_pos.y, 
            vision_result.timestamp
        );
        
        // 檢查緩存
        if let Some(cached) = self.grid_cache.get(&cache_key) {
            return cached.clone();
        }

        // 計算網格範圍
        let half_range = vision_result.range;
        let grid_half_size = (half_range / grid_size).ceil() as usize;
        let width = grid_half_size * 2 + 1;
        let height = grid_half_size * 2 + 1;

        // 初始化可見性網格
        let mut visibility_grid = vec![vec![VisibilityLevel::Invisible; width]; height];

        // 計算每個網格點的可見性
        for y in 0..height {
            for x in 0..width {
                let grid_x = x as i32 - grid_half_size as i32;
                let grid_y = y as i32 - grid_half_size as i32;
                let world_pos = vision_result.observer_pos + Vec2::new(
                    grid_x as f32 * grid_size,
                    grid_y as f32 * grid_size
                );

                visibility_grid[y][x] = self.calculate_grid_visibility(
                    world_pos, 
                    vision_result
                );
            }
        }

        let output = GridVisionOutput {
            format_type: "grid".to_string(),
            grid_size,
            observer_pos: vision_result.observer_pos,
            vision_range: vision_result.range,
            width,
            height,
            visibility_grid,
            timestamp: vision_result.timestamp,
        };

        // 緩存結果
        self.grid_cache.insert(cache_key, output.clone());
        output
    }

    /// 生成向量格式輸出
    pub fn generate_vector_output(&mut self, vision_result: &VisionResult) -> VectorVisionOutput {
        // 創建緩存鍵
        let cache_key = format!("{:.1}_{:.1}_{}", 
            vision_result.observer_pos.x, 
            vision_result.observer_pos.y, 
            vision_result.timestamp
        );
        
        // 檢查緩存
        if let Some(cached) = self.vector_cache.get(&cache_key) {
            return cached.clone();
        }

        // 轉換陰影為多邊形
        let shadow_polygons = vision_result.shadows.iter()
            .map(|shadow| self.convert_shadow_to_polygon(shadow))
            .collect();

        let output = VectorVisionOutput {
            format_type: "vector".to_string(),
            observer_pos: vision_result.observer_pos,
            vision_range: vision_result.range,
            visible_area: vision_result.visible_area.clone(),
            shadow_polygons,
            timestamp: vision_result.timestamp,
        };

        // 緩存結果
        self.vector_cache.insert(cache_key, output.clone());
        output
    }

    /// 計算網格點的可見性
    fn calculate_grid_visibility(
        &self,
        point: Vec2<f32>,
        vision_result: &VisionResult,
    ) -> VisibilityLevel {
        let distance = vision_result.observer_pos.distance(point);
        
        // 超出視野範圍
        if distance > vision_result.range {
            return VisibilityLevel::Invisible;
        }

        // 檢查是否被陰影遮擋
        let mut visibility = 1.0f32;
        let direction = (point - vision_result.observer_pos).normalized();

        for shadow in &vision_result.shadows {
            if let Some(shadow_factor) = self.calculate_shadow_factor(
                point, 
                direction, 
                distance, 
                shadow
            ) {
                visibility *= (1.0 - shadow_factor).max(0.0);
            }
        }

        // 根據可見性等級返回結果
        match visibility {
            v if v >= 0.9 => VisibilityLevel::Visible,
            v if v <= 0.1 => VisibilityLevel::Invisible,
            v => VisibilityLevel::Partial(v),
        }
    }

    /// 計算點在陰影中的影響程度
    fn calculate_shadow_factor(
        &self,
        point: Vec2<f32>,
        direction: Vec2<f32>,
        distance: f32,
        shadow: &ShadowArea,
    ) -> Option<f32> {
        match &shadow.geometry {
            ShadowGeometry::Sector { center, start_angle, end_angle, radius } => {
                let point_angle = (point - center).y.atan2((point - center).x);
                let point_distance = center.distance(point);

                // 正規化角度
                let normalize = |mut angle: f32| {
                    while angle < 0.0 { angle += 2.0 * std::f32::consts::PI; }
                    while angle >= 2.0 * std::f32::consts::PI { angle -= 2.0 * std::f32::consts::PI; }
                    angle
                };

                let norm_point = normalize(point_angle);
                let norm_start = normalize(*start_angle);
                let norm_end = normalize(*end_angle);

                // 檢查是否在扇形內
                let in_sector = if norm_start <= norm_end {
                    norm_point >= norm_start && norm_point <= norm_end
                } else {
                    norm_point >= norm_start || norm_point <= norm_end
                };

                if in_sector && point_distance <= *radius {
                    Some(0.8) // 扇形陰影影響程度
                } else {
                    None
                }
            },
            ShadowGeometry::Polygon { vertices } => {
                // 使用射線投射法檢查點是否在多邊形內
                if self.point_in_polygon(point, vertices) {
                    Some(1.0) // 完全遮擋
                } else {
                    None
                }
            },
            _ => None,
        }
    }

    /// 檢查點是否在多邊形內（射線投射法）
    fn point_in_polygon(&self, point: Vec2<f32>, vertices: &[Vec2<f32>]) -> bool {
        let mut inside = false;
        let mut j = vertices.len() - 1;

        for i in 0..vertices.len() {
            if ((vertices[i].y > point.y) != (vertices[j].y > point.y)) &&
               (point.x < (vertices[j].x - vertices[i].x) * (point.y - vertices[i].y) / 
                         (vertices[j].y - vertices[i].y) + vertices[i].x) {
                inside = !inside;
            }
            j = i;
        }

        inside
    }

    /// 將陰影轉換為多邊形
    fn convert_shadow_to_polygon(&self, shadow: &ShadowArea) -> ShadowPolygon {
        match &shadow.geometry {
            ShadowGeometry::Sector { center, start_angle, end_angle, radius } => {
                // 將扇形轉換為多邊形頂點
                let mut vertices = vec![*center];
                let steps = 16; // 扇形細分數
                let angle_step = (end_angle - start_angle) / steps as f32;

                for i in 0..=steps {
                    let angle = start_angle + i as f32 * angle_step;
                    let point = center + Vec2::new(angle.cos(), angle.sin()) * (*radius);
                    vertices.push(point);
                }

                ShadowPolygon {
                    shadow_type: shadow.shadow_type.clone(),
                    vertices,
                    opacity: 0.8,
                    center: Some(*center),
                    sector_params: Some(SectorParams {
                        start_angle: *start_angle,
                        end_angle: *end_angle,
                        radius: *radius,
                    }),
                }
            },
            ShadowGeometry::Polygon { vertices } => {
                ShadowPolygon {
                    shadow_type: shadow.shadow_type.clone(),
                    vertices: vertices.clone(),
                    opacity: 1.0,
                    center: None,
                    sector_params: None,
                }
            },
            _ => {
                // 默認空多邊形
                ShadowPolygon {
                    shadow_type: shadow.shadow_type.clone(),
                    vertices: Vec::new(),
                    opacity: 1.0,
                    center: None,
                    sector_params: None,
                }
            }
        }
    }

    /// 清理緩存
    pub fn clear_cache(&mut self) {
        self.grid_cache.clear();
        self.vector_cache.clear();
    }

    /// 設置緩存大小限制
    pub fn limit_cache_size(&mut self, max_entries: usize) {
        while self.grid_cache.len() > max_entries {
            if let Some(oldest_key) = self.grid_cache.keys().next().cloned() {
                self.grid_cache.remove(&oldest_key);
            } else {
                break;
            }
        }

        while self.vector_cache.len() > max_entries {
            if let Some(oldest_key) = self.vector_cache.keys().next().cloned() {
                self.vector_cache.remove(&oldest_key);
            } else {
                break;
            }
        }
    }
}

impl Default for VisionOutputGenerator {
    fn default() -> Self {
        Self::new(25.0) // 默認25米網格
    }
}

// 為VisibilityLevel實現數值轉換
impl From<VisibilityLevel> for f32 {
    fn from(level: VisibilityLevel) -> f32 {
        match level {
            VisibilityLevel::Invisible => 0.0,
            VisibilityLevel::Shadowed => 0.3,
            VisibilityLevel::Visible => 1.0,
            VisibilityLevel::Partial(value) => value,
        }
    }
}

impl From<f32> for VisibilityLevel {
    fn from(value: f32) -> Self {
        match value {
            v if v <= 0.1 => VisibilityLevel::Invisible,
            v if v <= 0.4 => VisibilityLevel::Shadowed,
            v if v >= 0.9 => VisibilityLevel::Visible,
            v => VisibilityLevel::Partial(v.clamp(0.0, 1.0)),
        }
    }
}