/// 視野計算核心

use vek::Vec2;
use std::time::{SystemTime, UNIX_EPOCH};

use super::components::*;
use crate::vision::{ShadowCalculator, Bounds};

/// 視野計算器
pub struct VisionCalculator {
    shadow_calculator: ShadowCalculator,
}

impl VisionCalculator {
    /// 創建新的視野計算器
    pub fn new() -> Self {
        Self {
            shadow_calculator: ShadowCalculator::new(),
        }
    }

    /// 配置計算器參數
    pub fn with_config(
        max_cache_size: usize,
        max_tree_depth: usize,
        max_obstacles_per_node: usize,
    ) -> Self {
        Self {
            shadow_calculator: ShadowCalculator::with_config(
                max_cache_size,
                max_tree_depth,
                max_obstacles_per_node,
            ),
        }
    }

    /// 初始化障礙物
    pub fn initialize_obstacles(&mut self, world_bounds: Bounds, obstacles: Vec<ObstacleInfo>) {
        // 轉換為shadow_calculator期待的格式
        let shadow_obstacles: Vec<crate::comp::circular_vision::ObstacleInfo> = obstacles
            .into_iter()
            .map(|obs| self.convert_obstacle_info(obs))
            .collect();

        self.shadow_calculator.initialize_quadtree(world_bounds, shadow_obstacles);
    }

    /// 計算圓形視野
    pub fn calculate_circular_vision(
        &mut self,
        observer_pos: Vec2<f32>,
        vision: &CircularVision,
    ) -> VisionResult {
        // 使用優化的陰影計算器
        let shadow_result = self.shadow_calculator.calculate_optimized_vision(
            observer_pos,
            vision.height,
            vision.range,
        );

        // 轉換結果格式
        VisionResult {
            observer_pos,
            range: vision.range,
            visible_area: shadow_result.visible_area,
            shadows: shadow_result.shadows.into_iter()
                .map(|s| self.convert_shadow_area(s))
                .collect(),
            timestamp: self.current_time(),
        }
    }

    /// 計算基礎圓形視野（無陰影）
    pub fn calculate_basic_circular_vision(
        observer_pos: Vec2<f32>,
        range: f32,
        precision: u32,
    ) -> VisionResult {
        let mut visible_area = Vec::new();
        let angle_step = 2.0 * std::f32::consts::PI / precision as f32;

        for i in 0..precision {
            let angle = i as f32 * angle_step;
            let direction = Vec2::new(angle.cos(), angle.sin());
            let point = observer_pos + direction * range;
            visible_area.push(point);
        }

        VisionResult {
            observer_pos,
            range,
            visible_area,
            shadows: Vec::new(),
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64(),
        }
    }

    /// 計算兩點間的視線是否被遮擋
    pub fn is_line_of_sight_clear(
        &self,
        start: Vec2<f32>,
        end: Vec2<f32>,
        obstacles: &[ObstacleInfo],
    ) -> bool {
        let direction = (end - start).normalized();
        let distance = (end - start).magnitude();

        for obstacle in obstacles {
            if self.line_intersects_obstacle(start, direction, distance, obstacle) {
                return false;
            }
        }

        true
    }

    /// 檢查射線是否與障礙物相交
    fn line_intersects_obstacle(
        &self,
        start: Vec2<f32>,
        direction: Vec2<f32>,
        max_distance: f32,
        obstacle: &ObstacleInfo,
    ) -> bool {
        match &obstacle.obstacle_type {
            ObstacleType::Circular { radius } => {
                self.line_intersects_circle(start, direction, max_distance, obstacle.position, *radius)
            }
            ObstacleType::Rectangle { width, height, rotation: _ } => {
                // 簡化為圓形檢測
                let effective_radius = (width * width + height * height).sqrt() * 0.5;
                self.line_intersects_circle(start, direction, max_distance, obstacle.position, effective_radius)
            }
            ObstacleType::Terrain { .. } => {
                // 地形通常不會完全遮擋視線
                false
            }
        }
    }

