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
            ability_id: ability_id.to_string(),
            caster_id: format!("{:?}", input.caster),
            target_position: input.target_position,
            target_entity: input.target_entity.map(|e| format!("{:?}", e)),
        };

        // 處理請求
        match processor.process_request(&request) {
            Ok(effects) => {
                for effect in effects {
                    Self::apply_ability_effect(effect, input, skill_entity, tr, tw);
                }
                true
            }
            Err(_) => false,
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
        match effect.effect_type.as_str() {
            "damage" => {
                if let Some(target) = input.target_entity {
                    let damage_amount = effect.value.unwrap_or(0.0);
                    let caster_pos = tr.positions.get(input.caster)
                        .map(|p| p.0)
                        .unwrap_or_default();
                    
                    tw.outcomes.push(Outcome::Damage {
                        pos: caster_pos,
                        phys: damage_amount,
                        magi: 0.0,
                        real: 0.0,
                        source: input.caster,
                        target: target,
                    });
                }
            }
            "heal" => {
                if let Some(target) = input.target_entity {
                    let heal_amount = effect.value.unwrap_or(0.0);
                    let caster_pos = tr.positions.get(input.caster)
                        .map(|p| p.0)
                        .unwrap_or_default();
                    
                    tw.outcomes.push(Outcome::Heal {
                        pos: caster_pos,
                        target: target,
                        amount: heal_amount,
                    });
                }
            }
            "buff" | "debuff" => {
                // 創建技能效果
                let mut skill_effect = SkillEffect::new(
                    effect.ability_id.unwrap_or_default(),
                    input.caster,
                    SkillEffectType::Buff,
                    effect.duration.unwrap_or(10.0),
                );

                if let Some(value) = effect.value {
                    match effect.stat.as_deref() {
                        Some("damage") => skill_effect.data.damage_bonus = value / 100.0,
                        Some("range") => skill_effect.data.range_bonus = value,
                        Some("attack_speed") => skill_effect.data.attack_speed_bonus = value / 100.0,
                        Some("move_speed") => skill_effect.data.move_speed_bonus = value / 100.0,
                        _ => {}
                    }
                }

                let effect_entity = tr.entities.create();
                tw.skill_effects.insert(effect_entity, skill_effect);
            }
            _ => {
                warn!("未知的技能效果類型: {}", effect.effect_type);
            }
        }
    }
}