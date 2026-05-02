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
use omoba_sim::Fixed64;

/// Per-entity SimRng op kinds for damage_tick. Each (entity, op) pair gets its
/// own deterministic stream — keep these constants stable; reordering would
/// invalidate replay determinism.
const OP_DODGE: u32 = 0;
const OP_CRIT: u32 = 1;

#[derive(SystemData)]
pub struct DamageRead<'a> {
    entities: Entities<'a>,
    time: Read<'a, Time>,
    dt: Read<'a, DeltaTime>,
    master_seed: Read<'a, MasterSeed>,
    tick: Read<'a, Tick>,
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
        let master_seed: u64 = tr.master_seed.0;
        let tick: u32 = tr.tick.0 as u32;

        // 收集所有單位的屬性用於傷害計算
        // (armor, magic_resist, crit_chance, dodge_chance) — all Fixed64 (Phase 1c.3).
        let mut unit_stats: HashMap<Entity, (Fixed64, Fixed64, Fixed64, Fixed64)> = HashMap::new();

        // 收集 Unit 屬性
        for (entity, unit, properties) in (&tr.entities, &tr.units, &tr.properties).join() {
            unit_stats.insert(entity, (
                unit.base_armor,
                unit.magic_resistance,
                Fixed64::ZERO, // TODO: 從裝備或技能獲取暴擊率
                Fixed64::ZERO, // TODO: 從裝備或技能獲取閃避率
            ));
        }

        // 收集 Hero 屬性
        for (entity, hero, properties) in (&tr.entities, &tr.heroes, &tr.properties).join() {
            let crit_chance = hero.get_crit_chance();
            unit_stats.insert(entity, (
                properties.def_physic,
                Fixed64::ZERO, // 魔抗暫時使用 0
                crit_chance,
                Fixed64::ZERO, // 閃避率暫時使用 0
            ));
        }

        // 處理所有傷害實例
        let mut damage_results = Vec::new();
        let mut outcomes = Vec::new();

        for damage_inst in tw.damage_instances.drain(..) {
            let result = calculate_damage(&damage_inst, &unit_stats, master_seed, tick);

            // 生成傷害事件而不是直接修改組件
            if !result.is_dodged && result.total_damage > Fixed64::ZERO {
                // Phase 1c.3: Outcome::Damage.pos now omoba_sim::Vec2 (Phase 1c.2).
                let target_pos = tr.positions.get(damage_inst.target)
                    .map(|p| p.0)
                    .unwrap_or(omoba_sim::Vec2::ZERO);

                // 生成傷害事件
                outcomes.push(Outcome::Damage {
                    pos: target_pos,
                    phys: result.actual_damage.physical,
                    magi: result.actual_damage.magical,
                    real: result.actual_damage.pure,
                    source: damage_inst.source.source_entity,
                    target: damage_inst.target,
                    predeclared: false, // ability-driven damage path — authoritative
                });

                log::info!(
                    "Generated damage event: {:.1} total damage to target",
                    result.total_damage.to_f32_for_render()
                );
            } else if result.is_dodged {
                log::info!("Attack dodged by target");
            }

            // 生命偷取 / 法術吸血：calculate_damage 已聚合到 result.healing，這裡 emit Heal 給來源
            if !result.is_dodged && result.healing > Fixed64::ZERO {
                let source_entity = damage_inst.source.source_entity;
                // Phase 1c.3: Outcome::Heal.pos now omoba_sim::Vec2 (Phase 1c.2).
                let source_pos = tr.positions.get(source_entity)
                    .map(|p| p.0)
                    .unwrap_or_else(|| {
                        tr.positions.get(damage_inst.target)
                            .map(|p| p.0)
                            .unwrap_or(omoba_sim::Vec2::ZERO)
                    });
                outcomes.push(Outcome::Heal {
                    pos: source_pos,
                    target: source_entity,
                    amount: result.healing,
                });
            }

            damage_results.push(result);
        }

        tw.outcomes.append(&mut outcomes);
    }
}