    /// 檢查射線是否與圓形相交
    fn line_intersects_circle(
        &self,
        start: Vec2<f32>,
        direction: Vec2<f32>,
        max_distance: f32,
        circle_center: Vec2<f32>,
        radius: f32,
    ) -> bool {
        let to_center = circle_center - start;
        let proj_length = to_center.dot(direction);

        // 射線背向圓心或超出最大距離
        if proj_length < 0.0 || proj_length > max_distance {
            return false;
        }

        let closest_point = start + direction * proj_length;
        let distance_to_center = (circle_center - closest_point).magnitude();

        distance_to_center <= radius
    }

    /// 轉換障礙物信息格式
    fn convert_obstacle_info(&self, obs: ObstacleInfo) -> crate::comp::circular_vision::ObstacleInfo {
        let obstacle_type = match obs.obstacle_type {
            ObstacleType::Circular { radius } => {
                crate::comp::circular_vision::ObstacleType::Circular { radius }
            }
            ObstacleType::Rectangle { width, height, rotation } => {
                crate::comp::circular_vision::ObstacleType::Rectangle { width, height, rotation }
            }
            ObstacleType::Terrain { elevation } => {
                crate::comp::circular_vision::ObstacleType::Terrain { elevation }
            }
        };

        crate::comp::circular_vision::ObstacleInfo {
            position: obs.position,
            obstacle_type,
            height: obs.height,
            properties: crate::comp::circular_vision::ObstacleProperties {
                blocks_completely: obs.properties.blocks_completely,
                opacity: obs.properties.opacity,
                shadow_multiplier: obs.properties.shadow_multiplier,
            },
        }
    }

    /// 轉換陰影區域格式
    fn convert_shadow_area(&self, shadow: crate::comp::circular_vision::ShadowArea) -> ShadowArea {
        let shadow_type = match shadow.shadow_type {
            crate::comp::circular_vision::ShadowType::Object => ShadowType::Object,
            crate::comp::circular_vision::ShadowType::Building => ShadowType::Building,
            crate::comp::circular_vision::ShadowType::Terrain => ShadowType::Terrain,
            crate::comp::circular_vision::ShadowType::Sector => ShadowType::Sector,
            crate::comp::circular_vision::ShadowType::Trapezoid => ShadowType::Sector, // Map to closest equivalent
            crate::comp::circular_vision::ShadowType::Temporary => ShadowType::Temporary,
        };

        let geometry = match shadow.geometry {
            crate::comp::circular_vision::ShadowGeometry::Sector { center, start_angle, end_angle, radius } => {
                ShadowGeometry::Sector { center, start_angle, end_angle, radius }
            }
            crate::comp::circular_vision::ShadowGeometry::Polygon { vertices } => {
                ShadowGeometry::Polygon { vertices }
            }
            crate::comp::circular_vision::ShadowGeometry::Trapezoid { vertices } => {
                ShadowGeometry::Trapezoid { vertices }
            }
        };

        ShadowArea {
            shadow_type,
            blocker_id: shadow.blocker_id,
            geometry,
            depth: shadow.depth,
        }
    }

    /// 獲取當前時間戳
    fn current_time(&self) -> f64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
    }

    /// 更新障礙物
    pub fn update_obstacle(&mut self, obstacle_id: String, obstacle: ObstacleInfo) {
        let converted = self.convert_obstacle_info(obstacle);
        self.shadow_calculator.update_obstacle(obstacle_id, converted);
    }

    /// 移除障礙物
    pub fn remove_obstacle(&mut self, obstacle_id: &str) {
        self.shadow_calculator.remove_obstacle(obstacle_id);
    }

    /// 獲取性能統計
    pub fn get_performance_stats(&self) -> VisionPerformanceStats {
        let shadow_stats = self.shadow_calculator.get_performance_stats();
        
        VisionPerformanceStats {
            cache_size: shadow_stats.cache_size,
            obstacle_count: shadow_stats.obstacle_count,
            quadtree_nodes: shadow_stats.quadtree_nodes,
            max_cache_size: shadow_stats.max_cache_size,
        }
    }
}

impl Default for VisionCalculator {
    fn default() -> Self {
        Self::new()
    }
}

/// 視野性能統計
#[derive(Debug, Clone)]
pub struct VisionPerformanceStats {
    pub cache_size: usize,
    pub obstacle_count: usize,
    pub quadtree_nodes: usize,
    pub max_cache_size: usize,
}