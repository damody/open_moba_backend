/// 狙擊模式 (sniper_mode)
/// 
/// 雜賀孫市的 W 技能 - 切換技能
/// 
/// 功能：
/// - 切換到狙擊模式，大幅增加射程和傷害
/// - 降低攻擊速度和移動速度
/// - 無冷卻時間，可隨時切換
/// - 增加命中率和精確度

use crate::handler::AbilityHandler;
use crate::*;

/// 狙擊模式處理器
pub struct SniperModeHandler;

impl SniperModeHandler {
    pub fn new() -> Self {
        Self
    }
}

impl AbilityHandler for SniperModeHandler {
    fn get_ability_id(&self) -> &str {
        "sniper_mode"
    }
    
    fn get_description(&self) -> &str {
        "狙擊模式 - 切換到狙擊模式，增加射程和傷害，但降低攻擊速度和移動速度"
    }
    
    fn execute(
        &self, 
        request: &AbilityRequest, 
        _config: &AbilityConfig, 
        level_data: &AbilityLevelData
    ) -> Vec<AbilityEffect> {
        let mut effects = Vec::new();
        
        // 射程加成
        if let Some(range_bonus) = self.get_custom_value(level_data, "range_bonus") {
            effects.push(AbilityEffect::StatusModifier {
                target: request.caster,
                modifier_type: "range_bonus".to_string(),
                value: range_bonus,
                duration: None, // 切換技能持續到下次切換
            });
        }
        
        // 傷害加成
        if let Some(damage_bonus) = self.get_custom_value(level_data, "damage_bonus") {
            effects.push(AbilityEffect::StatusModifier {
                target: request.caster,
                modifier_type: "damage_bonus".to_string(),
                value: damage_bonus,
                duration: None,
            });
        }
        
        // 攻擊速度懲罰
        if let Some(attack_speed_penalty) = self.get_custom_value(level_data, "attack_speed_penalty") {
            effects.push(AbilityEffect::StatusModifier {
                target: request.caster,
                modifier_type: "attack_speed_multiplier".to_string(),
                value: 1.0 + attack_speed_penalty, // -0.3 變成 0.7 倍速
                duration: None,
            });
        }
        
        // 移動速度懲罰
        if let Some(move_speed_penalty) = self.get_custom_value(level_data, "move_speed_penalty") {
            effects.push(AbilityEffect::StatusModifier {
                target: request.caster,
                modifier_type: "move_speed_multiplier".to_string(),
                value: 1.0 + move_speed_penalty, // -0.5 變成 0.5 倍速
                duration: None,
            });
        }
        
        // 命中率加成
        if let Some(accuracy_bonus) = self.get_custom_value(level_data, "accuracy_bonus") {
            effects.push(AbilityEffect::StatusModifier {
                target: request.caster,
                modifier_type: "accuracy_bonus".to_string(),
                value: accuracy_bonus,
                duration: None,
            });
        }
        
        effects
    }
    
    fn can_execute(
        &self, 
        request: &AbilityRequest, 
        config: &AbilityConfig, 
        state: &AbilityState
    ) -> bool {
        // 狙擊模式是切換技能，不需要檢查冷卻時間和充能
        // 只需要檢查目標類型
        self.check_target(request, config)
    }
    
    fn check_cooldown(&self, _state: &AbilityState) -> bool {
        // 切換技能無冷卻時間
        true
    }
    
    fn check_charges(&self, _state: &AbilityState) -> bool {
        // 切換技能無充能限制
        true
    }
    
    fn check_mana(&self, _request: &AbilityRequest, _config: &AbilityConfig, _level_data: &AbilityLevelData) -> bool {
        // 狙擊模式不消耗法力值
        true
    }
}

impl Default for SniperModeHandler {
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
            caster: Entity::from_raw(1), // 測試用實體
            ability_id: "sniper_mode".to_string(),
            level: 1,
            target_position: None,
            target_entity: None,
        }
    }
    
    fn create_test_config() -> AbilityConfig {
        AbilityConfig {
            name: "狙擊模式".to_string(),
            description: "測試".to_string(),
            ability_type: AbilityType::Toggle,
            target_type: TargetType::None,
            cast_type: CastType::Instant,
            levels: HashMap::new(),
            properties: HashMap::new(),
        }
    }
    
    fn create_test_level_data() -> AbilityLevelData {
        let mut extra = HashMap::new();
        extra.insert("range_bonus".to_string(), serde_json::Value::from(200.0));
        extra.insert("damage_bonus".to_string(), serde_json::Value::from(0.25));
        extra.insert("attack_speed_penalty".to_string(), serde_json::Value::from(-0.3));
        extra.insert("move_speed_penalty".to_string(), serde_json::Value::from(-0.5));
        extra.insert("accuracy_bonus".to_string(), serde_json::Value::from(0.1));
        
        AbilityLevelData {
            cooldown: 0.0,
            mana_cost: 0.0,
            cast_time: 0.0,
            range: 0.0,
            extra,
        }
    }
    
    #[test]
    fn test_ability_id() {
        let handler = SniperModeHandler::new();
        assert_eq!(handler.get_ability_id(), "sniper_mode");
    }
    
    #[test]
    fn test_execute_generates_effects() {
        let handler = SniperModeHandler::new();
        let request = create_test_request();
        let config = create_test_config();
        let level_data = create_test_level_data();
        
        let effects = handler.execute(&request, &config, &level_data);
        
        // 應該生成 5 個狀態修改效果
        assert_eq!(effects.len(), 5);
        
        // 檢查是否包含射程加成
        let range_effect = effects.iter().find(|e| {
            if let AbilityEffect::StatusModifier { modifier_type, .. } = e {
                modifier_type == "range_bonus"
            } else {
                false
            }
        });
        assert!(range_effect.is_some());
    }
    
    #[test]
    fn test_can_execute_toggle_skill() {
        let handler = SniperModeHandler::new();
        let request = create_test_request();
        let config = create_test_config();
        let state = AbilityState::default();
        
        // 切換技能應該總是可以執行
        assert!(handler.can_execute(&request, &config, &state));
    }
}