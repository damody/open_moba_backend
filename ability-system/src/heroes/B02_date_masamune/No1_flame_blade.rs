/// 火焰刀 (flame_blade)
/// 
/// 伊達政宗的 W 技能 - 前方攻擊技能
/// 
/// 功能：
/// - 往前方揮出一刀，造成大量傷害
/// - 施法距離 1100
/// - 冷卻時間 10 秒
/// - 耗魔 100/120/140/160

use crate::handler::AbilityHandler;
use crate::*;

/// 火焰刀處理器
pub struct FlameBladeHandler;

impl FlameBladeHandler {
    pub fn new() -> Self {
        Self
    }
}

impl AbilityHandler for FlameBladeHandler {
    fn get_ability_id(&self) -> &str {
        "flame_blade"
    }
    
    fn get_description(&self) -> &str {
        "火焰刀 - 往前方揮出一刀，造成大量傷害"
    }
    
    fn execute(
        &self, 
        request: &AbilityRequest, 
        _config: &AbilityConfig, 
        level_data: &AbilityLevelData
    ) -> Vec<AbilityEffect> {
        let mut effects = Vec::new();
        
        // 必須有目標位置或實體
        if let Some(target_position) = request.target_position {
            // 獲取傷害數值
            let damage = self.get_custom_value(level_data, "damage")
                .unwrap_or(200.0);
            
            // 獲取攻擊範圍（火焰刀的前方範圍）
            let range = level_data.range;
            let blade_width = 200.0; // 刀刃寬度
            
            // 創建前方錐形攻擊效果
            effects.push(AbilityEffect::AreaEffect {
                center: target_position,
                radius: blade_width / 2.0,
                effect_type: "flame_blade_slash".to_string(),
                damage: Some(damage),
                duration: 0.1, // 瞬間攻擊
            });
            
        } else if let Some(target_entity) = request.target_entity {
            // 直接對目標造成傷害
            let damage = self.get_custom_value(level_data, "damage")
                .unwrap_or(200.0);
            
            effects.push(AbilityEffect::Damage {
                target: target_entity,
                amount: damage,
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
        // 檢查基本條件
        self.check_cooldown(state) && 
        self.check_charges(state) &&
        self.check_target(request, config) &&
        self.check_mana(request, config, &AbilityLevelData::default()) &&
        self.check_range(request, config, &AbilityLevelData::default())
    }
    
    fn check_target(&self, request: &AbilityRequest, config: &AbilityConfig) -> bool {
        // 火焰刀可以對點或對目標使用
        match config.target_type {
            TargetType::Point => request.target_position.is_some(),
            TargetType::Unit => request.target_entity.is_some(),
            _ => true,
        }
    }
}

impl Default for FlameBladeHandler {
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
            ability_id: "flame_blade".to_string(),
            level: 1,
            target_position: Some(vek::Vec2::new(500.0, 300.0)),
            target_entity: None,
        }
    }
    
    fn create_test_config() -> AbilityConfig {
        AbilityConfig {
            name: "火焰刀".to_string(),
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
        extra.insert("damage".to_string(), serde_json::Value::from(200.0));
        
        AbilityLevelData {
            cooldown: 10.0,
            mana_cost: 100.0,
            cast_time: 0.0,
            range: 1100.0,
            extra,
        }
    }
    
    #[test]
    fn test_ability_id() {
        let handler = FlameBladeHandler::new();
        assert_eq!(handler.get_ability_id(), "flame_blade");
    }
    
    #[test]
    fn test_execute_area_attack() {
        let handler = FlameBladeHandler::new();
        let request = create_test_request();
        let config = create_test_config();
        let level_data = create_test_level_data();
        
        let effects = handler.execute(&request, &config, &level_data);
        
        // 應該生成一個區域效果
        assert_eq!(effects.len(), 1);
        
        if let AbilityEffect::AreaEffect { center, effect_type, damage, .. } = &effects[0] {
            assert_eq!(*center, vek::Vec2::new(500.0, 300.0));
            assert_eq!(effect_type, "flame_blade_slash");
            assert_eq!(damage.unwrap(), 200.0);
        } else {
            panic!("Expected AreaEffect");
        }
    }
    
    #[test]
    fn test_execute_single_target() {
        let handler = FlameBladeHandler::new();
        let mut request = create_test_request();
        let config = create_test_config();
        let level_data = create_test_level_data();
        
        // 改為單體目標
        request.target_position = None;
        request.target_entity = Some(Entity::from_raw(2));
        
        let effects = handler.execute(&request, &config, &level_data);
        
        // 應該生成一個傷害效果
        assert_eq!(effects.len(), 1);
        
        if let AbilityEffect::Damage { target, amount } = &effects[0] {
            assert_eq!(*target, Entity::from_raw(2));
            assert_eq!(*amount, 200.0);
        } else {
            panic!("Expected Damage effect");
        }
    }
}