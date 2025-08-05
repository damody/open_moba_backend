/// 火焰強襲 (flame_assault)
/// 
/// 伊達政宗的 R 技能 - 範圍暈眩技能
/// 
/// 功能：
/// - 施展一道巨型火焰，對範圍內敵人造成傷害並暈眩
/// - 範圍 500
/// - 傷害 200/300/400/500
/// - 暈眩時間 0.3/0.6/0.9/1.2 秒
/// - 冷卻時間 14 秒
/// - 耗魔 120/140/160/180

use crate::handler::AbilityHandler;
use crate::*;

/// 火焰強襲處理器
pub struct FlameAssaultHandler;

impl FlameAssaultHandler {
    pub fn new() -> Self {
        Self
    }
}

impl AbilityHandler for FlameAssaultHandler {
    fn get_ability_id(&self) -> &str {
        "flame_assault"
    }
    
    fn get_description(&self) -> &str {
        "火焰強襲 - 施展一道巨型火焰，對範圍內敵人造成傷害並暈眩"
    }
    
    fn execute(
        &self, 
        request: &AbilityRequest, 
        _config: &AbilityConfig, 
        level_data: &AbilityLevelData
    ) -> Vec<AbilityEffect> {
        let mut effects = Vec::new();
        
        // 必須有目標位置
        if let Some(target_position) = request.target_position {
            // 獲取傷害數值
            let damage = self.get_custom_value(level_data, "damage")
                .unwrap_or(200.0);
            
            // 獲取暈眩時間
            let stun_duration = self.get_custom_value(level_data, "stun_duration")
                .unwrap_or(0.3);
            
            // 獲取範圍
            let radius = self.get_custom_value(level_data, "radius")
                .unwrap_or(500.0);
            
            // 創建火焰區域傷害效果
            effects.push(AbilityEffect::AreaEffect {
                center: target_position,
                radius,
                effect_type: "flame_assault_explosion".to_string(),
                damage: Some(damage),
                duration: 0.2, // 爆炸持續時間
            });
            
            // 創建範圍暈眩效果
            // 注意：這裡簡化為區域效果，實際實作中可能需要對每個受影響的敵人
            // 分別施加暈眩狀態
            effects.push(AbilityEffect::AreaEffect {
                center: target_position,
                radius,
                effect_type: "flame_assault_stun".to_string(),
                damage: None, // 暈眩效果不造成傷害
                duration: stun_duration,
            });
            
            // 可以添加額外的視覺效果
            if let Some(screen_shake) = level_data.extra.get("screen_shake").and_then(|v| v.as_bool()) {
                if screen_shake {
                    effects.push(AbilityEffect::StatusModifier {
                        target: request.caster,
                        modifier_type: "screen_shake".to_string(),
                        value: 1.0,
                        duration: Some(0.5),
                    });
                }
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
        self.check_range(request, config, &AbilityLevelData::default())
    }
    
    fn check_target(&self, request: &AbilityRequest, config: &AbilityConfig) -> bool {
        match config.target_type {
            TargetType::Point => {
                // 區域技能需要目標位置
                request.target_position.is_some()
            },
            _ => true,
        }
    }
}

impl Default for FlameAssaultHandler {
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
            ability_id: "flame_assault".to_string(),
            level: 2,
            target_position: Some(vek::Vec2::new(600.0, 400.0)),
            target_entity: None,
        }
    }
    
    fn create_test_config() -> AbilityConfig {
        AbilityConfig {
            name: "火焰強襲".to_string(),
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
        extra.insert("damage".to_string(), serde_json::Value::from(300.0));
        extra.insert("stun_duration".to_string(), serde_json::Value::from(0.6));
        extra.insert("radius".to_string(), serde_json::Value::from(500.0));
        extra.insert("screen_shake".to_string(), serde_json::Value::from(true));
        
        AbilityLevelData {
            cooldown: 14.0,
            mana_cost: 140.0,
            cast_time: 0.0,
            range: 700.0,
            extra,
        }
    }
    
    #[test]
    fn test_ability_id() {
        let handler = FlameAssaultHandler::new();
        assert_eq!(handler.get_ability_id(), "flame_assault");
    }
    
    #[test]
    fn test_execute_creates_area_effects() {
        let handler = FlameAssaultHandler::new();
        let request = create_test_request();
        let config = create_test_config();
        let level_data = create_test_level_data();
        
        let effects = handler.execute(&request, &config, &level_data);
        
        // 應該生成 3 個效果：傷害爆炸、暈眩、螢幕震動
        assert_eq!(effects.len(), 3);
        
        // 檢查傷害效果
        let damage_effect = effects.iter().find(|e| {
            if let AbilityEffect::AreaEffect { effect_type, damage, .. } = e {
                effect_type == "flame_assault_explosion" && damage.is_some()
            } else {
                false
            }
        });
        assert!(damage_effect.is_some());
        
        if let Some(AbilityEffect::AreaEffect { center, radius, damage, .. }) = damage_effect {
            assert_eq!(*center, vek::Vec2::new(600.0, 400.0));
            assert_eq!(*radius, 500.0);
            assert_eq!(damage.unwrap(), 300.0);
        }
        
        // 檢查暈眩效果
        let stun_effect = effects.iter().find(|e| {
            if let AbilityEffect::AreaEffect { effect_type, damage, .. } = e {
                effect_type == "flame_assault_stun" && damage.is_none()
            } else {
                false
            }
        });
        assert!(stun_effect.is_some());
        
        // 檢查螢幕震動效果
        let shake_effect = effects.iter().find(|e| {
            if let AbilityEffect::StatusModifier { modifier_type, .. } = e {
                modifier_type == "screen_shake"
            } else {
                false
            }
        });
        assert!(shake_effect.is_some());
    }
    
    #[test]
    fn test_requires_target_position() {
        let handler = FlameAssaultHandler::new();
        let mut request = create_test_request();
        let config = create_test_config();
        
        // 有目標位置應該可以執行
        assert!(handler.check_target(&request, &config));
        
        // 沒有目標位置應該不能執行
        request.target_position = None;
        assert!(!handler.check_target(&request, &config));
    }
}