/// 計算傷害的核心函數
/// Phase 1c.3: full Fixed64 — armor / resist / damage / crit / dodge all deterministic.
fn calculate_damage(
    damage_inst: &DamageInstance,
    unit_stats: &HashMap<Entity, (Fixed64, Fixed64, Fixed64, Fixed64)>,
    master_seed: u64,
    tick: u32,
) -> DamageResult {
    let mut result = DamageResult {
        target: damage_inst.target,
        source: damage_inst.source.clone(),
        original_damage: damage_inst.damage_types.clone(),
        actual_damage: damage_inst.damage_types.clone(),
        total_damage: Fixed64::ZERO,
        absorbed: Fixed64::ZERO,
        is_critical: false,
        is_dodged: false,
        healing: Fixed64::ZERO,
    };

    // 獲取目標屬性
    let (armor, magic_resist, _, dodge_chance) = unit_stats.get(&damage_inst.target)
        .copied()
        .unwrap_or((Fixed64::ZERO, Fixed64::ZERO, Fixed64::ZERO, Fixed64::ZERO));

    // 獲取攻擊者屬性
    let (_, _, crit_chance, _) = unit_stats.get(&damage_inst.source.source_entity)
        .copied()
        .unwrap_or((Fixed64::ZERO, Fixed64::ZERO, Fixed64::ZERO, Fixed64::ZERO));

    let victim_id: u32 = damage_inst.target.id();
    let attacker_id: u32 = damage_inst.source.source_entity.id();

    // 檢查閃避 — Phase 1c.3: deterministic SimRng stream per (victim, OP_DODGE)
    if damage_inst.damage_flags.can_dodge && dodge_chance > Fixed64::ZERO {
        let mut dodge_rng =
            omoba_sim::SimRng::from_master_entity(master_seed, tick, victim_id, OP_DODGE);
        // gen_fixed64_unit returns Fixed64 in [0, 1) with raw in [0, 1024).
        let dodge_roll: Fixed64 = dodge_rng.gen_fixed64_unit();
        if dodge_roll < dodge_chance {
            result.is_dodged = true;
            return result;
        }
    }

    // 檢查暴擊 — Phase 1c.3: deterministic SimRng stream per (attacker, OP_CRIT)
    if damage_inst.damage_flags.can_crit && crit_chance > Fixed64::ZERO {
        let mut crit_rng =
            omoba_sim::SimRng::from_master_entity(master_seed, tick, attacker_id, OP_CRIT);
        let crit_roll: Fixed64 = crit_rng.gen_fixed64_unit();
        if crit_roll < crit_chance {
            result.is_critical = true;
        }
    }

    // 計算物理傷害
    let mut physical_damage = damage_inst.damage_types.physical;
    if result.is_critical {
        physical_damage = physical_damage * Fixed64::from_i32(2); // 暴擊傷害 200%
    }

    let hundred = Fixed64::from_i32(100);
    if !damage_inst.damage_flags.ignore_armor && armor > Fixed64::ZERO {
        let damage_reduction = armor / (armor + hundred);
        let absorbed = physical_damage * damage_reduction;
        physical_damage = physical_damage - absorbed;
        result.absorbed = result.absorbed + absorbed;
    }

    // 計算魔法傷害
    let mut magical_damage = damage_inst.damage_types.magical;
    if !damage_inst.damage_flags.ignore_magic_resist && magic_resist > Fixed64::ZERO {
        let mut damage_reduction = magic_resist / hundred;
        let cap = Fixed64::from_raw(768); // 0.75 in Q22.10
        if damage_reduction > cap {
            damage_reduction = cap;
        }
        let absorbed = magical_damage * damage_reduction;
        magical_damage = magical_damage - absorbed;
        result.absorbed = result.absorbed + absorbed;
    }

    // 純粹傷害不受防禦影響
    let pure_damage = damage_inst.damage_types.pure;

    // 更新實際傷害
    result.actual_damage.physical = if physical_damage < Fixed64::ZERO { Fixed64::ZERO } else { physical_damage };
    result.actual_damage.magical = if magical_damage < Fixed64::ZERO { Fixed64::ZERO } else { magical_damage };
    result.actual_damage.pure = if pure_damage < Fixed64::ZERO { Fixed64::ZERO } else { pure_damage };
    result.total_damage = result.actual_damage.total();

    // 計算治療（生命偷取、法術吸血）
    if damage_inst.damage_flags.lifesteal > Fixed64::ZERO {
        result.healing = result.healing + result.actual_damage.physical * damage_inst.damage_flags.lifesteal;
    }
    if damage_inst.damage_flags.spell_vamp > Fixed64::ZERO {
        result.healing = result.healing + result.actual_damage.magical * damage_inst.damage_flags.spell_vamp;
    }

    result
}