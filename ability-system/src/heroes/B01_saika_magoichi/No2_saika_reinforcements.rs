/// 雜賀眾 (saika_reinforcements)
/// 
/// 雜賀孫市的 E 技能 - 召喚技能
/// 
/// 功能：
/// - 在指定位置召喚雜賀鐵炮兵協助作戰
/// - 每級增加召喚數量和持續時間
/// - 支援多個充能，可連續使用
/// - 召喚的單位會自動攻擊附近敵人

use crate::handler::AbilityHandler;
use crate::*;

/// 雜賀眾處理器
pub struct SaikaReinforcementsHandler;

impl SaikaReinforcementsHandler {
    pub fn new() -> Self {
        Self
    }
}

impl AbilityHandler for SaikaReinforcementsHandler {
    fn get_ability_id(&self) -> &str {
        "saika_reinforcements"
    }
    
    fn get_description(&self) -> &str {
        "雜賀眾 - 召喚雜賀鐵炮兵協助作戰，每級增加召喚數量和持續時間"
    }
    
    fn execute(
        &self, 
        request: &AbilityRequest, 
        _config: &AbilityConfig, 
        level_data: &AbilityLevelData
    ) -> Vec<AbilityEffect> {
        let mut effects = Vec::new();
        
        // 獲取召喚位置，如果沒有指定則使用施法者位置
        let summon_position = request.target_position
            .unwrap_or_else(|| vek::Vec2::new(0.0, 0.0)); // TODO: 從 world_access 獲取施法者位置
        
        // 獲取召喚數量
        let summon_count = self.get_custom_int(level_data, "summon_count")
            .unwrap_or(1);
        
        // 獲取持續時間
        let duration = self.get_custom_value(level_data, "duration");
        
        // 創建召喚效果
        effects.push(AbilityEffect::Summon {
            position: summon_position,
            unit_type: "saika_gunner".to_string(),
            count: summon_count,
            duration,
        });
        
        // 如果配置了召喚陣形，可以在這裡處理多個召喚位置
        if summon_count > 1 {
            let formation_radius = self.get_custom_value(level_data, "formation_radius")
                .unwrap_or(100.0);
            
            // 為每個額外的單位創建隨機偏移位置
            for i in 1..summon_count {
                let angle = (i as f32) * std::f32::consts::PI * 2.0 / (summon_count as f32);
                let offset_x = formation_radius * angle.cos();
                let offset_y = formation_radius * angle.sin();
                
                let offset_position = vek::Vec2::new(
                    summon_position.x + offset_x,
                    summon_position.y + offset_y
                );
                
                effects.push(AbilityEffect::Summon {
                    position: offset_position,
                    unit_type: "saika_gunner".to_string(),
                    count: 1,
                    duration,
                });
            }
            
            // 移除第一個效果，因為我們已經分別處理了每個召喚位置
            effects.remove(0);
        }
        
        effects
    }
    
    fn can_execute(
        &self, 
        request: &AbilityRequest, 
        config: &AbilityConfig, 
        state: &AbilityState
    ) -> bool {
        // 檢查基本條件
        self.check_cooldown(state) && 
        self.check_charges(state) &&
        self.check_target(request, config) &&
        self.check_mana(request, config, &AbilityLevelData::default()) &&
        self.check_range(request, config, &AbilityLevelData::default())
    }
    
    fn check_target(&self, request: &AbilityRequest, config: &AbilityConfig) -> bool {
        match config.target_type {
            TargetType::Point => {
                // 召喚技能需要目標位置
                request.target_position.is_some()
            },
            _ => {
                // 其他目標類型使用預設檢查
                true
            }
        }
    }
}

impl Default for SaikaReinforcementsHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use specs::Entity;
    use std::collections::HashMap;
    
    fn create_test_request() -> AbilityRequest {
        AbilityRequest {
            caster: Entity::from_raw(1),
            ability_id: "saika_reinforcements".to_string(),
            level: 2,
            target_position: Some(vek::Vec2::new(100.0, 200.0)),
            target_entity: None,
        }
    }
    
    fn create_test_config() -> AbilityConfig {
        AbilityConfig {
            name: "雜賀眾".to_string(),
            description: "測試".to_string(),
            ability_type: AbilityType::Active,
            target_type: TargetType::Point,
            cast_type: CastType::Instant,
            levels: HashMap::new(),
            properties: HashMap::new(),
        }
    }
    
    fn create_test_level_data() -> AbilityLevelData {
        let mut extra = HashMap::new();
        extra.insert("summon_count".to_string(), serde_json::Value::from(2));
        extra.insert("duration".to_string(), serde_json::Value::from(50.0));
        extra.insert("formation_radius".to_string(), serde_json::Value::from(100.0));
        extra.insert("gunner_hp".to_string(), serde_json::Value::from(350.0));
        extra.insert("gunner_damage".to_string(), serde_json::Value::from(50.0));
        
        AbilityLevelData {
            cooldown: 25.0,
            mana_cost: 90.0,
            cast_time: 0.5,
            range: 800.0,
            extra,
        }
    }
    
    #[test]
    fn test_ability_id() {
        let handler = SaikaReinforcementsHandler::new();
        assert_eq!(handler.get_ability_id(), "saika_reinforcements");
    }
    
    #[test]
    fn test_execute_single_summon() {
        let handler = SaikaReinforcementsHandler::new();
        let mut request = create_test_request();
        let config = create_test_config();
        let mut level_data = create_test_level_data();
        
        // 設置只召喚一個單位
        level_data.extra.insert("summon_count".to_string(), serde_json::Value::from(1));
        
        let effects = handler.execute(&request, &config, &level_data);
        
        // 應該生成一個召喚效果
        assert_eq!(effects.len(), 1);
        
        if let AbilityEffect::Summon { position, unit_type, count, .. } = &effects[0] {
            assert_eq!(*position, vek::Vec2::new(100.0, 200.0));
            assert_eq!(unit_type, "saika_gunner");
            assert_eq!(*count, 1);
        } else {
            panic!("Expected Summon effect");
        }
    }
    
    #[test]
    fn test_execute_multiple_summons() {
        let handler = SaikaReinforcementsHandler::new();
        let request = create_test_request();
        let config = create_test_config();
        let level_data = create_test_level_data(); // summon_count = 2
        
        let effects = handler.execute(&request, &config, &level_data);
        
        // 應該生成 2 個召喚效果（因為 summon_count = 2）
        assert_eq!(effects.len(), 2);
        
        // 檢查所有效果都是召喚效果
        for effect in &effects {
            if let AbilityEffect::Summon { unit_type, count, .. } = effect {
                assert_eq!(unit_type, "saika_gunner");
                assert_eq!(*count, 1);
            } else {
                panic!("Expected Summon effect");
            }
        }
    }
    
    #[test]
    fn test_requires_target_position() {
        let handler = SaikaReinforcementsHandler::new();
        let mut request = create_test_request();
        let config = create_test_config();
        let state = AbilityState::default();
        
        // 有目標位置應該可以執行
        assert!(handler.check_target(&request, &config));
        
        // 沒有目標位置應該不能執行
        request.target_position = None;
        assert!(!handler.check_target(&request, &config));
    }
}