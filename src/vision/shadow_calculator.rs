/// 高效能陰影計算器
/// 
/// 提供空間分割優化、陰影合併、增量計算等性能優化功能
use vek::Vec2;
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::comp::circular_vision::{VisionResult, ShadowArea, ObstacleInfo};

// Import refactored modules from the vision module
use crate::vision::quadtree::{QuadTree, Bounds};
use crate::vision::shadow_calculation::ShadowCalculator as ShadowCalc;
use crate::vision::vision_cache::{CacheManager, CacheStats};
use crate::vision::geometry_utils::GeometryUtils;

/// 高效能陰影計算器
pub struct ShadowCalculator {
    /// 四叉樹
    quadtree: QuadTree,
    /// 緩存管理器
    cache_manager: CacheManager,
    /// 障礙物位置索引
    obstacle_index: BTreeMap<String, ObstacleInfo>,
}

impl ShadowCalculator {
    /// 創建新的陰影計算器
    pub fn new() -> Self {
        Self {
            quadtree: QuadTree::new(8, 10),
            cache_manager: CacheManager::new(1000),
            obstacle_index: BTreeMap::new(),
        }
    }

    /// 配置計算器參數
    pub fn with_config(
        max_cache_size: usize,
        max_tree_depth: usize,
        max_obstacles_per_node: usize,
    ) -> Self {
        Self {
            quadtree: QuadTree::new(max_tree_depth, max_obstacles_per_node),
            cache_manager: CacheManager::new(max_cache_size),
            obstacle_index: BTreeMap::new(),
        }
    }

    /// 初始化四叉樹
    pub fn initialize_quadtree(&mut self, world_bounds: Bounds, obstacles: Vec<ObstacleInfo>) {
        // 重建障礙物索引
        self.obstacle_index.clear();
        for (i, obstacle) in obstacles.iter().enumerate() {
            let id = format!("obstacle_{}", i);
            self.obstacle_index.insert(id, obstacle.clone());
        }

        // 初始化四叉樹
        self.quadtree.initialize(world_bounds, obstacles);

        // 清理可能失效的緩存
        self.cache_manager.invalidate_all_cache();
    }

    /// 高效率視野計算（使用空間分割優化）
    pub fn calculate_optimized_vision(
        &mut self,
        observer_pos: Vec2<f32>,
        observer_height: f32,
        vision_range: f32,
    ) -> VisionResult {
        let cache_key = format!("{:.1}_{:.1}_{:.1}_{:.1}", 
            observer_pos.x, observer_pos.y, observer_height, vision_range);

        // 檢查緩存
        if let Some(cached) = self.cache_manager.get_cached_vision(&cache_key) {
            return cached.result.clone();
        }

        // 使用四叉樹查詢相關障礙物
        let relevant_obstacles = self.quadtree.query_obstacles_in_range(observer_pos, vision_range);

        // 使用陰影投射算法
        let mut shadows = Vec::new();
        for obstacle in &relevant_obstacles {
            if let Some(shadow) = ShadowCalc::calculate_obstacle_shadow(
                observer_pos, 
                observer_height, 
                vision_range, 
                obstacle
            ) {
                shadows.push(shadow);
            }
        }

        // 合併重疊的陰影
        shadows = ShadowCalc::merge_overlapping_shadows(shadows);

        // 計算可見區域
        let visible_area = ShadowCalc::calculate_visible_area(observer_pos, vision_range, &shadows);

        let result = VisionResult {
            observer_pos,
            range: vision_range,
            visible_area,
            shadows,
            timestamp: self.current_time(),
        };

        // 緩存結果
        let dependencies = relevant_obstacles.iter()
            .enumerate()
            .map(|(i, _)| format!("obstacle_{}", i))
            .collect();
            
        self.cache_manager.cache_vision_result(cache_key, result.clone(), dependencies);

        result
    }

    /// 增量更新障礙物
    pub fn update_obstacle(&mut self, obstacle_id: String, obstacle: ObstacleInfo) {
        self.obstacle_index.insert(obstacle_id.clone(), obstacle);
        
        // 使相關緩存失效
        self.cache_manager.invalidate_cache_for_obstacle(&obstacle_id);
        
        // TODO: 增量更新四叉樹而非重建
        // 目前簡化為標記需要重建
    }

    /// 移除障礙物
    pub fn remove_obstacle(&mut self, obstacle_id: &str) {
        self.obstacle_index.remove(obstacle_id);
        self.cache_manager.invalidate_cache_for_obstacle(obstacle_id);
    }

    /// 獲取當前時間戳
    fn current_time(&self) -> f64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
    }

    /// 獲取性能統計
    pub fn get_performance_stats(&self) -> PerformanceStats {
        let cache_stats = self.cache_manager.get_cache_stats();
        PerformanceStats {
            cache_size: cache_stats.cache_size,
            obstacle_count: self.obstacle_index.len(),
            quadtree_nodes: self.quadtree.count_nodes(),
            max_cache_size: cache_stats.max_cache_size,
        }
    }

    /// 射線與線段相交檢測（暴露給外部使用）
    pub fn ray_line_intersection(
        ray_origin: Vec2<f32>,
        ray_direction: Vec2<f32>,
        line_start: Vec2<f32>,
        line_end: Vec2<f32>,
    ) -> Option<f32> {
        ShadowCalc::ray_line_intersection(ray_origin, ray_direction, line_start, line_end)
    }

    /// 射線與扇形相交檢測（改進版本）
    pub fn ray_sector_intersection_improved(
        origin: Vec2<f32>,
        direction: Vec2<f32>,
        center: Vec2<f32>,
        start_angle: f32,
        end_angle: f32,
        radius: f32,
    ) -> Option<f32> {
        GeometryUtils::ray_sector_intersection_improved(
            origin, direction, center, start_angle, end_angle, radius
        )
    }

    /// 檢查角度是否在扇形範圍內
    pub fn angle_in_sector(angle: f32, start_angle: f32, end_angle: f32) -> bool {
        GeometryUtils::angle_in_sector(angle, start_angle, end_angle)
    }

    /// 檢查兩個扇形是否重疊或相鄰
    pub fn sectors_overlap_or_adjacent(start1: f32, end1: f32, start2: f32, end2: f32) -> bool {
        ShadowCalc::sectors_overlap_or_adjacent(start1, end1, start2, end2)
    }
}

/// 性能統計信息
#[derive(Debug, Clone)]
pub struct PerformanceStats {
    pub cache_size: usize,
    pub obstacle_count: usize,
    pub quadtree_nodes: usize,
    pub max_cache_size: usize,
}

impl Default for ShadowCalculator {
    fn default() -> Self {
        Self::new()
    }
}