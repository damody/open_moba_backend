/// 重構後的圓形視野系統
/// 
/// 將原本的大型 circular_vision.rs 拆分為多個模組以提升可維護性

pub use crate::comp::vision::{
    VisionCalculator, ShadowSystem, ResultManager,
    components::{CircularVision, VisionResult, ShadowArea, ObstacleInfo},
};

// 重新導出原有的障礙物相關類型
pub use crate::comp::circular_vision::{
    ShadowType, ShadowGeometry, ObstacleType, ObstacleProperties,
};

use specs::{World, Entity, ReadStorage, WriteStorage, Join, WorldExt};
use vek::Vec2;
use crate::comp::Pos;

/// 視野系統管理器
pub struct VisionSystemManager {
    calculator: VisionCalculator,
    result_manager: ResultManager,
}

impl VisionSystemManager {
    /// 創建新的視野系統管理器
    pub fn new() -> Self {
        Self {
            calculator: VisionCalculator::new(),
            result_manager: ResultManager::new(),
        }
    }

    /// 配置視野系統
    pub fn with_config(
        max_cache_size: usize,
        max_tree_depth: usize,
        max_obstacles_per_node: usize,
    ) -> Self {
        Self {
            calculator: VisionCalculator::with_config(
                max_cache_size,
                max_tree_depth,
                max_obstacles_per_node,
            ),
            result_manager: ResultManager::new(),
        }
    }

    /// 更新所有實體的視野
    pub fn update_all_visions(
        &mut self,
        world: &World,
        current_time: f64,
    ) -> Result<(), String> {
        let entities = world.entities();
        let positions = world.read_storage::<Pos>();
        let mut visions = world.write_storage::<CircularVision>();

        for (entity, pos, vision) in (&entities, &positions, &mut visions).join() {
            if self.result_manager.needs_vision_update(entity, current_time) || 
               vision.needs_recalculation(current_time) {
                
                let result = self.calculator.calculate_circular_vision(pos.0, &*vision);
                vision.vision_result = Some(result.clone());
                self.result_manager.update_entity_vision(entity, result);
            }
        }

        // 清理過期結果
        self.result_manager.cleanup_expired_results(current_time, 5.0);

        Ok(())
    }

    /// 更新單個實體的視野
    pub fn update_entity_vision(
        &mut self,
        entity: Entity,
        position: Vec2<f32>,
        vision: &mut CircularVision,
        current_time: f64,
    ) -> Option<VisionResult> {
        if self.result_manager.needs_vision_update(entity, current_time) || 
           vision.needs_recalculation(current_time) {
            
            let result = self.calculator.calculate_circular_vision(position, &*vision);
            vision.vision_result = Some(result.clone());
            self.result_manager.update_entity_vision(entity, result.clone());
            Some(result)
        } else {
            self.result_manager.get_entity_vision(entity).cloned()
        }
    }

    /// 檢查兩點間視線
    pub fn check_line_of_sight(
        &self,
        start: Vec2<f32>,
        end: Vec2<f32>,
        obstacles: &[ObstacleInfo],
    ) -> bool {
        self.calculator.is_line_of_sight_clear(start, end, obstacles)
    }

    /// 獲取實體視野內的其他實體
    pub fn get_entities_in_vision(
        &self,
        observer: Entity,
        world: &World,
    ) -> Vec<Entity> {
        let entities = world.entities();
        let positions = world.read_storage::<Pos>();
        
        let entity_positions: Vec<_> = (&entities, &positions)
            .join()
            .map(|(e, pos)| (e, pos.0))
            .collect();

        self.result_manager.get_entities_in_vision(observer, &entity_positions)
    }

    /// 初始化障礙物
    pub fn initialize_obstacles(
        &mut self,
        world_bounds: crate::vision::Bounds,
        obstacles: Vec<ObstacleInfo>,
    ) {
        self.calculator.initialize_obstacles(world_bounds, obstacles);
    }

    /// 添加障礙物
    pub fn add_obstacle(&mut self, obstacle_id: String, obstacle: ObstacleInfo) {
        self.calculator.update_obstacle(obstacle_id, obstacle);
    }

    /// 移除障礙物
    pub fn remove_obstacle(&mut self, obstacle_id: &str) {
        self.calculator.remove_obstacle(obstacle_id);
    }

    /// 獲取性能統計
    pub fn get_performance_stats(&self) -> VisionSystemStats {
        VisionSystemStats {
            total_calculations: 0, // 暫時硬編碼
            average_time: 0.0,     // 暫時硬編碼
            cached_results: 0,     // 暫時硬編碼
        }
    }

    /// 重置統計
    pub fn reset_stats(&mut self) {
        self.result_manager.reset_stats();
    }

    /// 清理所有緩存
    pub fn clear_all_cache(&mut self) {
        self.result_manager.clear_all();
    }

    /// 導出調試數據
    pub fn export_debug_data(&self) -> VisionDebugData {
        VisionDebugData {
            debug_info: "Debug info placeholder".to_string(),
            performance_stats: self.get_performance_stats(),
        }
    }
}

impl Default for VisionSystemManager {
    fn default() -> Self {
        Self::new()
    }
}

/// 視野系統統計
#[derive(Debug, Clone)]
pub struct VisionSystemStats {
    pub total_calculations: usize,
    pub average_time: f32,
    pub cached_results: usize,
}

/// 視野調試數據
#[derive(Debug, Clone)]
pub struct VisionDebugData {
    pub debug_info: String,
    pub performance_stats: VisionSystemStats,
}

// 輔助函數
impl VisionSystemManager {
    /// 快速創建基礎圓形視野（無障礙物）
    pub fn create_basic_vision(
        observer_pos: Vec2<f32>,
        range: f32,
        precision: u32,
    ) -> VisionResult {
        VisionCalculator::calculate_basic_circular_vision(observer_pos, range, precision)
    }

    /// 計算視野覆蓋率
    pub fn calculate_vision_coverage(
        &self,
        entity1: Entity,
        entity2: Entity,
    ) -> Option<f32> {
        self.result_manager.calculate_vision_overlap(entity1, entity2)
    }

    /// 獲取視野品質評分
    pub fn get_vision_quality(&self, entity: Entity) -> Option<f32> {
        self.result_manager.get_vision_quality_score(entity)
    }
}