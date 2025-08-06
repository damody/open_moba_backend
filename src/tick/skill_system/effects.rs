use specs::{Entity, Join};
use crate::comp::*;
use super::{SkillRead, SkillWrite};

/// 技能效果管理器
pub struct EffectManager;

impl EffectManager {
    /// 更新所有技能效果
    pub fn update_effects(tr: &SkillRead, tw: &mut SkillWrite) {
        let time = tr.time.0 as f32;
        let dt = tr.dt.0;
        
        let mut expired_effects = Vec::new();
        let mut effects_to_tick = Vec::new();
        
        // 收集需要處理的效果
        for (entity, effect) in (&tr.entities, &mut tw.skill_effects).join() {
            effect.update(dt, time);
            
            // 處理需要 tick 的效果
            if effect.should_tick(time) {
                effects_to_tick.push((entity, effect.clone()));
            }
            
            // 標記過期的效果
            if effect.is_expired() {
                expired_effects.push((entity, effect.clone()));
            }
        }
        
        // 處理 tick 效果
        for (effect_entity, mut effect) in effects_to_tick {
            Self::process_effect_tick(effect_entity, &mut effect, tr, tw);
            if let Some(eff) = tw.skill_effects.get_mut(effect_entity) {
                eff.tick(time);
            }
        }
        
        // 移除過期的技能效果
        for (effect_entity, effect) in expired_effects {
            Self::remove_effect(effect_entity, &effect, tr, tw);
        }
    }

    /// 處理技能效果的 tick
    fn process_effect_tick(
        effect_entity: Entity,
        effect: &mut SkillEffect,
        tr: &SkillRead,
        tw: &mut SkillWrite,
    ) {
        match &effect.effect_type {
            SkillEffectType::DamageOverTime => {
                Self::apply_damage_over_time(effect, tr, tw);
            }
            SkillEffectType::HealOverTime => {
                Self::apply_heal_over_time(effect, tr, tw);
            }
            SkillEffectType::AreaEffect => {
                Self::apply_area_effect(effect, tr, tw);
            }
            _ => {}
        }
    }

    /// 應用持續傷害效果
    fn apply_damage_over_time(
        effect: &SkillEffect,
        tr: &SkillRead,
        tw: &mut SkillWrite,
    ) {
        if let Some(target_entity) = effect.target {
            if let Some(target_pos) = tr.positions.get(target_entity) {
                let damage = effect.data.damage_per_tick;
                
                tw.outcomes.push(Outcome::Damage {
                    pos: target_pos.0,
                    phys: damage,
                    magi: 0.0,
                    real: 0.0,
                    source: effect.caster,
                    target: target_entity,
                });
            }
        }
    }

    /// 應用持續治療效果
    fn apply_heal_over_time(
        effect: &SkillEffect,
        tr: &SkillRead,
        tw: &mut SkillWrite,
    ) {
        if let Some(target_entity) = effect.target {
            if let Some(target_pos) = tr.positions.get(target_entity) {
                let heal = effect.data.heal_per_tick;
                
                tw.outcomes.push(Outcome::Heal {
                    pos: target_pos.0,
                    target: target_entity,
                    amount: heal,
                });
            }
        }
    }

    /// 應用範圍效果
    fn apply_area_effect(
        effect: &SkillEffect,
        tr: &SkillRead,
        tw: &mut SkillWrite,
    ) {
        if let Some(center_pos) = effect.area_center {
            let radius = effect.data.area_radius;
            
            // 找到範圍內的所有單位
            for (entity, pos, faction) in (&tr.entities, &tr.positions, &tr.factions).join() {
                let distance = (pos.0 - center_pos).magnitude();
                
                if distance <= radius {
                    // 根據陣營決定效果類型
                    let caster_faction = tr.factions.get(effect.caster);
                    let is_ally = caster_faction.map_or(false, |cf| cf.team_id == faction.team_id);
                    
                    if effect.data.affects_allies && is_ally {
                        // 對友軍施加正面效果
                        if effect.data.heal_per_tick > 0.0 {
                            tw.outcomes.push(Outcome::Heal {
                                pos: pos.0,
                                target: entity,
                                amount: effect.data.heal_per_tick,
                            });
                        }
                    } else if effect.data.affects_enemies && !is_ally {
                        // 對敵軍施加負面效果
                        if effect.data.damage_per_tick > 0.0 {
                            tw.outcomes.push(Outcome::Damage {
                                pos: pos.0,
                                phys: effect.data.damage_per_tick,
                                magi: 0.0,
                                real: 0.0,
                                source: effect.caster,
                                target: entity,
                            });
                        }
                    }
                }
            }
        }
    }

    /// 移除技能效果
    fn remove_effect(
        effect_entity: Entity,
        effect: &SkillEffect,
        tr: &SkillRead,
        tw: &mut SkillWrite,
    ) {
        // 移除效果前的清理工作
        match &effect.effect_type {
            SkillEffectType::Transform => {
                // 變身效果結束，恢復原始狀態
                log::info!("Transform effect ended for skill: {}", effect.skill_id);
            }
            SkillEffectType::Buff | SkillEffectType::Debuff => {
                // Buff/Debuff效果結束
                log::info!("Buff/Debuff effect ended for skill: {}", effect.skill_id);
            }
            _ => {}
        }
        
        tw.skill_effects.remove(effect_entity);
    }
}