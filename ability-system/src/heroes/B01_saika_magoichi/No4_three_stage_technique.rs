/// 三段擊 (three_stage_technique)
/// 
/// 雜賀孫市的 T 技能 - 攻擊技能
/// 
/// 功能：
/// - 快速進行三次連續攻擊
/// - 每次攻擊造成額外傷害
/// - 需要單體目標
/// - 短冷卻時間可頻繁使用

use crate::handler::AbilityHandler;
use crate::*;

/// 三段擊處理器
pub struct ThreeStageHandler;

impl ThreeStageHandler {
    pub fn new() -> Self {
        Self
    }
}

impl AbilityHandler for ThreeStageHandler {
    fn get_ability_id(&self) -> &str {
        "three_stage_technique"
    }
    
    fn get_description(&self) -> &str {
        "三段擊 - 快速進行三次連續攻擊，每次攻擊造成額外傷害"
    }
    
    fn execute(
        &self, 
        request: &AbilityRequest, 
        _config: &AbilityConfig, 
        level_data: &AbilityLevelData
    ) -> Vec<AbilityEffect> {
        let mut effects = Vec::new();
        
        // 必須有目標實體
        if let Some(target_entity) = request.target_entity {
            // 獲取每次攻擊的傷害
            let damage_per_attack = self.get_custom_value(level_data, "damage_per_attack")
                .unwrap_or(50.0);
            
            // 獲取攻擊次數
            let attacks_count = self.get_custom_int(level_data, "attacks_count")
                .unwrap_or(3);
            
            // 獲取攻擊間隔
            let attack_interval = self.get_custom_value(level_data, "attack_interval")
                .unwrap_or(0.15);
            
            // 創建多次傷害效果
            for attack_index in 0..attacks_count {
                effects.push(AbilityEffect::Damage {
                    target: target_entity,
                    amount: damage_per_attack,
                });
                
                // 可以添加延遲效果來模擬攻擊間隔
                // 實際實作中可能需要在 ECS 系統中處理時間延遲
                // 這裡我們簡化為立即執行所有攻擊
            }
            
            // 可以添加額外效果，如降低目標護甲等
            if let Some(armor_reduction) = self.get_custom_value(level_data, "armor_reduction") {
                effects.push(AbilityEffect::StatusModifier {
                    target: target_entity,
                    modifier_type: "armor_reduction".to_string(),
                    value: -armor_reduction, // 負值表示減少護甲
                    duration: Some(3.0), // 持續3秒
                });
            }
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
        self.check_range(request, config, &AbilityLevelData::default()) &&
        // 額外檢查：不能在施法中
        !state.is_casting
    }
    
    fn check_target(&self, request: &AbilityRequest, config: &AbilityConfig) -> bool {
        match config.target_type {
            TargetType::Unit => {
                // 單體攻擊技能需要目標實體
                request.target_entity.is_some()
            },
            _ => true,
        }
    }
    
    fn check_range(&self, request: &AbilityRequest, config: &AbilityConfig, level_data: &AbilityLevelData) -> bool {
        // 三段擊使用普攻射程，range 配置為 0.0 表示使用普攻射程
        if level_data.range <= 0.0 {
            // 使用普攻射程，這裡總是返回 true
            // 實際檢查需要在 ECS 系統中進行
            return true;
        }
        
        // 如果有設定特定射程，使用預設檢查
        // TODO: 實作實際的射程檢查
        true
    }
    
    fn check_mana(&self, _request: &AbilityRequest, _config: &AbilityConfig, level_data: &AbilityLevelData) -> bool {
        // 三段擊消耗中等法力值
        // TODO: 實作實際的法力值檢查
        level_data.mana_cost >= 50.0 && level_data.mana_cost <= 150.0
    }
}

impl Default for ThreeStageHandler {
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
            ability_id: "three_stage_technique".to_string(),
            level: 2,
            target_position: None,
            target_entity: Some(Entity::from_raw(2)),
        }
    }
    
    fn create_test_config() -> AbilityConfig {
        AbilityConfig {
            name: "三段擊".to_string(),
            description: "測試".to_string(),
            ability_type: AbilityType::Active,
            target_type: TargetType::Unit,
            cast_type: CastType::Instant,
            levels: HashMap::new(),
            properties: HashMap::new(),
        }
    }
    
    fn create_test_level_data() -> AbilityLevelData {
        let mut extra = HashMap::new();
        extra.insert("damage_per_attack".to_string(), serde_json::Value::from(75.0));
        extra.insert("attacks_count".to_string(), serde_json::Value::from(3));
        extra.insert("attack_interval".to_string(), serde_json::Value::from(0.15));
        extra.insert("armor_reduction".to_string(), serde_json::Value::from(2.0));
        
        AbilityLevelData {
            cooldown: 11.0,
            mana_cost: 70.0,
            cast_time: 0.0,
            range: 0.0, // 使用普攻射程
            extra,
        }
    }
    
    #[test]
    fn test_ability_id() {
        let handler = ThreeStageHandler::new();
        assert_eq!(handler.get_ability_id(), "three_stage_technique");
    }
    
    #[test]
    fn test_execute_creates_multiple_damage_effects() {
        let handler = ThreeStageHandler::new();
        let request = create_test_request();
        let config = create_test_config();
        let level_data = create_test_level_data();
        
        let effects = handler.execute(&request, &config, &level_data);
        
        // 應該生成 3 個傷害效果 + 1 個狀態修改效果
        assert_eq!(effects.len(), 4);
        
        // 檢查前3個效果是傷害效果
        for i in 0..3 {
            if let AbilityEffect::Damage { target, amount } = &effects[i] {
                assert_eq!(*target, Entity::from_raw(2));
                assert_eq!(*amount, 75.0);
            } else {
                panic!("Expected Damage effect at index {}", i);
            }
        }
        
        // 檢查第4個效果是狀態修改效果
        if let AbilityEffect::StatusModifier { target, modifier_type, value, duration } = &effects[3] {
            assert_eq!(*target, Entity::from_raw(2));
            assert_eq!(modifier_type, "armor_reduction");
            assert_eq!(*value, -2.0); // 負值表示減少護甲
            assert_eq!(*duration, Some(3.0));
        } else {
            panic!("Expected StatusModifier effect at index 3");
        }
    }
    
    #[test]
    fn test_requires_target_entity() {
        let handler = ThreeStageHandler::new();
        let mut request = create_test_request();
        let config = create_test_config();
        
        // 有目標實體應該可以執行
        assert!(handler.check_target(&request, &config));
        
        // 沒有目標實體應該不能執行
        request.target_entity = None;
        assert!(!handler.check_target(&request, &config));
    }
    
    #[test]
    fn test_uses_attack_range() {
        let handler = ThreeStageHandler::new();
        let request = create_test_request();
        let config = create_test_config();
        let level_data = create_test_level_data(); // range = 0.0
        
        // range = 0.0 應該使用普攻射程，總是返回 true
        assert!(handler.check_range(&request, &config, &level_data));
    }
    
    #[test]
    fn test_mana_cost_validation() {
        let handler = ThreeStageHandler::new();
        let request = create_test_request();
        let config = create_test_config();
        let mut level_data = create_test_level_data();
        
        // 正常法力值成本
        level_data.mana_cost = 80.0;
        assert!(handler.check_mana(&request, &config, &level_data));
        
        // 法力值成本太低
        level_data.mana_cost = 30.0;
        assert!(!handler.check_mana(&request, &config, &level_data));
        
        // 法力值成本太高
        level_data.mana_cost = 200.0;
        assert!(!handler.check_mana(&request, &config, &level_data));
    }
    
    #[test]
    fn test_no_target_entity_no_effects() {
        let handler = ThreeStageHandler::new();
        let mut request = create_test_request();
        let config = create_test_config();
        let level_data = create_test_level_data();
        
        // 沒有目標實體時不應該生成任何效果
        request.target_entity = None;
        
        let effects = handler.execute(&request, &config, &level_data);
        assert_eq!(effects.len(), 0);
    }
}