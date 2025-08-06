/// 視野結果管理器

use std::collections::HashMap;
use specs::Entity;
use super::components::*;

/// 視野結果管理器
pub struct ResultManager {
    /// 實體視野結果緩存
    entity_results: HashMap<Entity, VisionResult>,
    /// 視野更新統計
    update_stats: VisionUpdateStats,
}

impl ResultManager {
    /// 創建新的結果管理器
    pub fn new() -> Self {
        Self {
            entity_results: HashMap::new(),
            update_stats: VisionUpdateStats::default(),
        }
    }

    /// 更新實體的視野結果
    pub fn update_entity_vision(&mut self, entity: Entity, result: VisionResult) {
        self.entity_results.insert(entity, result);
        self.update_stats.total_updates += 1;
    }

    /// 獲取實體的視野結果
    pub fn get_entity_vision(&self, entity: Entity) -> Option<&VisionResult> {
        self.entity_results.get(&entity)
    }

    /// 檢查實體是否需要更新視野
    pub fn needs_vision_update(&self, entity: Entity, current_time: f64) -> bool {
        if let Some(result) = self.entity_results.get(&entity) {
            current_time - result.timestamp > 0.1 // 100ms更新間隔
        } else {
            true // 沒有結果，需要更新
        }
    }

    /// 清理過期的視野結果
    pub fn cleanup_expired_results(&mut self, current_time: f64, max_age: f64) {
        let before_count = self.entity_results.len();
        
        self.entity_results.retain(|_, result| {
            current_time - result.timestamp < max_age
        });
        
        let removed_count = before_count - self.entity_results.len();
        self.update_stats.expired_cleanups += removed_count;
    }

    /// 移除實體的視野結果
    pub fn remove_entity_vision(&mut self, entity: Entity) {
        if self.entity_results.remove(&entity).is_some() {
            self.update_stats.manual_removals += 1;
        }
    }

    /// 檢查兩個實體之間的視線
    pub fn check_line_of_sight(&self, observer: Entity, target: Entity) -> Option<bool> {
        let observer_result = self.entity_results.get(&observer)?;
        let target_result = self.entity_results.get(&target)?;
        
        // 檢查目標是否在觀察者的視野內
        Some(observer_result.is_point_visible(target_result.observer_pos))
    }

    /// 獲取實體視野內的其他實體
    pub fn get_entities_in_vision(&self, observer: Entity, all_entities: &[(Entity, Vec2<f32>)]) -> Vec<Entity> {
        if let Some(observer_result) = self.entity_results.get(&observer) {
            all_entities.iter()
                .filter(|(entity, pos)| {
                    *entity != observer && observer_result.is_point_visible(*pos)
                })
                .map(|(entity, _)| *entity)
                .collect()
        } else {
            Vec::new()
        }
    }

    /// 計算實體間的視野覆蓋
    pub fn calculate_vision_overlap(&self, entity1: Entity, entity2: Entity) -> Option<f32> {
        let result1 = self.entity_results.get(&entity1)?;
        let result2 = self.entity_results.get(&entity2)?;
        
        // 簡化計算：檢查兩個視野圓的重疊
        let distance = (result1.observer_pos - result2.observer_pos).magnitude();
        let total_range = result1.range + result2.range;
        
        if distance >= total_range {
            Some(0.0) // 無重疊
        } else if distance <= (result1.range - result2.range).abs() {
            // 一個完全包含另一個
            let smaller_area = std::f32::consts::PI * result1.range.min(result2.range).powi(2);
            let larger_area = std::f32::consts::PI * result1.range.max(result2.range).powi(2);
            Some(smaller_area / larger_area)
        } else {
            // 部分重疊，使用近似計算
            let overlap_factor = (total_range - distance) / total_range;
            Some(overlap_factor * 0.5) // 近似值
        }
    }

    /// 批量更新多個實體的視野
    pub fn batch_update_visions(&mut self, updates: Vec<(Entity, VisionResult)>) {
        for (entity, result) in updates {
            self.entity_results.insert(entity, result);
        }
        self.update_stats.total_updates += updates.len();
        self.update_stats.batch_updates += 1;
    }

    /// 獲取視野品質評分
    pub fn get_vision_quality_score(&self, entity: Entity) -> Option<f32> {
        let result = self.entity_results.get(&entity)?;
        
        // 計算視野品質評分
        let total_area = std::f32::consts::PI * result.range.powi(2);
        let visible_area = result.get_visible_area();
        let visibility_ratio = visible_area / total_area;
        
        // 考慮陰影數量對性能的影響
        let shadow_penalty = (result.shadows.len() as f32 * 0.01).min(0.2);
        
        Some((visibility_ratio - shadow_penalty).max(0.0))
    }

    /// 獲取統計資訊
    pub fn get_stats(&self) -> &VisionUpdateStats {
        &self.update_stats
    }

    /// 重置統計資訊
    pub fn reset_stats(&mut self) {
        self.update_stats = VisionUpdateStats::default();
    }

    /// 獲取緩存大小
    pub fn get_cache_size(&self) -> usize {
        self.entity_results.len()
    }

    /// 清空所有結果
    pub fn clear_all(&mut self) {
        self.entity_results.clear();
        self.update_stats.manual_removals += 1;
    }

    /// 導出視野數據（用於調試）
    pub fn export_vision_data(&self) -> HashMap<Entity, VisionExportData> {
        self.entity_results.iter()
            .map(|(entity, result)| {
                let export_data = VisionExportData {
                    observer_pos: result.observer_pos,
                    range: result.range,
                    visible_area_size: result.get_visible_area(),
                    shadow_count: result.shadows.len(),
                    timestamp: result.timestamp,
                };
                (*entity, export_data)
            })
            .collect()
    }
}

impl Default for ResultManager {
    fn default() -> Self {
        Self::new()
    }
}

/// 視野更新統計
#[derive(Debug, Default, Clone)]
pub struct VisionUpdateStats {
    /// 總更新次數
    pub total_updates: usize,
    /// 批量更新次數
    pub batch_updates: usize,
    /// 手動移除次數
    pub manual_removals: usize,
    /// 過期清理次數
    pub expired_cleanups: usize,
}

/// 視野導出數據
#[derive(Debug, Clone)]
pub struct VisionExportData {
    pub observer_pos: Vec2<f32>,
    pub range: f32,
    pub visible_area_size: f32,
    pub shadow_count: usize,
    pub timestamp: f64,
}