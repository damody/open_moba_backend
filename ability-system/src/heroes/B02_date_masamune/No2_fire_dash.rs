/// 踏火無痕 (fire_dash)
/// 
/// 伊達政宗的 E 技能 - 衝刺技能
/// 
/// 功能：
/// - 往前衝刺，衝刺時碰撞到敵方部隊造成傷害
/// - 每 0.1 秒給予敵人傷害
/// - 冷卻時間 30 秒
/// - 耗魔 80/100/120/140
/// - 施法距離 900/1000/1100/1200

use crate::handler::AbilityHandler;
use crate::*;

/// 踏火無痕處理器
pub struct FireDashHandler;

impl FireDashHandler {
    pub fn new() -> Self {
        Self
    }
}

impl AbilityHandler for FireDashHandler {
    fn get_ability_id(&self) -> &str {
        "fire_dash"
    }
    
    fn get_description(&self) -> &str {
        "踏火無痕 - 往前衝刺，衝刺時碰撞到敵方部隊每0.1秒給予敵人傷害"
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
            let damage_per_tick = self.get_custom_value(level_data, "damage_per_tick")
                .unwrap_or(50.0);
            
            // 獲取衝刺參數
            let dash_distance = level_data.range;
            let dash_duration = self.get_custom_value(level_data, "dash_duration")
                .unwrap_or(0.5); // 衝刺持續時間
            let tick_interval = 0.1; // 每 0.1 秒造成傷害
            let dash_width = 150.0; // 衝刺路徑寬度
            
            // 創建移動效果（瞬移或快速移動）
            effects.push(AbilityEffect::StatusModifier {
                target: request.caster,
                modifier_type: "dash_to_position".to_string(),
                value: 0.0, // 使用 target_position 作為目標
                duration: Some(dash_duration),
            });
            
            // 創建衝刺路徑上的持續傷害效果
            // 這裡簡化為一個區域效果，實際實作中可能需要路徑追蹤
            effects.push(AbilityEffect::AreaEffect {
                center: target_position,
                radius: dash_width / 2.0,
                effect_type: "fire_dash_trail".to_string(),
                damage: Some(damage_per_tick),
                duration: dash_duration,
            });
            
            // 添加移動速度加成效果
            if let Some(speed_bonus) = self.get_custom_value(level_data, "speed_bonus") {
                effects.push(AbilityEffect::StatusModifier {
                    target: request.caster,
                    modifier_type: "move_speed_multiplier".to_string(),
                    value: 1.0 + speed_bonus, // 增加移動速度
                    duration: Some(dash_duration),
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
        self.check_range(request, config, &AbilityLevelData::default())
    }
    
    fn check_target(&self, request: &AbilityRequest, config: &AbilityConfig) -> bool {
        match config.target_type {
            TargetType::Point => {
                // 衝刺技能需要目標位置
                request.target_position.is_some()
            },
            _ => true,
        }
    }
}

impl Default for FireDashHandler {
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
            ability_id: "fire_dash".to_string(),
            level: 1,
            target_position: Some(vek::Vec2::new(800.0, 400.0)),
            target_entity: None,
        }
    }
    
    fn create_test_config() -> AbilityConfig {
        AbilityConfig {
            name: "踏火無痕".to_string(),
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
        extra.insert("damage_per_tick".to_string(), serde_json::Value::from(50.0));
        extra.insert("dash_duration".to_string(), serde_json::Value::from(0.5));
        extra.insert("speed_bonus".to_string(), serde_json::Value::from(2.0));
        
        AbilityLevelData {
            cooldown: 30.0,
            mana_cost: 80.0,
            cast_time: 0.0,
            range: 900.0,
            extra,
        }
    }
    
    #[test]
    fn test_ability_id() {
        let handler = FireDashHandler::new();
        assert_eq!(handler.get_ability_id(), "fire_dash");
    }
    
    #[test]
    fn test_execute_creates_dash_effects() {
        let handler = FireDashHandler::new();
        let request = create_test_request();
        let config = create_test_config();
        let level_data = create_test_level_data();
        
        let effects = handler.execute(&request, &config, &level_data);
        
        // 應該生成 3 個效果：衝刺位移、路徑傷害、速度加成
        assert_eq!(effects.len(), 3);
        
        // 檢查衝刺位移效果
        let dash_effect = effects.iter().find(|e| {
            if let AbilityEffect::StatusModifier { modifier_type, .. } = e {
                modifier_type == "dash_to_position"
            } else {
                false
            }
        });
        assert!(dash_effect.is_some());
        
        // 檢查路徑傷害效果
        let trail_effect = effects.iter().find(|e| {
            if let AbilityEffect::AreaEffect { effect_type, .. } = e {
                effect_type == "fire_dash_trail"
            } else {
                false
            }
        });
        assert!(trail_effect.is_some());
        
        // 檢查速度加成效果
        let speed_effect = effects.iter().find(|e| {
            if let AbilityEffect::StatusModifier { modifier_type, .. } = e {
                modifier_type == "move_speed_multiplier"
            } else {
                false
            }
        });
        assert!(speed_effect.is_some());
    }
    
    #[test]
    fn test_requires_target_position() {
        let handler = FireDashHandler::new();
        let mut request = create_test_request();
        let config = create_test_config();
        
        // 有目標位置應該可以執行
        assert!(handler.check_target(&request, &config));
        
        // 沒有目標位置應該不能執行
        request.target_position = None;
        assert!(!handler.check_target(&request, &config));
    }
}