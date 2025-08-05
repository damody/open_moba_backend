use specs::{
    shred::{ResourceId, World}, Entities, Join, LazyUpdate, Read, ReadExpect, ReadStorage, SystemData,
    Write, WriteStorage, ParJoin, Entity, System,
};
use crate::comp::*;
use crate::json_preprocessor::JsonPreprocessor;
use specs::prelude::ParallelIterator;
use std::{
    time::{Duration, Instant},
    collections::HashMap,
    fs,
};
use ability_system::{AbilityProcessor, AbilityRequest, AbilityEffect};
use log::{info, warn, error};
use std::sync::{Once, Mutex};
use std::sync::Arc;

#[derive(SystemData)]
pub struct SkillRead<'a> {
    entities: Entities<'a>,
    time: Read<'a, Time>,
    dt: Read<'a, DeltaTime>,
    heroes: ReadStorage<'a, Hero>,
    units: ReadStorage<'a, Unit>,
    abilities: Read<'a, std::collections::BTreeMap<String, Ability>>,
    positions: ReadStorage<'a, Pos>,
    factions: ReadStorage<'a, Faction>,
    properties: ReadStorage<'a, CProperty>,
    attacks: ReadStorage<'a, TAttack>,
}

#[derive(SystemData)]
pub struct SkillWrite<'a> {
    outcomes: Write<'a, Vec<Outcome>>,
    skills: WriteStorage<'a, Skill>,
    skill_effects: WriteStorage<'a, SkillEffect>,
    skill_inputs: Write<'a, Vec<SkillInput>>,
    damage_instances: Write<'a, Vec<DamageInstance>>,
}

// 全局AbilityProcessor單例
static ABILITY_PROCESSOR_INIT: Once = Once::new();
static mut ABILITY_PROCESSOR: Option<Arc<Mutex<AbilityProcessor>>> = None;

