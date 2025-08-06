/// 陰影系統

use vek::Vec2;
use super::components::*;

/// 陰影系統管理器
pub struct ShadowSystem;

impl ShadowSystem {
    /// 計算物體產生的陰影
    pub fn calculate_object_shadow(
        observer_pos: Vec2<f32>,
        observer_height: f32,
        obstacle: &ObstacleInfo,
        vision_range: f32,
    ) -> Option<ShadowArea> {
        let distance = (obstacle.position - observer_pos).magnitude();
        
        // 超出視野範圍
        if distance > vision_range {
            return None;
        }

        // 障礙物太低，不產生陰影
        if obstacle.height <= observer_height * 0.5 {
            return None;
        }

        match &obstacle.obstacle_type {
            ObstacleType::Circular { radius } => {
                Self::calculate_circular_shadow(
                    observer_pos, obstacle.position, *radius, 
                    obstacle.height, observer_height, vision_range
                )
            }
            ObstacleType::Rectangle { width, height, rotation } => {
                Self::calculate_rectangular_shadow(
                    observer_pos, obstacle.position, *width, *height, *rotation,
                    obstacle.height, observer_height, vision_range
                )
            }
            ObstacleType::Terrain { elevation } => {
                Self::calculate_terrain_shadow(
                    observer_pos, obstacle.position, *elevation,
                    observer_height, vision_range
                )
            }
        }
    }

    /// 計算圓形物體的陰影
    fn calculate_circular_shadow(
        observer_pos: Vec2<f32>,
        obstacle_pos: Vec2<f32>,
        radius: f32,
        obstacle_height: f32,
        observer_height: f32,
        vision_range: f32,
    ) -> Option<ShadowArea> {
        let to_obstacle = obstacle_pos - observer_pos;
        let distance = to_obstacle.magnitude();
        
        if distance <= radius {
            // 觀察者在物體內部，無陰影
            return None;
        }

        // 計算切線角度
        let center_angle = to_obstacle.y.atan2(to_obstacle.x);
        let angle_offset = (radius / distance).asin();
        
        // 陰影太小，忽略
        if angle_offset.to_degrees() < 0.5 {
            return None;
        }

        // 計算陰影長度（考慮高度差）
        let height_ratio = obstacle_height / observer_height;
        let shadow_length = (vision_range - distance) * height_ratio.min(2.0);
        
        Some(ShadowArea {
            shadow_type: ShadowType::Object,
            blocker_id: None,
            geometry: ShadowGeometry::Sector {
                center: observer_pos,
                start_angle: center_angle - angle_offset,
                end_angle: center_angle + angle_offset,
                radius: distance + shadow_length,
            },
            depth: shadow_length,
        })
    }

    /// 計算矩形物體的陰影
    fn calculate_rectangular_shadow(
        observer_pos: Vec2<f32>,
        obstacle_pos: Vec2<f32>,
        width: f32,
        height: f32,
        rotation: f32,
        obstacle_height: f32,
        observer_height: f32,
        vision_range: f32,
    ) -> Option<ShadowArea> {
        // 計算矩形的四個角點
        let half_width = width * 0.5;
        let half_height = height * 0.5;
        
        let cos_rot = rotation.cos();
        let sin_rot = rotation.sin();
        
        let corners = [
            Vec2::new(-half_width, -half_height),
            Vec2::new(half_width, -half_height),
            Vec2::new(half_width, half_height),
            Vec2::new(-half_width, half_height),
        ];
        
        let mut transformed_corners = Vec::new();
        for corner in &corners {
            let rotated = Vec2::new(
                corner.x * cos_rot - corner.y * sin_rot,
                corner.x * sin_rot + corner.y * cos_rot,
            );
            transformed_corners.push(obstacle_pos + rotated);
        }

        // 找到最左和最右的角點（相對於觀察者）
        let mut angles: Vec<_> = transformed_corners.iter()
            .map(|&corner| {
                let to_corner = corner - observer_pos;
                to_corner.y.atan2(to_corner.x)
            })
            .collect();
        
        angles.sort_by(|a, b| a.partial_cmp(b).unwrap());
        
        let min_angle = angles[0];
        let max_angle = angles[angles.len() - 1];
        
        // 處理角度跨越問題
        let (start_angle, end_angle) = if max_angle - min_angle > std::f32::consts::PI {
            (max_angle, min_angle + 2.0 * std::f32::consts::PI)
        } else {
            (min_angle, max_angle)
        };

        let distance = (obstacle_pos - observer_pos).magnitude();
        let height_ratio = obstacle_height / observer_height;
        let shadow_length = (vision_range - distance) * height_ratio.min(2.0);

        Some(ShadowArea {
            shadow_type: ShadowType::Object,
            blocker_id: None,
            geometry: ShadowGeometry::Sector {
                center: observer_pos,
                start_angle,
                end_angle,
                radius: distance + shadow_length,
            },
            depth: shadow_length,
        })
    }

