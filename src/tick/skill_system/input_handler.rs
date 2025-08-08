use specs::{Entity, Join};
use crate::comp::*;
use super::{SkillRead, SkillWrite, SkillProcessor};

/// 技能輸入處理器
pub struct InputHandler;

impl InputHandler {
    /// 處理所有技能輸入
    pub fn process_inputs(tr: &SkillRead, tw: &mut SkillWrite) {
        let skill_inputs: Vec<SkillInput> = tw.skill_inputs.drain(..).collect();
        
        for input in skill_inputs {
            Self::process_single_input(&input, tr, tw);
        }
    }

    /// 處理單個技能輸入
    fn process_single_input(
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
        let processor = SkillProcessor::get_ability_processor();
        if let Ok(mut processor_guard) = processor.lock() {
            if SkillProcessor::try_process_with_ability_system(
                &mut *processor_guard, input, skill_entity, &ability_id, tr, tw
            ) {
                log::info!("技能 {} 使用ability-system處理", ability_id);
                return;
            }
        }
        
        // 回退到硬編碼邏輯
        Self::execute_hardcoded_skill(skill_entity, input, &ability, tr, tw);
    }

    /// 執行硬編碼技能邏輯
    fn execute_hardcoded_skill(
        skill_entity: Entity,
        input: &SkillInput,
        ability: &Ability,
        tr: &SkillRead,
        tw: &mut SkillWrite,
    ) {
        match ability.id.as_str() {
            "sniper_mode" => {
                Self::execute_sniper_mode(skill_entity, input, ability, tr, tw);
            }
            "saika_reinforcements" => {
                Self::execute_saika_reinforcements(skill_entity, input, ability, tr, tw);
            }
            "rain_iron_cannon" => {
                Self::execute_rain_iron_cannon(skill_entity, input, ability, tr, tw);
            }
            "three_stage_technique" => {
                Self::execute_three_stage_technique(skill_entity, input, ability, tr, tw);
            }
            _ => {
                log::warn!("Unknown skill: {}", ability.id);
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

    /// 執行雜賀眾召喚 (E)
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
            // 召喚雜賀鐵炮兵的邏輯
            let summon_count = skill.current_level;
            let target_pos = input.target_position.unwrap_or_else(|| {
                if let Some(caster_pos) = tr.positions.get(input.caster) {
                    caster_pos.0
                } else {
                    vek::Vec2::new(0.0, 0.0)
                }
            });
            
            // 創建召喚單位
            for i in 0..summon_count {
                let angle = (i as f32 / summon_count as f32) * std::f32::consts::PI * 2.0;
                let offset = vek::Vec2::new(angle.cos(), angle.sin()) * 100.0;
                let summon_pos = target_pos + offset;
                
                // 這裡應該創建召喚單位，但需要完整的召喚系統
                log::info!("召喚雜賀鐵炮兵 at position: {:?}", summon_pos);
            }
        }
    }

    /// 執行雨鐵砲 (R)
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
            // 範圍傷害技能
            if let Some(target_pos) = input.target_position {
                let skill_level = skill.current_level;
                let base_damage = 150.0 + (skill_level - 1) as f32 * 50.0;
                let area_radius = 300.0 + (skill_level - 1) as f32 * 50.0;
                
                // 創建範圍傷害效果
                let mut effect = SkillEffect::new(
                    ability.id.clone(),
                    input.caster,
                    SkillEffectType::Area,
                    0.1, // 瞬間效果
                );
                
                // 暫時使用現有的字段來儲存範圍效果資料
                effect.target_pos = Some(target_pos);
                effect.data.damage_per_second = base_damage;
                // 使用 duration 字段來儲存範圍半徑
                effect.duration = area_radius;
                
                let effect_entity = tr.entities.create();
                tw.skill_effects.insert(effect_entity, effect);
                
                log::info!("Rain Iron Cannon cast at position: {:?}", target_pos);
            }
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
            // 三段攻擊技能
            if let Some(target) = input.target_entity {
                let skill_level = skill.current_level;
                let damage_per_hit = 75.0 + (skill_level - 1) as f32 * 25.0;
                
                let caster_pos = tr.positions.get(input.caster)
                    .map(|p| p.0)
                    .unwrap_or_default();
                
                // 創建三次攻擊
                for i in 0..3 {
                    tw.outcomes.push(Outcome::Damage {
                        pos: caster_pos,
                        phys: damage_per_hit,
                        magi: 0.0,
                        real: 0.0,
                        source: input.caster,
                        target: target,
                    });
                }
                
                log::info!("Three Stage Technique executed on target: {:?}", target);
            }
        }
    }
}