use specs::{
    shred, Entities, Join, LazyUpdate, Read, ReadExpect, ReadStorage, SystemData,
    Write, WriteStorage, ParJoin, Entity, World,
};
use crate::comp::*;
use specs::prelude::ParallelIterator;
use std::{
    time::{Duration, Instant},
    collections::HashMap,
};
use rand::Rng;

#[derive(SystemData)]
pub struct DamageRead<'a> {
    entities: Entities<'a>,
    time: Read<'a, Time>,
    dt: Read<'a, DeltaTime>,
    units: ReadStorage<'a, Unit>,
    heroes: ReadStorage<'a, Hero>,
    factions: ReadStorage<'a, Faction>,
    properties: ReadStorage<'a, CProperty>,
    positions: ReadStorage<'a, Pos>,
}

#[derive(SystemData)]
pub struct DamageWrite<'a> {
    outcomes: Write<'a, Vec<Outcome>>,
    damage_instances: Write<'a, Vec<DamageInstance>>,
}

#[derive(Default)]
pub struct Sys;

impl<'a> System<'a> for Sys {
    type SystemData = (
        DamageRead<'a>,
        DamageWrite<'a>,
    );

    const NAME: &'static str = "damage";

    fn run(_job: &mut Job<Self>, (tr, mut tw): Self::SystemData) {
        let time = tr.time.0;
        let dt = tr.dt.0;
        
        // 收集所有單位的屬性用於傷害計算
        let mut unit_stats: HashMap<Entity, (f32, f32, f32, f32)> = HashMap::new(); // (armor, magic_resist, crit_chance, dodge_chance)
        
        // 收集 Unit 屬性
        for (entity, unit, properties) in (&tr.entities, &tr.units, &tr.properties).join() {
            unit_stats.insert(entity, (
                unit.base_armor,
                unit.magic_resistance,
                0.0, // TODO: 從裝備或技能獲取暴擊率
                0.0, // TODO: 從裝備或技能獲取閃避率
            ));
        }
        
        // 收集 Hero 屬性
        for (entity, hero, properties) in (&tr.entities, &tr.heroes, &tr.properties).join() {
            let crit_chance = hero.get_crit_chance();
            unit_stats.insert(entity, (
                properties.def_physic,
                0.0, // 魔抗暫時使用 0
                crit_chance,
                0.0, // 閃避率暫時使用 0
            ));
        }
        
        // 處理所有傷害實例
        let mut damage_results = Vec::new();
        let mut outcomes = Vec::new();
        
        for damage_inst in tw.damage_instances.drain(..) {
            let result = calculate_damage(&damage_inst, &unit_stats);
            
            // 生成傷害事件而不是直接修改組件
            if !result.is_dodged && result.total_damage > 0.0 {
                // 獲取目標位置
                let target_pos = tr.positions.get(damage_inst.target)
                    .map(|pos| pos.0)
                    .unwrap_or(vek::Vec2::new(0.0, 0.0));
                
                // 生成傷害事件
                outcomes.push(Outcome::Damage {
                    pos: target_pos,
                    phys: result.actual_damage.physical,
                    magi: result.actual_damage.magical,
                    real: result.actual_damage.pure,
                    source: damage_inst.source.source_entity,
                    target: damage_inst.target,
                });
                
                log::info!("Generated damage event: {:.1} total damage to target", result.total_damage);
            } else if result.is_dodged {
                log::info!("Attack dodged by target");
            }
            
            damage_results.push(result);
        }
        
        tw.outcomes.append(&mut outcomes);
        
        // TODO: 處理治療（生命偷取、法術吸血）
        // TODO: 發送傷害事件到UI
    }
}

/// 計算傷害的核心函數
fn calculate_damage(
    damage_inst: &DamageInstance, 
    unit_stats: &HashMap<Entity, (f32, f32, f32, f32)>
) -> DamageResult {
    let mut result = DamageResult {
        target: damage_inst.target,
        source: damage_inst.source.clone(),
        original_damage: damage_inst.damage_types.clone(),
        actual_damage: damage_inst.damage_types.clone(),
        total_damage: 0.0,
        absorbed: 0.0,
        is_critical: false,
        is_dodged: false,
        healing: 0.0,
    };
    
    // 獲取目標屬性
    let (armor, magic_resist, _, dodge_chance) = unit_stats.get(&damage_inst.target)
        .copied()
        .unwrap_or((0.0, 0.0, 0.0, 0.0));
    
    // 獲取攻擊者屬性
    let (_, _, crit_chance, _) = unit_stats.get(&damage_inst.source.source_entity)
        .copied()
        .unwrap_or((0.0, 0.0, 0.0, 0.0));
    
    // 檢查閃避
    if damage_inst.damage_flags.can_dodge && dodge_chance > 0.0 {
        let mut rng = rand::thread_rng();
        if rng.gen::<f32>() < dodge_chance {
            result.is_dodged = true;
            return result;
        }
    }
    
    // 檢查暴擊
    if damage_inst.damage_flags.can_crit && crit_chance > 0.0 {
        let mut rng = rand::thread_rng();
        if rng.gen::<f32>() < crit_chance {
            result.is_critical = true;
        }
    }
    
    // 計算物理傷害
    let mut physical_damage = damage_inst.damage_types.physical;
    if result.is_critical {
        physical_damage *= 2.0; // 暴擊傷害 200%
    }
    
    if !damage_inst.damage_flags.ignore_armor && armor > 0.0 {
        let damage_reduction = armor / (armor + 100.0);
        let absorbed = physical_damage * damage_reduction;
        physical_damage -= absorbed;
        result.absorbed += absorbed;
    }
    
    // 計算魔法傷害
    let mut magical_damage = damage_inst.damage_types.magical;
    if !damage_inst.damage_flags.ignore_magic_resist && magic_resist > 0.0 {
        let damage_reduction = magic_resist / 100.0;
        let absorbed = magical_damage * damage_reduction.min(0.75); // 魔抗上限 75%
        magical_damage -= absorbed;
        result.absorbed += absorbed;
    }
    
    // 純粹傷害不受防禦影響
    let pure_damage = damage_inst.damage_types.pure;
    
    // 更新實際傷害
    result.actual_damage.physical = physical_damage.max(0.0);
    result.actual_damage.magical = magical_damage.max(0.0);
    result.actual_damage.pure = pure_damage.max(0.0);
    result.total_damage = result.actual_damage.total();
    
    // 計算治療（生命偷取、法術吸血）
    if damage_inst.damage_flags.lifesteal > 0.0 {
        result.healing += result.actual_damage.physical * damage_inst.damage_flags.lifesteal;
    }
    if damage_inst.damage_flags.spell_vamp > 0.0 {
        result.healing += result.actual_damage.magical * damage_inst.damage_flags.spell_vamp;
    }
    
    result
}