    /// 計算地形陰影
    fn calculate_terrain_shadow(
        observer_pos: Vec2<f32>,
        terrain_pos: Vec2<f32>,
        elevation: f32,
        observer_height: f32,
        vision_range: f32,
    ) -> Option<ShadowArea> {
        // 地形只有在高於觀察者時才產生陰影
        if elevation <= observer_height {
            return None;
        }

        let to_terrain = terrain_pos - observer_pos;
        let distance = to_terrain.magnitude();
        
        if distance > vision_range {
            return None;
        }

        let center_angle = to_terrain.y.atan2(to_terrain.x);
        
        // 地形陰影通常較小
        let angle_width = 0.1; // 約5.7度
        let height_ratio = elevation / observer_height;
        let shadow_length = (vision_range - distance) * (height_ratio - 1.0).min(1.0);

        Some(ShadowArea {
            shadow_type: ShadowType::Terrain,
            blocker_id: None,
            geometry: ShadowGeometry::Sector {
                center: observer_pos,
                start_angle: center_angle - angle_width,
                end_angle: center_angle + angle_width,
                radius: distance + shadow_length,
            },
            depth: shadow_length,
        })
    }

    /// 合併相鄰的陰影
    pub fn merge_adjacent_shadows(shadows: Vec<ShadowArea>) -> Vec<ShadowArea> {
        if shadows.len() <= 1 {
            return shadows;
        }

        let mut sector_shadows: Vec<_> = shadows.into_iter()
            .filter(|s| matches!(s.geometry, ShadowGeometry::Sector { .. }))
            .collect();

        if sector_shadows.is_empty() {
            return Vec::new();
        }

        // 按起始角度排序
        sector_shadows.sort_by(|a, b| {
            if let (ShadowGeometry::Sector { start_angle: a_start, .. },
                    ShadowGeometry::Sector { start_angle: b_start, .. }) = (&a.geometry, &b.geometry) {
                a_start.partial_cmp(b_start).unwrap_or(std::cmp::Ordering::Equal)
            } else {
                std::cmp::Ordering::Equal
            }
        });

        let mut merged = Vec::new();
        let mut current = sector_shadows[0].clone();

        for next in sector_shadows.into_iter().skip(1) {
            if Self::can_merge_shadows(&current, &next) {
                current = Self::merge_two_shadows(current, next);
            } else {
                merged.push(current);
                current = next;
            }
        }
        
        merged.push(current);
        merged
    }

    /// 檢查兩個陰影是否可以合併
    fn can_merge_shadows(shadow1: &ShadowArea, shadow2: &ShadowArea) -> bool {
        if let (ShadowGeometry::Sector { center: c1, end_angle: e1, .. },
                ShadowGeometry::Sector { center: c2, start_angle: s2, .. }) 
                = (&shadow1.geometry, &shadow2.geometry) {
            
            // 中心相同且角度相鄰（容許小間隙）
            c1.distance(*c2) < 1.0 && (*s2 - *e1).abs() < 0.1
        } else {
            false
        }
    }

    /// 合併兩個陰影
    fn merge_two_shadows(shadow1: ShadowArea, shadow2: ShadowArea) -> ShadowArea {
        if let (ShadowGeometry::Sector { center, start_angle: s1, radius: r1, .. },
                ShadowGeometry::Sector { end_angle: e2, radius: r2, .. }) 
                = (&shadow1.geometry, &shadow2.geometry) {
            
            ShadowArea {
                shadow_type: shadow1.shadow_type,
                blocker_id: shadow1.blocker_id,
                geometry: ShadowGeometry::Sector {
                    center: *center,
                    start_angle: *s1,
                    end_angle: *e2,
                    radius: r1.max(*r2),
                },
                depth: shadow1.depth.max(shadow2.depth),
            }
        } else {
            shadow1
        }
    }

    /// 優化陰影列表（移除重疊和無效陰影）
    pub fn optimize_shadows(shadows: Vec<ShadowArea>) -> Vec<ShadowArea> {
        let mut optimized = Vec::new();
        
        for shadow in shadows {
            // 檢查陰影是否有效
            if Self::is_valid_shadow(&shadow) {
                // 檢查是否與已有陰影重疊
                if !Self::is_shadow_redundant(&shadow, &optimized) {
                    optimized.push(shadow);
                }
            }
        }

        optimized
    }

    /// 檢查陰影是否有效
    fn is_valid_shadow(shadow: &ShadowArea) -> bool {
        match &shadow.geometry {
            ShadowGeometry::Sector { start_angle, end_angle, radius, .. } => {
                *radius > 0.0 && (*end_angle - *start_angle).abs() > 0.01
            }
            ShadowGeometry::Polygon { vertices } => {
                vertices.len() >= 3
            }
            ShadowGeometry::Trapezoid { .. } => {
                true
            }
        }
    }

    /// 檢查陰影是否冗余
    fn is_shadow_redundant(shadow: &ShadowArea, existing: &[ShadowArea]) -> bool {
        for existing_shadow in existing {
            if Self::shadow_contains_shadow(existing_shadow, shadow) {
                return true;
            }
        }
        false
    }

    /// 檢查一個陰影是否完全包含另一個陰影
    fn shadow_contains_shadow(container: &ShadowArea, contained: &ShadowArea) -> bool {
        // 簡化實現：只處理扇形陰影
        if let (ShadowGeometry::Sector { center: c1, start_angle: s1, end_angle: e1, radius: r1 },
                ShadowGeometry::Sector { center: c2, start_angle: s2, end_angle: e2, radius: r2 }) 
                = (&container.geometry, &contained.geometry) {
            
            c1.distance(*c2) < 1.0 && // 同心
            *s1 <= *s2 && *e1 >= *e2 && // 角度包含
            *r1 >= *r2 // 半徑包含
        } else {
            false
        }
    }
}