fn get_ability_processor() -> Arc<Mutex<AbilityProcessor>> {
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

pub struct Sys;

impl Default for Sys {
    fn default() -> Self {
        Self
    }
}


impl<'a> crate::comp::ecs::System<'a> for Sys {
    type SystemData = (
        SkillRead<'a>,
        SkillWrite<'a>,
    );

    const NAME: &'static str = "skill";

    fn run(job: &mut crate::comp::ecs::Job<Self>, (tr, mut tw): Self::SystemData) {
        let time = tr.time.0;
        let dt = tr.dt.0;
        
        // 更新所有技能的冷卻時間和層數
        for (entity, skill) in (&tr.entities, &mut tw.skills).join() {
            skill.update(dt, time as f32);
        }
        
        // 處理技能輸入
        let skill_inputs: Vec<SkillInput> = tw.skill_inputs.drain(..).collect();
        for input in skill_inputs {
            process_skill_input(&input, &tr, &mut tw);
        }
        
        // 更新技能效果
        let mut expired_effects = Vec::new();
        let mut effects_to_tick = Vec::new();
        
        // 收集需要處理的效果
        for (entity, effect) in (&tr.entities, &mut tw.skill_effects).join() {
            effect.update(dt, time as f32);
            
            // 處理需要 tick 的效果
            if effect.should_tick(time as f32) {
                effects_to_tick.push((entity, effect.clone()));
            }
            
            // 標記過期的效果
            if effect.is_expired() {
                expired_effects.push((entity, effect.clone()));
            }
        }
        
        // 處理 tick 效果
        for (effect_entity, mut effect) in effects_to_tick {
            process_skill_effect_tick(effect_entity, &mut effect, &tr, &mut tw);
            if let Some(eff) = tw.skill_effects.get_mut(effect_entity) {
                eff.tick(time as f32);
            }
        }
        
        // 移除過期的技能效果
        for (effect_entity, effect) in expired_effects {
            remove_skill_effect(effect_entity, &effect, &tr, &mut tw);
        }
    }
}

/// 處理技能輸入
fn process_skill_input(
    input: &SkillInput,
    tr: &SkillRead,
    tw: &mut SkillWrite,
) {
    // 找到對應的技能
    let skill_entity = if let Some((skill_entity, _)) = (&tr.entities, &tw.skills)
        .join()
        .find(|(_, skill)| skill.owner == input.caster && skill.ability_id == input.skill_id)
    {
        skill_entity
    } else {
        return;
    };
    
    // 檢查技能是否可用
    let skill_ready = if let Some(skill) = tw.skills.get(skill_entity) {
        skill.is_ready()
    } else {
        return;
    };
    
    if !skill_ready {
        return;
    }
    
    // 獲取技能定義
    let ability_id = if let Some(skill) = tw.skills.get(skill_entity) {
        skill.ability_id.clone()
    } else {
        return;
    };
    
    let ability = if let Some(ability) = tr.abilities.get(&ability_id) {
        ability.clone()
    } else {
        return;
    };

    // 先嘗試使用ability-system處理
    let processor = get_ability_processor();
    if let Ok(mut processor_guard) = processor.lock() {
        if try_process_with_ability_system(&mut *processor_guard, input, skill_entity, &ability_id, tr, tw) {
            info!("技能 {} 使用ability-system處理", ability_id);
            return;
        }
    }
    
    // 回退到硬編碼邏輯
    match ability_id.as_str() {
        "sniper_mode" => {
            execute_sniper_mode(skill_entity, input, &ability, tr, tw);
        }
        "saika_reinforcements" => {
            execute_saika_reinforcements(skill_entity, input, &ability, tr, tw);
        }
        "rain_iron_cannon" => {
            execute_rain_iron_cannon(skill_entity, input, &ability, tr, tw);
        }
        "three_stage_technique" => {
            execute_three_stage_technique(skill_entity, input, &ability, tr, tw);
        }
        _ => {
            log::warn!("Unknown skill: {}", ability_id);
        }
    }
}

/// 執行狙擊模式 (W)
fn execute_sniper_mode(
    skill_entity: Entity,
    input: &SkillInput,
    ability: &Ability,
    tr: &SkillRead,
    tw: &mut SkillWrite,
) {
    let skill = if let Some(skill) = tw.skills.get_mut(skill_entity) {
        skill
    } else {
        return;
    };
    
    if skill.use_skill(0.0) { // 切換技能無冷卻
        let effect_type = SkillEffectType::Transform;
        let toggle_state = skill.toggle_state;
        let skill_level = skill.current_level;
        let ability_id = skill.ability_id.clone();
        
        // 移除舊的狙擊模式效果
        let old_effects: Vec<Entity> = (&tr.entities, &tw.skill_effects)
            .join()
            .filter(|(_, effect)| effect.caster == input.caster && effect.skill_id == ability_id)
            .map(|(entity, _)| entity)
            .collect();
        
        for effect_entity in old_effects {
            tw.skill_effects.remove(effect_entity);
        }
        
        if toggle_state {
            // 啟動狙擊模式
            let mut effect = SkillEffect::new(
                ability_id,
                input.caster,
                effect_type,
                f32::INFINITY, // 持續到關閉
            );
            
            // 狙擊模式效果：增加射程和傷害，降低攻速和移速
            effect.data.range_bonus = 200.0 + (skill_level - 1) as f32 * 50.0; // +200/250/300/350
            effect.data.damage_bonus = 0.25 + (skill_level - 1) as f32 * 0.05; // +25%/30%/35%/40%
            effect.data.attack_speed_bonus = -0.3; // -30% 攻速
            effect.data.move_speed_bonus = -0.5; // -50% 移速
            effect.data.accuracy_bonus = 0.1 + (skill_level - 1) as f32 * 0.05; // +10%/15%/20%/25% 命中
            
            let effect_entity = tr.entities.create();
            tw.skill_effects.insert(effect_entity, effect);
            
            log::info!("Sniper mode activated for hero");
        } else {
            log::info!("Sniper mode deactivated for hero");
        }
    }
}

/// 執行雜賀眾 (E) 
fn execute_saika_reinforcements(
    skill_entity: Entity,
    input: &SkillInput,
    ability: &Ability,
    tr: &SkillRead,
    tw: &mut SkillWrite,
) {
    let skill = if let Some(skill) = tw.skills.get_mut(skill_entity) {
        skill
    } else {
        return;
    };
    
    if skill.use_skill(ability.cooldown[skill.current_level as usize - 1]) {
        // 在目標位置召喚雜賀鐵炮兵
        let summon_count = skill.current_level;
        let target_pos = input.target_position.unwrap_or_else(|| {
            // 如果沒有指定位置，在施法者周圍召喚
            if let Some(caster_pos) = tr.positions.get(input.caster) {
                caster_pos.0
            } else {
                vek::Vec2::new(0.0, 0.0)
            }
        });
        
        for i in 0..summon_count {
            let angle = (i as f32 / summon_count as f32) * std::f32::consts::PI * 2.0;
            let offset = vek::Vec2::new(angle.cos(), angle.sin()) * 100.0;
            let summon_pos = target_pos + offset;
            
            // TODO: 創建召喚物實體
            // 這裡應該創建雜賀鐵炮兵單位
            log::info!("Summoned Saika gunner at ({:.1}, {:.1})", summon_pos.x, summon_pos.y);
        }
        
        log::info!("Saika reinforcements summoned ({} units)", summon_count);
    }
}

/// 執行雨鐵炮 (R)
fn execute_rain_iron_cannon(
    skill_entity: Entity,
    input: &SkillInput,
    ability: &Ability,
    tr: &SkillRead,
    tw: &mut SkillWrite,
) {
    let skill = if let Some(skill) = tw.skills.get_mut(skill_entity) {
        skill
    } else {
        return;
    };
    
    if skill.use_skill(ability.cooldown[skill.current_level as usize - 1]) {
        let skill_level = skill.current_level;
        let ability_id = skill.ability_id.clone();
        
        let target_pos = input.target_position.unwrap_or_else(|| {
            if let Some(caster_pos) = tr.positions.get(input.caster) {
                caster_pos.0
            } else {
                vek::Vec2::new(0.0, 0.0)
            }
        });
        
        // 創建範圍傷害效果
        let mut effect = SkillEffect::new(
            ability_id,
            input.caster,
            SkillEffectType::Area,
            3.0, // 持續3秒
        );
        
        let base_damage = 80.0 + (skill_level - 1) as f32 * 40.0; // 80/120/160/200
        effect.position = Some(target_pos);
        let radius = 300.0 + (skill_level - 1) as f32 * 50.0; // 300/350/400/450
        effect.radius = radius;
        effect.data.damage_per_second = base_damage / 3.0; // 分3秒造成傷害
        effect.tick_interval = 0.2; // 每0.2秒一次傷害
        
        let effect_entity = tr.entities.create();
        tw.skill_effects.insert(effect_entity, effect);
        
        log::info!("Rain Iron Cannon cast at ({:.1}, {:.1}), radius: {:.1}", 
                  target_pos.x, target_pos.y, radius);
    }
}

/// 執行三段擊 (T)
fn execute_three_stage_technique(
    skill_entity: Entity,
    input: &SkillInput,
    ability: &Ability,
    tr: &SkillRead,
    tw: &mut SkillWrite,
) {
    let skill = if let Some(skill) = tw.skills.get_mut(skill_entity) {
        skill
    } else {
        return;
    };
    
    if skill.use_skill(ability.cooldown[skill.current_level as usize - 1]) {
        // 立即進行三次攻擊
        let damage_per_shot = 50.0 + (skill.current_level - 1) as f32 * 25.0; // 50/75/100/125
        
        // TODO: 實現三段攻擊邏輯
        // 這裡應該創建三個快速連續的攻擊
        
        log::info!("Three Stage Technique executed, {} damage per shot", damage_per_shot);
    }
}

/// 處理技能效果的 tick
fn process_skill_effect_tick(
    effect_entity: Entity,
    effect: &mut SkillEffect,
    tr: &SkillRead,
    tw: &mut SkillWrite,
) {
    match effect.effect_type {
        SkillEffectType::Area => {
            // 處理地面範圍效果（如雨鐵炮）
            if let Some(pos) = effect.position {
                // TODO: 搜尋範圍內的敵人並造成傷害
                // 這裡需要使用 searcher 來找到範圍內的目標
                log::debug!("Area effect tick at ({:.1}, {:.1}), radius: {:.1}", 
                           pos.x, pos.y, effect.radius);
            }
        }
        SkillEffectType::Transform => {
            // 處理變身效果（如狙擊模式）
            if let Some(target_attack) = tr.attacks.get(effect.caster) {
                // TODO: 實現變身效果的狀態管理
                // 這些修改應該通過事件系統處理，而不是直接修改組件
            }
        }
        _ => {}
    }
}

/// 移除技能效果
fn remove_skill_effect(
    effect_entity: Entity,
    effect: &SkillEffect,
    tr: &SkillRead,
    tw: &mut SkillWrite,
) {
    match effect.effect_type {
        SkillEffectType::Transform => {
            // 移除變身效果的屬性修改
            log::info!("Removing transform effect for entity: {:?}", effect.caster);
        }
        _ => {}
    }
    
    // 移除效果實體
    tw.skill_effects.remove(effect_entity);
}

/// 嘗試使用ability-system處理技能
fn try_process_with_ability_system(
    processor: &mut AbilityProcessor,
    input: &SkillInput,
    skill_entity: Entity,
    ability_id: &str,
    tr: &SkillRead,
    tw: &mut SkillWrite,
) -> bool {
    // 將SkillInput轉換為AbilityRequest
    let ability_request = convert_skill_input_to_ability_request(input, skill_entity, tw);
    
    // 需要創建一個默認的AbilityState
    let default_state = ability_system::AbilityState::default();
    
    // 處理技能請求
    let result = processor.process_ability(&ability_request, &default_state);
    
    if result.success {
        // 將AbilityEffect轉換為SkillEffect並應用
        for effect in result.effects {
            apply_ability_effect_as_skill_effect(effect, tr, tw);
        }
        
        // 使用技能（更新冷卻等）
        if let Some(skill) = tw.skills.get_mut(skill_entity) {
            // 這裡可能需要從processor獲取冷卻時間
            skill.use_skill(30.0); // 暫時使用固定值
        }
        
        true
    } else {
        warn!("ability-system處理技能 {} 失敗: {}", ability_id, result.error_message.unwrap_or_default());
        false
    }
}

/// 將SkillInput轉換為AbilityRequest
fn convert_skill_input_to_ability_request(
    input: &SkillInput,
    skill_entity: Entity,
    tw: &mut SkillWrite,
) -> AbilityRequest {
    let level = if let Some(skill) = tw.skills.get(skill_entity) {
        skill.current_level as u8
    } else {
        1
    };
    
    AbilityRequest {
        caster: input.caster,
        ability_id: input.skill_id.clone(),
        level,
        target_position: input.target_position,
        target_entity: input.target_entity,
    }
}

/// 將AbilityEffect轉換並應用為SkillEffect
fn apply_ability_effect_as_skill_effect(
    effect: AbilityEffect,
    tr: &SkillRead,
    tw: &mut SkillWrite,
) {
    match effect {
        AbilityEffect::Damage { target, amount } => {
            // 創建傷害實例
            let damage_types = DamageTypes {
                physical: amount,
                magical: 0.0,
                pure: 0.0,
            };
            
            let damage_flags = DamageFlags {
                can_crit: true,
                can_dodge: true,
                ignore_armor: false,
                ignore_magic_resist: false,
                lifesteal: 0.0,
                spell_vamp: 0.0,
            };
            
            tw.damage_instances.push(DamageInstance {
                target,
                damage_types,
                is_critical: false,
                is_dodged: false,
                damage_flags,
                source: DamageSource {
                    source_entity: target, // 暫時使用target作為source
                    source_type: DamageSourceType::Ability,
                    ability_id: Some("ability_system".to_string()),
                },
            });
        }
        AbilityEffect::Heal { target, amount } => {
            // 生成治療事件
            let target_pos = tr.positions.get(target)
                .map(|pos| pos.0)
                .unwrap_or(vek::Vec2::new(0.0, 0.0));
            
            tw.outcomes.push(Outcome::Heal {
                pos: target_pos,
                target,
                amount,
            });
            info!("為實體 {:?} 生成治療事件 {} 點生命值", target, amount);
        }
        AbilityEffect::StatusModifier { target, modifier_type, value, duration } => {
            // 創建狀態修改效果
            let mut skill_effect = SkillEffect::new(
                format!("ability_modifier_{}", modifier_type),
                target,
                SkillEffectType::Buff,
                duration.unwrap_or(f32::INFINITY),
            );
            
            // 根據modifier_type設置對應的數值
            match modifier_type.as_str() {
                "damage_bonus" => skill_effect.data.damage_bonus = value,
                "range_bonus" => skill_effect.data.range_bonus = value,
                "attack_speed_bonus" => skill_effect.data.attack_speed_bonus = value,
                "move_speed_bonus" => skill_effect.data.move_speed_bonus = value,
                _ => warn!("未知的狀態修改類型: {}", modifier_type),
            }
            
            let effect_entity = tr.entities.create();
            tw.skill_effects.insert(effect_entity, skill_effect);
        }
        AbilityEffect::AreaEffect { center, radius, damage, .. } => {
            // 創建區域效果
            let mut skill_effect = SkillEffect::new(
                "ability_area_effect".to_string(),
                tr.entities.create(), // 創建新實體作為施法者
                SkillEffectType::Area,
                3.0, // 默認持續3秒
            );
            
            skill_effect.position = Some(center);
            skill_effect.radius = radius;
            if let Some(dmg) = damage {
                skill_effect.data.damage_per_second = dmg / 3.0; // 分3秒造成傷害
            }
            skill_effect.tick_interval = 0.2;
            
            let effect_entity = tr.entities.create();
            tw.skill_effects.insert(effect_entity, skill_effect);
        }
        AbilityEffect::Summon { position, unit_type, count, .. } => {
            // 召喚效果處理
            info!("在位置 ({:.1}, {:.1}) 召喚 {} 個 {} 單位", 
                  position.x, position.y, count, unit_type);
            // TODO: 實際的召喚邏輯
        }
    }
}