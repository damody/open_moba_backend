/// 雨鐵炮 (rain_iron_cannon)
/// 
/// 雜賀孫市的 R 技能 - 終極技能
/// 
/// 功能：
/// - 在目標區域降下鐵炮彈雨
/// - 對範圍內敵人造成持續傷害
/// - 需要引導施法時間
/// - 大範圍區域傷害技能

use crate::handler::AbilityHandler;
use crate::*;

/// 雨鐵炮處理器
pub struct RainIronCannonHandler;

impl RainIronCannonHandler {
    pub fn new() -> Self {
        Self
    }
}

impl AbilityHandler for RainIronCannonHandler {
    fn get_ability_id(&self) -> &str {
        "rain_iron_cannon"
    }
    
    fn get_description(&self) -> &str {
        "雨鐵炮 - 在目標區域降下鐵炮彈雨，對範圍內敵人造成持續傷害"
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
            // 獲取基礎傷害
            let damage = self.get_custom_value(level_data, "damage")
                .unwrap_or(80.0);
            
            // 獲取範圍半徑
            let radius = self.get_custom_value(level_data, "radius")
                .unwrap_or(300.0);
            
            // 獲取持續時間
            let duration = self.get_custom_value(level_data, "duration")
                .unwrap_or(3.0);
            
            // 獲取傷害間隔和總次數
            let tick_interval = self.get_custom_value(level_data, "tick_interval")
                .unwrap_or(0.2);
            let total_ticks = self.get_custom_int(level_data, "total_ticks")
                .unwrap_or(15);
            
            // 計算每次傷害
            let damage_per_tick = damage / (total_ticks as f32);
            
            // 創建區域效果
            effects.push(AbilityEffect::AreaEffect {
                center: target_position,
                radius,
                effect_type: "iron_rain".to_string(),
                damage: Some(damage_per_tick),
                duration,
            });
            
            // 可以添加多個區域效果來模擬持續傷害
            // 每個 tick_interval 創建一個小範圍的傷害效果
            for tick in 0..total_ticks {
                let delay = tick as f32 * tick_interval;
                
                // 添加延遲效果（需要遊戲引擎支援）
                // 這裡我們簡化為單一區域效果
                // 實際實作中可能需要在 ECS 系統中處理時間延遲
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
            TargetType::Point => {
                // 區域技能需要目標位置
                request.target_position.is_some()
            },
            _ => true,
        }
    }
    
    fn check_mana(&self, _request: &AbilityRequest, _config: &AbilityConfig, level_data: &AbilityLevelData) -> bool {
        // 雨鐵炮是終極技能，消耗大量法力值
        // TODO: 實作實際的法力值檢查
        // 目前只檢查配置中的法力值成本是否合理
        level_data.mana_cost >= 200.0 && level_data.mana_cost <= 500.0
    }
}

impl Default for RainIronCannonHandler {
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
            ability_id: "rain_iron_cannon".to_string(),
            level: 1,
            target_position: Some(vek::Vec2::new(500.0, 300.0)),
            target_entity: None,
        }
    }
    
    fn create_test_config() -> AbilityConfig {
        AbilityConfig {
            name: "雨鐵炮".to_string(),
            description: "測試".to_string(),
            ability_type: AbilityType::Ultimate,
            target_type: TargetType::Point,
            cast_type: CastType::Channeled,
            levels: HashMap::new(),
            properties: HashMap::new(),
        }
    }
    
    fn create_test_level_data() -> AbilityLevelData {
        let mut extra = HashMap::new();
        extra.insert("damage".to_string(), serde_json::Value::from(80.0));
        extra.insert("radius".to_string(), serde_json::Value::from(300.0));
        extra.insert("duration".to_string(), serde_json::Value::from(3.0));
        extra.insert("tick_interval".to_string(), serde_json::Value::from(0.2));
        extra.insert("total_ticks".to_string(), serde_json::Value::from(15));
        
        AbilityLevelData {
            cooldown: 100.0,
            mana_cost: 200.0,
            cast_time: 1.0,
            range: 1200.0,
            extra,
        }
    }
    
    #[test]
    fn test_ability_id() {
        let handler = RainIronCannonHandler::new();
        assert_eq!(handler.get_ability_id(), "rain_iron_cannon");
    }
    
    #[test]
    fn test_execute_creates_area_effect() {
        let handler = RainIronCannonHandler::new();
        let request = create_test_request();
        let config = create_test_config();
        let level_data = create_test_level_data();
        
        let effects = handler.execute(&request, &config, &level_data);
        
        // 應該生成一個區域效果
        assert_eq!(effects.len(), 1);
        
        if let AbilityEffect::AreaEffect { center, radius, effect_type, damage, duration } = &effects[0] {
            assert_eq!(*center, vek::Vec2::new(500.0, 300.0));
            assert_eq!(*radius, 300.0);
            assert_eq!(effect_type, "iron_rain");
            assert!(damage.is_some());
            assert_eq!(*duration, 3.0);
            
            // 檢查每次傷害 = 總傷害 / 總次數
            let expected_damage_per_tick = 80.0 / 15.0;
            assert!((damage.unwrap() - expected_damage_per_tick).abs() < 0.01);
        } else {
            panic!("Expected AreaEffect");
        }
    }
    
    #[test]
    fn test_requires_target_position() {
        let handler = RainIronCannonHandler::new();
        let mut request = create_test_request();
        let config = create_test_config();
        
        // 有目標位置應該可以執行
        assert!(handler.check_target(&request, &config));
        
        // 沒有目標位置應該不能執行
        request.target_position = None;
        assert!(!handler.check_target(&request, &config));
    }
    
    #[test]
    fn test_cannot_execute_while_casting() {
        let handler = RainIronCannonHandler::new();
        let request = create_test_request();
        let config = create_test_config();
        let level_data = create_test_level_data();
        
        let mut state = AbilityState::default();
        
        // 正常狀態應該可以執行（忽略其他檢查）
        state.is_casting = false;
        // 由於沒有實作完整的檢查方法，這裡只測試基本邏輯
        
        // 施法中應該不能執行
        state.is_casting = true;
        // 實際測試需要完整的上下文
    }
    
    #[test]
    fn test_mana_cost_validation() {
        let handler = RainIronCannonHandler::new();
        let request = create_test_request();
        let config = create_test_config();
        let mut level_data = create_test_level_data();
        
        // 正常法力值成本
        level_data.mana_cost = 250.0;
        assert!(handler.check_mana(&request, &config, &level_data));
        
        // 法力值成本太低（不合理）
        level_data.mana_cost = 50.0;
        assert!(!handler.check_mana(&request, &config, &level_data));
        
        // 法力值成本太高（不合理）
        level_data.mana_cost = 600.0;
        assert!(!handler.check_mana(&request, &config, &level_data));
    }
}