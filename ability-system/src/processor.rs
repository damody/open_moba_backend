use crate::types::*;
use crate::config::*;

/// 技能處理器介面
pub trait AbilityProcessor: Send + Sync {
    /// 執行技能
    fn execute(&self, config: &AbilityConfig, context: &mut AbilityContext) -> AbilityResult;
    
    /// 檢查技能是否可用
    fn can_execute(&self, config: &AbilityConfig, context: &AbilityContext) -> bool {
        // 預設實現：檢查基礎條件
        self.check_conditions(config, context) && 
        self.check_resources(config, context) &&
        self.check_cooldown(config, context)
    }
    
    /// 檢查條件
    fn check_conditions(&self, config: &AbilityConfig, context: &AbilityContext) -> bool {
        for condition in &config.conditions {
            if !self.evaluate_condition(condition, context) {
                return false;
            }
        }
        true
    }
    
    /// 檢查資源（法力、能量等）
    fn check_resources(&self, config: &AbilityConfig, context: &AbilityContext) -> bool {
        if let Some(level_data) = config.get_level_data(context.level) {
            // 這裡需要實作具體的資源檢查邏輯
            // 暫時返回 true
            true
        } else {
            false
        }
    }
    
    /// 檢查冷卻時間
    fn check_cooldown(&self, config: &AbilityConfig, _context: &AbilityContext) -> bool {
        // 這裡需要實作具體的冷卻檢查邏輯
        // 暫時返回 true
        true
    }
    
    /// 評估條件
    fn evaluate_condition(&self, condition: &Condition, context: &AbilityContext) -> bool {
        match &condition.condition_type {
            ConditionType::HasBuff(buff_name) => {
                // 檢查是否有指定的 buff
                // 需要通過 world_access 實現
                true // 暫時返回 true
            }
            ConditionType::HealthBelow(threshold) => {
                // 檢查血量是否低於閾值
                true // 暫時返回 true
            }
            ConditionType::HealthAbove(threshold) => {
                // 檢查血量是否高於閾值
                true // 暫時返回 true
            }
            ConditionType::InRange(range) => {
                // 檢查是否在範圍內
                if let (Some(target), Some(caster_pos)) = (
                    context.target,
                    context.world_access.get_position(context.caster)
                ) {
                    if let Some(target_pos) = context.world_access.get_position(target) {
                        let distance = (target_pos - caster_pos).magnitude();
                        distance <= *range
                    } else {
                        false
                    }
                } else {
                    true // 如果沒有目標，假設條件滿足
                }
            }
            ConditionType::HasMana(required_mana) => {
                // 檢查法力值
                true // 暫時返回 true
            }
            ConditionType::Custom(_) => {
                // 自定義條件需要子類實現
                true
            }
        }
    }
}

/// 預設的技能處理器實現
pub struct DefaultAbilityProcessor;

impl AbilityProcessor for DefaultAbilityProcessor {
    fn execute(&self, config: &AbilityConfig, context: &mut AbilityContext) -> AbilityResult {
        if !self.can_execute(config, context) {
            return AbilityResult::Failed("條件不滿足".to_string());
        }
        
        let mut effects = Vec::new();
        
        // 處理配置中定義的效果
        for effect in &config.effects {
            effects.push(self.process_effect(effect.clone(), config, context));
        }
        
        AbilityResult::Success(effects)
    }
}

impl DefaultAbilityProcessor {
    pub fn new() -> Self {
        Self
    }
    
    /// 處理單個效果
    fn process_effect(&self, mut effect: AbilityEffect, config: &AbilityConfig, context: &AbilityContext) -> AbilityEffect {
        // 根據等級和上下文調整效果數值
        if let Some(level_data) = config.get_level_data(context.level) {
            effect = self.scale_effect_with_level(effect, level_data, context);
        }
        
        effect
    }
    
    /// 根據等級縮放效果
    fn scale_effect_with_level(&self, mut effect: AbilityEffect, level_data: &AbilityLevelData, _context: &AbilityContext) -> AbilityEffect {
        match &mut effect {
            AbilityEffect::InstantDamage { damage, .. } => {
                if let Some(base_damage) = level_data.damage {
                    *damage = base_damage;
                }
            }
            AbilityEffect::Buff { duration, .. } => {
                if let Some(buff_duration) = level_data.duration {
                    *duration = buff_duration;
                }
            }
            AbilityEffect::AreaEffect { radius, duration, .. } => {
                if let Some(area_radius) = level_data.radius {
                    *radius = area_radius;
                }
                if let Some(area_duration) = level_data.duration {
                    *duration = area_duration;
                }
            }
            AbilityEffect::Summon { count, .. } => {
                if let Some(summon_count) = level_data.charges {
                    *count = summon_count;
                }
            }
            _ => {}
        }
        
        effect
    }
}

impl Default for DefaultAbilityProcessor {
    fn default() -> Self {
        Self::new()
    }
}

/// 狙擊模式處理器
pub struct SniperModeProcessor;

impl AbilityProcessor for SniperModeProcessor {
    fn execute(&self, config: &AbilityConfig, context: &mut AbilityContext) -> AbilityResult {
        if !self.can_execute(config, context) {
            return AbilityResult::Failed("狙擊模式無法啟動".to_string());
        }
        
        if let Some(level_data) = config.get_level_data(context.level) {
            // 創建變身效果
            let mut effects = std::collections::HashMap::new();
            effects.insert("range_bonus".to_string(), level_data.get_custom_value("range_bonus").unwrap_or(200.0));
            effects.insert("damage_bonus".to_string(), level_data.get_custom_value("damage_bonus").unwrap_or(0.25));
            effects.insert("attack_speed_penalty".to_string(), level_data.get_custom_value("attack_speed_penalty").unwrap_or(-0.3));
            effects.insert("move_speed_penalty".to_string(), level_data.get_custom_value("move_speed_penalty").unwrap_or(-0.5));
            
            let transform_effect = AbilityEffect::Transform {
                target: "self".to_string(),
                transform_id: "sniper_mode".to_string(),
                duration: None, // 無限持續，直到切換
            };
            
            AbilityResult::Success(vec![transform_effect])
        } else {
            AbilityResult::Failed("無效的技能等級".to_string())
        }
    }
}

/// 雜賀眾處理器
pub struct SaikaReinforcementsProcessor;

impl AbilityProcessor for SaikaReinforcementsProcessor {
    fn execute(&self, config: &AbilityConfig, context: &mut AbilityContext) -> AbilityResult {
        if !self.can_execute(config, context) {
            return AbilityResult::Failed("無法召喚雜賀眾".to_string());
        }
        
        if let Some(level_data) = config.get_level_data(context.level) {
            let summon_count = level_data.charges.unwrap_or(context.level);
            let position = context.target_position
                .or_else(|| context.world_access.get_position(context.caster))
                .unwrap_or_else(|| vek::Vec2::new(0.0, 0.0));
            
            let summon_effect = AbilityEffect::Summon {
                position,
                unit_type: "saika_gunner".to_string(),
                count: summon_count,
                duration: level_data.duration,
            };
            
            AbilityResult::Success(vec![summon_effect])
        } else {
            AbilityResult::Failed("無效的技能等級".to_string())
        }
    }
}