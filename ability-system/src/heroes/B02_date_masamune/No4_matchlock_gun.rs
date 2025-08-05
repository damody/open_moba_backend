/// 火繩銃 (matchlock_gun)
/// 
/// 伊達政宗的 T 技能 - 變身技能
/// 
/// 功能：
/// - 將武器轉換成火繩銃
/// - 增加攻擊距離 700/800/900
/// - 增加基礎攻擊力 90/130/170 點
/// - 87% 機會使敵人暈眩 0.1/0.2/0.3 秒
/// - 持續 45 秒
/// - 冷卻時間 110 秒
/// - 耗魔 260/310/360

use crate::handler::AbilityHandler;
use crate::*;

/// 火繩銃處理器
pub struct MatchlockGunHandler;

impl MatchlockGunHandler {
    pub fn new() -> Self {
        Self
    }
}

impl AbilityHandler for MatchlockGunHandler {
    fn get_ability_id(&self) -> &str {
        "matchlock_gun"
    }
    
    fn get_description(&self) -> &str {
        "火繩銃 - 將武器轉換成火繩銃，增加攻擊距離和傷害，攻擊時有機會暈眩敵人"
    }
    
    fn execute(
        &self, 
        request: &AbilityRequest, 
        _config: &AbilityConfig, 
        level_data: &AbilityLevelData
    ) -> Vec<AbilityEffect> {
        let mut effects = Vec::new();
        
        // 獲取變身持續時間
        let duration = self.get_custom_value(level_data, "duration")
            .unwrap_or(45.0);
        
        // 獲取攻擊距離加成
        if let Some(range_bonus) = self.get_custom_value(level_data, "range_bonus") {
            effects.push(AbilityEffect::StatusModifier {
                target: request.caster,
                modifier_type: "attack_range_bonus".to_string(),
                value: range_bonus,
                duration: Some(duration),
            });
        }
        
        // 獲取攻擊力加成
        if let Some(damage_bonus) = self.get_custom_value(level_data, "damage_bonus") {
            effects.push(AbilityEffect::StatusModifier {
                target: request.caster,
                modifier_type: "base_damage_bonus".to_string(),
                value: damage_bonus,
                duration: Some(duration),
            });
        }
        
        // 獲取暈眩機會和時間
        let stun_chance = self.get_custom_value(level_data, "stun_chance")
            .unwrap_or(0.87); // 87%
        let stun_duration = self.get_custom_value(level_data, "stun_duration")
            .unwrap_or(0.1);
        
        // 添加攻擊時暈眩效果
        effects.push(AbilityEffect::StatusModifier {
            target: request.caster,
            modifier_type: "attack_stun_chance".to_string(),
            value: stun_chance,
            duration: Some(duration),
        });
        
        effects.push(AbilityEffect::StatusModifier {
            target: request.caster,
            modifier_type: "attack_stun_duration".to_string(),
            value: stun_duration,
            duration: Some(duration),
        });
        
        // 添加武器變身標記
        effects.push(AbilityEffect::StatusModifier {
            target: request.caster,
            modifier_type: "weapon_transform".to_string(),
            value: 1.0, // 1 = 火繩銃模式
            duration: Some(duration),
        });
        
        // 可以添加視覺/音效效果
        if let Some(transformation_effect) = level_data.extra.get("transformation_effect") {
            if let Some(effect_name) = transformation_effect.as_str() {
                effects.push(AbilityEffect::StatusModifier {
                    target: request.caster,
                    modifier_type: "visual_effect".to_string(),
                    value: 0.0, // 使用字串效果名稱
                    duration: Some(0.5), // 變身特效持續時間
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
        // 變身技能不需要檢查射程
        !state.is_casting
    }
    
    fn check_target(&self, _request: &AbilityRequest, config: &AbilityConfig) -> bool {
        // 變身技能不需要目標
        match config.target_type {
            TargetType::None => true,
            _ => true, // 允許其他目標類型以增加靈活性
        }
    }
    
    fn check_mana(&self, _request: &AbilityRequest, _config: &AbilityConfig, level_data: &AbilityLevelData) -> bool {
        // 火繩銃是終極變身技能，消耗大量法力值
        level_data.mana_cost >= 250.0 && level_data.mana_cost <= 400.0
    }
}

impl Default for MatchlockGunHandler {
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
            ability_id: "matchlock_gun".to_string(),
            level: 2,
            target_position: None,
            target_entity: None,
        }
    }
    
    fn create_test_config() -> AbilityConfig {
        AbilityConfig {
            name: "火繩銃".to_string(),
            description: "測試".to_string(),
            ability_type: AbilityType::Active,
            target_type: TargetType::None,
            cast_type: CastType::Instant,
            levels: HashMap::new(),
            properties: HashMap::new(),
        }
    }
    
    fn create_test_level_data() -> AbilityLevelData {
        let mut extra = HashMap::new();
        extra.insert("duration".to_string(), serde_json::Value::from(45.0));
        extra.insert("range_bonus".to_string(), serde_json::Value::from(800.0));
        extra.insert("damage_bonus".to_string(), serde_json::Value::from(130.0));
        extra.insert("stun_chance".to_string(), serde_json::Value::from(0.87));
        extra.insert("stun_duration".to_string(), serde_json::Value::from(0.2));
        extra.insert("transformation_effect".to_string(), serde_json::Value::from("matchlock_transform"));
        
        AbilityLevelData {
            cooldown: 110.0,
            mana_cost: 310.0,
            cast_time: 0.0,
            range: 800.0,
            extra,
        }
    }
    
    #[test]
    fn test_ability_id() {
        let handler = MatchlockGunHandler::new();
        assert_eq!(handler.get_ability_id(), "matchlock_gun");
    }
    
    #[test]
    fn test_execute_creates_transformation_effects() {
        let handler = MatchlockGunHandler::new();
        let request = create_test_request();
        let config = create_test_config();
        let level_data = create_test_level_data();
        
        let effects = handler.execute(&request, &config, &level_data);
        
        // 應該生成 6 個效果：射程加成、傷害加成、暈眩機會、暈眩時間、變身標記、視覺效果
        assert_eq!(effects.len(), 6);
        
        // 檢查射程加成
        let range_effect = effects.iter().find(|e| {
            if let AbilityEffect::StatusModifier { modifier_type, value, duration } = e {
                modifier_type == "attack_range_bonus" && *value == 800.0 && *duration == Some(45.0)
            } else {
                false
            }
        });
        assert!(range_effect.is_some());
        
        // 檢查傷害加成
        let damage_effect = effects.iter().find(|e| {
            if let AbilityEffect::StatusModifier { modifier_type, value, .. } = e {
                modifier_type == "base_damage_bonus" && *value == 130.0
            } else {
                false
            }
        });
        assert!(damage_effect.is_some());
        
        // 檢查暈眩機會
        let stun_chance_effect = effects.iter().find(|e| {
            if let AbilityEffect::StatusModifier { modifier_type, value, .. } = e {
                modifier_type == "attack_stun_chance" && *value == 0.87
            } else {
                false
            }
        });
        assert!(stun_chance_effect.is_some());
        
        // 檢查變身標記
        let transform_effect = effects.iter().find(|e| {
            if let AbilityEffect::StatusModifier { modifier_type, value, .. } = e {
                modifier_type == "weapon_transform" && *value == 1.0
            } else {
                false
            }
        });
        assert!(transform_effect.is_some());
    }
    
    #[test]
    fn test_no_target_required() {
        let handler = MatchlockGunHandler::new();
        let request = create_test_request();
        let config = create_test_config();
        
        // 變身技能不需要目標
        assert!(handler.check_target(&request, &config));
    }
    
    #[test]
    fn test_mana_cost_validation() {
        let handler = MatchlockGunHandler::new();
        let request = create_test_request();
        let config = create_test_config();
        let mut level_data = create_test_level_data();
        
        // 正常法力值成本
        level_data.mana_cost = 300.0;
        assert!(handler.check_mana(&request, &config, &level_data));
        
        // 法力值成本太低
        level_data.mana_cost = 100.0;
        assert!(!handler.check_mana(&request, &config, &level_data));
        
        // 法力值成本太高
        level_data.mana_cost = 500.0;
        assert!(!handler.check_mana(&request, &config, &level_data));
    }
}