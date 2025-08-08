/// 視野系統的 ECS 整合
/// 
/// 提供視野更新系統、結果緩存、事件過濾等 ECS 相關功能
use specs::prelude::*;
use vek::Vec2;
use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::comp::{
    CircularVision, Pos, Player, VisionResult,
};
use crate::vision::{VisionOutputGenerator, ShadowCalculator, GridVisionOutput, VectorVisionOutput};

/// 視野結果緩存資源
#[derive(Default)]
pub struct VisionResultCache {
    /// 玩家視野結果
    pub player_visions: HashMap<String, VisionResult>,
    /// 實體視野結果
    pub entity_visions: HashMap<Entity, VisionResult>,
    /// 最後更新時間
    pub last_update: HashMap<String, f64>,
    /// 輸出格式緩存
    pub output_cache: HashMap<String, (GridVisionOutput, VectorVisionOutput)>,
}

/// 視野更新系統
pub struct VisionUpdateSystem {
    /// 陰影計算器
    shadow_calculator: ShadowCalculator,
    /// 輸出生成器
    output_generator: VisionOutputGenerator,
    /// 更新間隔（秒）
    update_interval: f64,
}

/// 視野事件過濾系統
pub struct VisionEventFilter {
    /// 可見性緩存
    visibility_cache: HashMap<(String, Entity), bool>,
}

impl VisionUpdateSystem {
    /// 創建新的視野更新系統
    pub fn new() -> Self {
        Self {
            shadow_calculator: ShadowCalculator::with_config(500, 6, 8), // 中等性能配置
            output_generator: VisionOutputGenerator::new(25.0),
            update_interval: 1.0 / 30.0, // 30 FPS 更新頻率
        }
    }

    /// 配置系統參數
    pub fn with_config(
        mut self,
        update_frequency: f64,
        cache_size: usize,
        tree_depth: usize,
    ) -> Self {
        self.update_interval = 1.0 / update_frequency;
        self.shadow_calculator = ShadowCalculator::with_config(cache_size, tree_depth, 10);
        self
    }

    /// 初始化障礙物
    pub fn initialize_obstacles(&mut self, obstacles: Vec<crate::comp::circular_vision::ObstacleInfo>) {
        use crate::vision::Bounds;
        
        // 假設地圖範圍 4000x4000
        let world_bounds = Bounds::new(
            Vec2::new(0.0, 0.0),
            Vec2::new(4000.0, 4000.0)
        );
        
        self.shadow_calculator.initialize_quadtree(world_bounds, obstacles);
    }

    /// 當前時間戳
    fn current_time(&self) -> f64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
    }
}

impl<'a> System<'a> for VisionUpdateSystem {
    type SystemData = (
        Entities<'a>,
        ReadStorage<'a, Pos>,
        WriteStorage<'a, CircularVision>,
        ReadStorage<'a, Player>,
        Write<'a, VisionResultCache>,
    );

    fn run(&mut self, (entities, positions, mut visions, players, mut cache): Self::SystemData) {
        let current_time = self.current_time();

        // 更新所有具有視野的實體
        for (entity, position, vision, player_opt) in (&entities, &positions, &mut visions, (&players).maybe()).join() {
            let player_name = player_opt.map(|p| p.name.clone());
            
            // 檢查是否需要更新
            let needs_update = if let Some(ref player_name) = player_name {
                if let Some(last_update) = cache.last_update.get(player_name) {
                    current_time - last_update > self.update_interval
                } else {
                    true
                }
            } else {
                // 非玩家實體，檢查視野結果是否存在或過時
                vision.vision_result.is_none() || 
                (vision.vision_result.as_ref().unwrap().timestamp + self.update_interval < current_time)
            };

            if !needs_update {
                continue;
            }

            // 計算新的視野
            let vision_result = self.shadow_calculator.calculate_optimized_vision(
                position.0,
                vision.height,
                vision.range,
            );

            // 更新視野組件
            vision.vision_result = Some(vision_result.clone());

            // 緩存結果
            if let Some(player_name) = player_name {
                cache.player_visions.insert(player_name.clone(), vision_result.clone());
                cache.last_update.insert(player_name.clone(), current_time);
                
                // 生成輸出格式
                let grid_output = self.output_generator.generate_grid_output(&vision_result, None);
                let vector_output = self.output_generator.generate_vector_output(&vision_result);
                cache.output_cache.insert(player_name, (grid_output, vector_output));
            } else {
                cache.entity_visions.insert(entity, vision_result);
            }
        }

        // 清理過時的緩存
        self.cleanup_expired_cache(&mut cache, current_time);
    }
}

impl VisionUpdateSystem {
    /// 清理過時的緩存
    fn cleanup_expired_cache(&mut self, cache: &mut VisionResultCache, current_time: f64) {
        let expire_time = 60.0; // 60秒後過期
        
        // 清理玩家視野緩存
        let expired_players: Vec<String> = cache.last_update
            .iter()
            .filter_map(|(player, time)| {
                if current_time - time > expire_time {
                    Some(player.clone())
                } else {
                    None
                }
            })
            .collect();

        for player in expired_players {
            cache.player_visions.remove(&player);
            cache.last_update.remove(&player);
            cache.output_cache.remove(&player);
        }

        // 限制輸出生成器緩存大小
        self.output_generator.limit_cache_size(100);
    }

