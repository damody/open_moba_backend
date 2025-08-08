use ability_system::{AbilityProcessor, AbilityRequest, AbilityEffect};
use log::{info, warn, error};
use std::sync::{Once, Mutex, Arc};
use std::fs;

use crate::json_preprocessor::JsonPreprocessor;
use super::{SkillRead, SkillWrite};
use crate::comp::*;

/// 技能處理器管理
pub struct SkillProcessor;

// 全局AbilityProcessor單例
static ABILITY_PROCESSOR_INIT: Once = Once::new();
static mut ABILITY_PROCESSOR: Option<Arc<Mutex<AbilityProcessor>>> = None;

impl SkillProcessor {
    /// 獲取全局技能處理器
    pub fn get_ability_processor() -> Arc<Mutex<AbilityProcessor>> {
        unsafe {
            ABILITY_PROCESSOR_INIT.call_once(|| {
                let mut processor = AbilityProcessor::new();
                
                // 載入技能配置文件
                let config_path = "ability-configs/sniper_abilities.json";
                if let Ok(content) = fs::read_to_string(config_path) {
                    // 使用JsonPreprocessor處理註解
                    let processed_content = JsonPreprocessor::remove_comments(&content);
                    if let Err(e) = processor.load_from_json(&processed_content) {
                        error!("載入技能配置失敗: {}", e);
                    } else {
                        info!("成功載入技能配置: {}", config_path);
                    }
                } else {
                    warn!("無法讀取技能配置文件: {}，僅使用硬編碼技能", config_path);
                }
                
                ABILITY_PROCESSOR = Some(Arc::new(Mutex::new(processor)));
            });
            
            ABILITY_PROCESSOR.as_ref().unwrap().clone()
        }
    }

    /// 嘗試使用ability-system處理技能
    pub fn try_process_with_ability_system(
        processor: &mut AbilityProcessor,
        input: &SkillInput,
        skill_entity: specs::Entity,
        ability_id: &str,
        tr: &SkillRead,
        tw: &mut SkillWrite,
    ) -> bool {
        // 創建請求
        let request = AbilityRequest {
            caster: input.caster,
            ability_id: ability_id.to_string(),
            level: 1, // 預設等級為1
            target_position: input.target_position,
            target_entity: input.target_entity,
        };

        // 创建當前狀態（預設）
        let current_state = ability_system::AbilityState::default();
        
        // 處理請求
        let result = processor.process_ability(&request, &current_state);
        
        if result.success {
            for effect in result.effects {
                Self::apply_ability_effect(effect, input, skill_entity, tr, tw);
            }
            true
        } else {
            if let Some(err) = result.error_message {
                warn!("技能執行失敗: {}", err);
            }
            false
        }
    }

    /// 應用技能效果
    fn apply_ability_effect(
        effect: AbilityEffect,
        input: &SkillInput,
        skill_entity: specs::Entity,
        tr: &SkillRead,
        tw: &mut SkillWrite,
    ) {
        use ability_system::AbilityEffect::*;
        
        match effect {
            Damage { target, amount } => {
                let caster_pos = tr.positions.get(input.caster)
                    .map(|p| p.0)
                    .unwrap_or_default();
                
                tw.outcomes.push(Outcome::Damage {
                    pos: caster_pos,
                    phys: amount,
                    magi: 0.0,
                    real: 0.0,
                    source: input.caster,
                    target,
                });
            }
            Heal { target, amount } => {
                let caster_pos = tr.positions.get(input.caster)
                    .map(|p| p.0)
                    .unwrap_or_default();
                
                tw.outcomes.push(Outcome::Heal {
                    pos: caster_pos,
                    target,
                    amount,
                });
            }
            StatusModifier { target, modifier_type, value, duration } => {
                // 創建技能效果
                let mut skill_effect = SkillEffect::new(
                    input.skill_id.clone(),
                    input.caster,
                    if value > 0.0 { SkillEffectType::Buff } else { SkillEffectType::Debuff },
                    duration.unwrap_or(10.0),
                );
                skill_effect.target = Some(target);

                match modifier_type.as_str() {
                    "damage" => skill_effect.data.damage_bonus = value / 100.0,
                    "range" => skill_effect.data.range_bonus = value,
                    "attack_speed" => skill_effect.data.attack_speed_bonus = value / 100.0,
                    "move_speed" => skill_effect.data.move_speed_bonus = value / 100.0,
                    _ => {}
                }

                let effect_entity = tr.entities.create();
                tw.skill_effects.insert(effect_entity, skill_effect);
            }
            Summon { position, unit_type, count, duration } => {
                // 召喚效果的處理
                info!("召喚 {} 個 {} 在位置 {:?}", count, unit_type, position);
            }
            AreaEffect { center, radius, effect_type, damage, duration } => {
                // 區域效果
                let mut skill_effect = SkillEffect::new(
                    input.skill_id.clone(),
                    input.caster,
                    SkillEffectType::Area,
                    duration,
                );
                skill_effect.area_center = Some(center);
                skill_effect.radius = radius;
                
                if let Some(dmg) = damage {
                    skill_effect.data.damage_per_tick = dmg;
                    skill_effect.data.affects_enemies = true;
                }
                
                let effect_entity = tr.entities.create();
                tw.skill_effects.insert(effect_entity, skill_effect);
            }
        }
    }
}