    /// 添加障礙物
    pub fn add_obstacle(&mut self, obstacle_id: String, obstacle: crate::comp::circular_vision::ObstacleInfo) {
        self.shadow_calculator.update_obstacle(obstacle_id, obstacle);
    }

    /// 移除障礙物
    pub fn remove_obstacle(&mut self, obstacle_id: &str) {
        self.shadow_calculator.remove_obstacle(obstacle_id);
    }

    /// 獲取性能統計
    pub fn get_performance_stats(&self) -> crate::vision::shadow_calculator::PerformanceStats {
        self.shadow_calculator.get_performance_stats()
    }
}

impl Default for VisionUpdateSystem {
    fn default() -> Self {
        Self::new()
    }
}

// 視野事件過濾系統
impl VisionEventFilter {
    /// 創建新的事件過濾器
    pub fn new() -> Self {
        Self {
            visibility_cache: HashMap::new(),
        }
    }

    /// 檢查實體是否對玩家可見
    pub fn is_entity_visible_to_player(
        &mut self,
        player_name: &str,
        entity: Entity,
        entity_position: Vec2<f32>,
        cache: &VisionResultCache,
    ) -> bool {
        let cache_key = (player_name.to_string(), entity);
        
        // 檢查緩存
        if let Some(&visibility) = self.visibility_cache.get(&cache_key) {
            return visibility;
        }

        // 計算可見性
        let is_visible = if let Some(player_vision) = cache.player_visions.get(player_name) {
            self.point_in_visible_area(entity_position, &player_vision.visible_area)
        } else {
            false
        };

        // 緩存結果
        self.visibility_cache.insert(cache_key, is_visible);
        is_visible
    }

    /// 檢查點是否在可見區域內
    fn point_in_visible_area(&self, point: Vec2<f32>, visible_area: &[Vec2<f32>]) -> bool {
        if visible_area.len() < 3 {
            return false;
        }

        // 使用射線投射算法檢查點是否在多邊形內
        let mut inside = false;
        let mut j = visible_area.len() - 1;

        for i in 0..visible_area.len() {
            if ((visible_area[i].y > point.y) != (visible_area[j].y > point.y)) &&
               (point.x < (visible_area[j].x - visible_area[i].x) * (point.y - visible_area[i].y) / 
                         (visible_area[j].y - visible_area[i].y) + visible_area[i].x) {
                inside = !inside;
            }
            j = i;
        }

        inside
    }

    /// 獲取玩家可見的實體列表
    pub fn get_visible_entities_for_player(
        &mut self,
        player_name: &str,
        entities: &Entities,
        positions: &ReadStorage<Pos>,
        cache: &VisionResultCache,
    ) -> Vec<Entity> {
        let mut visible_entities = Vec::new();

        for (entity, position) in (entities, positions).join() {
            if self.is_entity_visible_to_player(player_name, entity, position.0, cache) {
                visible_entities.push(entity);
            }
        }

        visible_entities
    }

    /// 過濾事件列表（僅返回玩家可見的事件）
    pub fn filter_events_for_player<T>(
        &mut self,
        player_name: &str,
        events: &[(Entity, T, Vec2<f32>)],
        cache: &VisionResultCache,
    ) -> Vec<(Entity, T)> 
    where 
        T: Clone,
    {
        events
            .iter()
            .filter_map(|(entity, event, position)| {
                if self.is_entity_visible_to_player(player_name, *entity, *position, cache) {
                    Some((*entity, event.clone()))
                } else {
                    None
                }
            })
            .collect()
    }

    /// 清理過時的可見性緩存
    pub fn cleanup_visibility_cache(&mut self, valid_entities: &HashSet<Entity>) {
        self.visibility_cache.retain(|(_, entity), _| {
            valid_entities.contains(entity)
        });
    }
}

impl Default for VisionEventFilter {
    fn default() -> Self {
        Self::new()
    }
}

// 視野查詢 API
impl VisionResultCache {
    /// 獲取玩家的網格格式視野
    pub fn get_player_grid_vision(&self, player_name: &str) -> Option<&GridVisionOutput> {
        self.output_cache.get(player_name).map(|(grid, _)| grid)
    }

    /// 獲取玩家的向量格式視野
    pub fn get_player_vector_vision(&self, player_name: &str) -> Option<&VectorVisionOutput> {
        self.output_cache.get(player_name).map(|(_, vector)| vector)
    }

    /// 獲取玩家的原始視野結果
    pub fn get_player_vision_result(&self, player_name: &str) -> Option<&VisionResult> {
        self.player_visions.get(player_name)
    }

    /// 獲取實體的視野結果
    pub fn get_entity_vision_result(&self, entity: Entity) -> Option<&VisionResult> {
        self.entity_visions.get(&entity)
    }

    /// 檢查玩家是否有有效視野
    pub fn has_valid_vision(&self, player_name: &str) -> bool {
        self.player_visions.contains_key(player_name)
    }

    /// 獲取所有有視野的玩家
    pub fn get_players_with_vision(&self) -> Vec<String> {
        self.player_visions.keys().cloned().collect()
    }

    /// 清理指定玩家的緩存
    pub fn clear_player_cache(&mut self, player_name: &str) {
        self.player_visions.remove(player_name);
        self.last_update.remove(player_name);
        self.output_cache.remove(player_name);
    }

    /// 清理所有緩存
    pub fn clear_all_cache(&mut self) {
        self.player_visions.clear();
        self.entity_visions.clear();
        self.last_update.clear();
        self.output_cache.clear();
    }
}