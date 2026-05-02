//! Drain `ScriptEventQueue` and dispatch hooks to the matching `UnitScript`s.
//!
//! This runs on the main thread with exclusive `&mut World` (E1), so
//! `WorldAdapter` does not need any locking. Every hook invocation is
//! wrapped in `catch_unwind` (P1 — panic → log + skip).

use abi_stable::{
    RMut,
    sabi_trait::prelude::TD_Opaque,
    std_types::{RNone, RSome},
};
use crossbeam_channel::Sender;
use omb_script_abi::{
    types::{DamageInfo, DamageKind, EntityHandle, Fixed64, Target, Vec2},
    world::{GameWorld, GameWorldDyn, GameWorld_TO},
};
use specs::{Entity, Join, World, WorldExt};
use std::panic::{catch_unwind, AssertUnwindSafe};

use std::time::Instant;

use crate::transport::OutboundMsg;
use super::event::{ScriptEvent, ScriptEventQueue, SkillTarget};
use super::registry::ScriptRegistry;
use super::tag::ScriptUnitTag;
use super::world_adapter::{AdapterCache, WorldAdapter};

/// Main entry point — call once per tick, AFTER all parallel tick systems
/// have finished and BEFORE `world.maintain()`.
///
/// 每 tick 先對所有有 `ScriptUnitTag` 的 entity 派發 `on_tick`，然後 drain
/// `ScriptEventQueue` 處理其他 hooks（`AttackHit`, `Death` 等）。
pub fn run_script_dispatch(
    world: &mut World,
    registry: &ScriptRegistry,
    rng_seed: u64,
    dt: Fixed64,
    mqtx: Sender<OutboundMsg>,
) {
    // 先收集所有帶 tag 的 entity（避免 adapter 建立後又要 read_storage 借用衝突）
    let tagged: Vec<(Entity, String)> = {
        let entities = world.entities();
        let tags = world.read_storage::<ScriptUnitTag>();
        (&entities, &tags).join().map(|(e, t)| (e, t.unit_id.clone())).collect()
    };

    let events = {
        let mut queue = world.write_resource::<ScriptEventQueue>();
        queue.drain()
    };

    if tagged.is_empty() && events.is_empty() {
        return;
    }

    // One adapter per tick; RNG is local to this dispatch pass for
    // deterministic replay (seed driven by tick counter upstream).
    // Cached storages 在這個 adapter 生命週期內共用，所有 GameWorld API 不再
    // 重新 borrow specs storage（單筆 4.17µs → 預期 ~1.5µs）。
    let mut adapter = WorldAdapter::new(&*world, rng_seed, mqtx);

    // Dispatch queued events first（Spawn / AttackHit / Damage / Death / ...）
    // 這樣新 spawn 的塔 on_spawn 能先初始化 stats，第一次 on_tick 看得到正確值
    let event_count = events.len();
    let event_t = Instant::now();
    for ev in events {
        dispatch_one(&mut adapter, registry, ev);
    }
    let event_ns = event_t.elapsed().as_nanos();

    // Dispatch on_tick for every tagged entity（塔主動行為）
    // 收集 (script_id, ns) — 不能在迴圈裡觸 adapter borrow 的 world，所以先攢著
    // 之後 drop(adapter) 再一次性 push 到 TickProfile。
    let mut on_tick_timings: Vec<(String, u128)> = Vec::with_capacity(tagged.len());
    for (ent, uid) in &tagged {
        let Some(script) = registry.get(uid) else { continue };
        let handle = WorldAdapter::entity_to_handle(*ent);
        let t = Instant::now();
        let mut world_dyn = world_dyn_of(&mut adapter);
        let r = catch_unwind(AssertUnwindSafe(|| {
            script.on_tick(handle, dt, &mut world_dyn);
        }));
        let ns = t.elapsed().as_nanos();
        if r.is_err() {
            log::error!("[scripting] panic in on_tick of {}", uid);
        }
        on_tick_timings.push((uid.clone(), ns));
    }

    // 釋放 adapter 對 world 的 &mut 借用，才能拿到 TickProfile resource
    drop(adapter);

    {
        use crate::comp::TickProfile;
        let mut profile = world.write_resource::<TickProfile>();
        if event_count > 0 {
            // queued events 的耗時拆出來（events 內部又會分 Spawn/Damage/...，這裡只收總和）
            for _ in 0..event_count {
                profile.record_script_event(event_ns / event_count as u128);
            }
        }
        for (id, ns) in on_tick_timings {
            profile.record_script(&id, ns);
        }
    }
}

fn dispatch_one(adapter: &mut WorldAdapter<'_>, registry: &ScriptRegistry, ev: ScriptEvent) {
    match ev {
        ScriptEvent::Spawn { e } => {
            with_script(adapter, registry, e, |script, handle, world_dyn| {
                script.on_spawn(handle, world_dyn);
            });
        }

        ScriptEvent::Death { victim, killer } => {
            let killer_handle = killer.map(WorldAdapter::entity_to_handle);
            with_script(adapter, registry, victim, |script, handle, world_dyn| {
                let k = match killer_handle {
                    Some(h) => RSome(h),
                    None => RNone,
                };
                script.on_death(handle, k, world_dyn);
            });
        }

        ScriptEvent::Damage { attacker, victim, amount, kind } => {
            let victim_handle = WorldAdapter::entity_to_handle(victim);
            let attacker_handle_opt = attacker.map(WorldAdapter::entity_to_handle);

            let mut info = DamageInfo {
                attacker: match attacker_handle_opt {
                    Some(h) => RSome(h),
                    None => RNone,
                },
                // Phase 1c.3: amount is already Fixed64 (ScriptEvent::Damage migrated 1c.2).
                amount,
                kind,
            };

            // 1) victim.on_damage_taken (may mutate info.amount)
            if let Some(uid) = script_id_of(&adapter.cache, victim) {
                if let Some(script) = registry.get(&uid) {
                    let mut world_dyn = world_dyn_of(adapter);
                    let r = catch_unwind(AssertUnwindSafe(|| {
                        script.on_damage_taken(victim_handle, &mut info, &mut world_dyn)
                    }));
                    if let Err(_) = r {
                        log::error!("[scripting] panic in on_damage_taken of {}", uid);
                    }
                }
            }

            // 2) attacker.on_damage_dealt (reads final amount)
            if let (Some(att), Some(att_h)) = (attacker, attacker_handle_opt) {
                if let Some(uid) = script_id_of(&adapter.cache, att) {
                    if let Some(script) = registry.get(&uid) {
                        let mut world_dyn = world_dyn_of(adapter);
                        let r = catch_unwind(AssertUnwindSafe(|| {
                            script.on_damage_dealt(att_h, victim_handle, info.amount, &mut world_dyn)
                        }));
                        if let Err(_) = r {
                            log::error!("[scripting] panic in on_damage_dealt of {}", uid);
                        }
                    }
                }
            }

            // 3) host applies final amount
            // PHASE 2: apply_damage helper still takes f32 — full Fixed64 damage pipeline lands with
            // Outcome::Damage redesign in Phase 2 KCP tag rework.
            apply_damage(adapter, victim, info.amount.to_f32_for_render(), info.kind);
        }

        ScriptEvent::SkillCast { caster, skill_id, target } => {
            // Silence 檢查：施法者若有 silence/stun buff，跳過整個 cast
            if adapter.cache.buffs.is_silenced(caster) {
                log::info!(
                    "[scripting] skill '{}' by entity {} blocked — silenced/stunned",
                    skill_id,
                    caster.id()
                );
                return;
            }

            // Cooldown / Passive gate：
            // - Passive 技能不該走 SkillCast 路徑（on_learn 已處理）
            // - Active / Toggle / Ultimate：若仍在 CD 中直接拒絕
            if let Some(hero) = adapter.cache.hero.get(caster) {
                if hero.is_on_cooldown(&skill_id) {
                    // NOTE: log uses f32 boundary — Fixed64 has no Display.
                    log::info!(
                        "[scripting] skill '{}' blocked — on cooldown ({:.1}s remaining)",
                        skill_id,
                        hero.get_cooldown(&skill_id).to_f32_for_render()
                    );
                    return;
                }
            }
            if let Some((def, _)) = registry.get_ability(&skill_id) {
                if def.ability_type == omoba_core::ability_meta::AbilityType::Passive {
                    log::info!(
                        "[scripting] skill '{}' is passive — cannot be cast actively",
                        skill_id
                    );
                    return;
                }
            }

            let caster_handle = WorldAdapter::entity_to_handle(caster);
            let target_abi = match target {
                SkillTarget::Entity(e) => Target::Entity(WorldAdapter::entity_to_handle(e)),
                // Phase 1c.3: SkillTarget::Point now { x: Fixed64, y: Fixed64 } (Phase 1c.2).
                SkillTarget::Point { x, y } => Target::Point(Vec2 { x, y }),
                SkillTarget::None => Target::None,
            };

            // 取 caster 英雄身上該技能的等級（未習得則預設 1 讓腳本至少 fire）
            let level: u8 = adapter.cache.hero
                .get(caster)
                .and_then(|h| h.ability_levels.get(&skill_id).copied())
                .map(|lv| lv.max(1) as u8)
                .unwrap_or(1);

            // 1) 先呼叫 caster unit 本身的 on_skill_cast（pre-processing 機會）
            {
                let skill_id_for_unit = skill_id.clone();
                let target_for_unit = target_abi.clone();
                with_script(adapter, registry, caster, move |script, handle, world_dyn| {
                    script.on_skill_cast(
                        handle,
                        (&*skill_id_for_unit).into(),
                        target_for_unit,
                        world_dyn,
                    );
                });
            }

            // 2) 呼叫 ability 本身的 execute（DLL handler 實際執行效果）
            if let Some((def, ability_script)) = registry.get_ability(&skill_id) {
                let level_data = def.get_level_data(level).cloned();
                let level_data_json = level_data
                    .as_ref()
                    .and_then(|ld| serde_json::to_string(ld).ok())
                    .unwrap_or_else(|| "{}".to_string());
                let cd_seconds = level_data.as_ref().map(|ld| ld.cooldown).unwrap_or(0.0);

                let exec_ok = {
                    let mut world_dyn = world_dyn_of(adapter);
                    let r = catch_unwind(AssertUnwindSafe(|| {
                        ability_script.execute(
                            caster_handle,
                            target_abi,
                            level,
                            (&*level_data_json).into(),
                            &mut world_dyn,
                        )
                    }));
                    match r {
                        Ok(res) if res.is_err() => {
                            log::warn!(
                                "[scripting] ability '{}' execute returned error",
                                skill_id
                            );
                            false
                        }
                        Ok(_) => true,
                        Err(_) => {
                            log::error!(
                                "[scripting] panic in AbilityScript::execute of {}",
                                skill_id
                            );
                            false
                        }
                    }
                    // world_dyn 在此 block 結束時釋放 adapter 的借用
                };

                // 執行成功後啟動 CD；失敗不扣 CD（讓玩家重試）
                if exec_ok && cd_seconds > 0.0 {
                    if let Some(hero) = adapter.cache.hero.get_mut(caster) {
                        // PHASE 2: AbilityLevelData.cooldown still f32 — convert at boundary; full Fixed64
                        // ability metadata redesign in Phase 2 KCP tag rework.
                        hero.start_cooldown(&skill_id, Fixed64::from_raw((cd_seconds * 1024.0) as i64));
                    }
                }
            } else {
                log::debug!(
                    "[scripting] SkillCast '{}' has no registered AbilityScript handler",
                    skill_id
                );
            }
        }

        ScriptEvent::SkillLearn { caster, skill_id, new_level } => {
            // 派發 on_learn；Passive 技在此套永久 buff
            if let Some((_def, ability_script)) = registry.get_ability(&skill_id) {
                let caster_handle = WorldAdapter::entity_to_handle(caster);
                let mut world_dyn = world_dyn_of(adapter);
                let r = catch_unwind(AssertUnwindSafe(|| {
                    ability_script.on_learn(caster_handle, new_level, &mut world_dyn);
                }));
                if r.is_err() {
                    log::error!("[scripting] panic in AbilityScript::on_learn of {}", skill_id);
                }
            }
        }

        ScriptEvent::AttackHit { attacker, victim } => {
            let victim_handle = WorldAdapter::entity_to_handle(victim);
            // 1) UnitScript hook（tower / creep 等用這個做命中附加效果）
            with_script(adapter, registry, attacker, |script, handle, world_dyn| {
                script.on_attack_hit(handle, victim_handle, world_dyn);
            });

            // 2) 若 attacker 是 Hero，輪詢已學的 Passive ability 並呼 on_attack_hit。
            //    先 snapshot passive ids + levels 避免 dispatch 中借用 hero storage 與 world_dyn 衝突。
            let passive_calls: Vec<(String, u8)> = match adapter.cache.hero.get(attacker) {
                Some(hero) => hero
                    .ability_levels
                    .iter()
                    .filter(|(_, lv)| **lv > 0)
                    .filter_map(|(ability_id, lv)| {
                        registry.get_ability(ability_id).and_then(|(def, _)| {
                            if def.ability_type == omoba_core::ability_meta::AbilityType::Passive {
                                Some((ability_id.clone(), (*lv).max(1) as u8))
                            } else {
                                None
                            }
                        })
                    })
                    .collect(),
                None => Vec::new(),
            };
            if !passive_calls.is_empty() {
                let attacker_handle = WorldAdapter::entity_to_handle(attacker);
                for (ability_id, lv) in passive_calls {
                    if let Some((_, ability_script)) = registry.get_ability(&ability_id) {
                        let mut world_dyn = world_dyn_of(adapter);
                        let r = catch_unwind(AssertUnwindSafe(|| {
                            ability_script.on_attack_hit(
                                attacker_handle,
                                attacker_handle,
                                victim_handle,
                                lv,
                                &mut world_dyn,
                            );
                        }));
                        if r.is_err() {
                            log::error!(
                                "[scripting] panic in passive AbilityScript::on_attack_hit of {}",
                                ability_id
                            );
                        }
                    }
                }
            }
        }

        ScriptEvent::Respawn { e } => {
            with_script(adapter, registry, e, |script, handle, world_dyn| {
                script.on_respawn(handle, world_dyn);
            });
        }

        ScriptEvent::AttackStart { attacker, target } => {
            let target_handle = target.map(WorldAdapter::entity_to_handle);
            let t_opt = match target_handle {
                Some(h) => RSome(h),
                None => RNone,
            };
            with_script(adapter, registry, attacker, move |script, handle, world_dyn| {
                script.on_attack_start(handle, t_opt, world_dyn);
            });
        }

        ScriptEvent::AttackLanded { attacker, victim, damage } => {
            let victim_handle = WorldAdapter::entity_to_handle(victim);
            // Phase 1c.3: damage already Fixed64 (ScriptEvent migrated 1c.2).
            with_script(adapter, registry, attacker, |script, handle, world_dyn| {
                script.on_attack_landed(handle, victim_handle, damage, world_dyn);
            });
        }

        ScriptEvent::AttackFail { attacker, victim } => {
            let victim_handle = WorldAdapter::entity_to_handle(victim);
            with_script(adapter, registry, attacker, |script, handle, world_dyn| {
                script.on_attack_fail(handle, victim_handle, world_dyn);
            });
        }

        ScriptEvent::Attacked { attacker, victim } => {
            let attacker_handle = WorldAdapter::entity_to_handle(attacker);
            with_script(adapter, registry, victim, |script, handle, world_dyn| {
                script.on_attacked(handle, attacker_handle, world_dyn);
            });
        }

        ScriptEvent::HealthGained { e, amount } => {
            // Phase 1c.3: amount already Fixed64.
            with_script(adapter, registry, e, |script, handle, world_dyn| {
                script.on_health_gained(handle, amount, world_dyn);
            });
        }

        ScriptEvent::ManaGained { e, amount } => {
            // Phase 1c.3: amount already Fixed64.
            with_script(adapter, registry, e, |script, handle, world_dyn| {
                script.on_mana_gained(handle, amount, world_dyn);
            });
        }

        ScriptEvent::SpentMana { caster, cost, ability_id } => {
            let id_clone = ability_id.clone();
            // Phase 1c.3: cost already Fixed64.
            with_script(adapter, registry, caster, move |script, handle, world_dyn| {
                script.on_spent_mana(handle, cost, (&*id_clone).into(), world_dyn);
            });
        }

        ScriptEvent::HealReceived { target, amount, source } => {
            let source_opt = match source.map(WorldAdapter::entity_to_handle) {
                Some(h) => RSome(h),
                None => RNone,
            };
            // Phase 1c.3: amount already Fixed64.
            with_script(adapter, registry, target, move |script, handle, world_dyn| {
                script.on_heal_received(handle, amount, source_opt, world_dyn);
            });
        }

        ScriptEvent::StateChanged { e, state_id, active } => {
            let id_clone = state_id.clone();
            with_script(adapter, registry, e, move |script, handle, world_dyn| {
                script.on_state_changed(handle, (&*id_clone).into(), active, world_dyn);
            });
        }

        ScriptEvent::ModifierAdded { e, modifier_id } => {
            let id_clone = modifier_id.clone();
            with_script(adapter, registry, e, move |script, handle, world_dyn| {
                script.on_modifier_added(handle, (&*id_clone).into(), world_dyn);
            });
        }

        ScriptEvent::ModifierRemoved { e, modifier_id } => {
            let id_clone = modifier_id.clone();
            with_script(adapter, registry, e, move |script, handle, world_dyn| {
                script.on_modifier_removed(handle, (&*id_clone).into(), world_dyn);
            });
        }

        ScriptEvent::Order { e, order_kind, target } => {
            let kind_clone = order_kind.clone();
            let target_abi = match target {
                SkillTarget::Entity(t) => Target::Entity(WorldAdapter::entity_to_handle(t)),
                // Phase 1c.3: SkillTarget::Point now { x: Fixed64, y: Fixed64 } (Phase 1c.2).
                SkillTarget::Point { x, y } => Target::Point(Vec2 { x, y }),
                SkillTarget::None => Target::None,
            };
            with_script(adapter, registry, e, move |script, handle, world_dyn| {
                script.on_order(handle, (&*kind_clone).into(), target_abi, world_dyn);
            });
        }
    }
}

/// Look up the `ScriptUnitTag` for an entity, returning its `unit_id`.
fn script_id_of(cache: &AdapterCache, e: Entity) -> Option<String> {
    cache.tags.get(e).map(|t| t.unit_id.clone())
}

/// Helper: fetch script for an entity and invoke `f` with (script, handle, world).
fn with_script<F>(
    adapter: &mut WorldAdapter<'_>,
    registry: &ScriptRegistry,
    entity: Entity,
    f: F,
) where
    F: FnOnce(&omb_script_abi::script::UnitScript_TO<'static, abi_stable::std_types::RBox<()>>,
              EntityHandle,
              &mut GameWorldDyn<'_>),
{
    let Some(uid) = script_id_of(&adapter.cache, entity) else { return };
    let Some(script) = registry.get(&uid) else { return };

    let handle = WorldAdapter::entity_to_handle(entity);
    let mut world_dyn = world_dyn_of(adapter);

    let r = catch_unwind(AssertUnwindSafe(|| {
        f(script, handle, &mut world_dyn);
    }));
    if let Err(_) = r {
        log::error!("[scripting] panic in hook of unit {}", uid);
    }
}

/// Build a `GameWorldDyn` borrowing the adapter for one hook call.
fn world_dyn_of<'a>(adapter: &'a mut WorldAdapter<'_>) -> GameWorldDyn<'a> {
    GameWorld_TO::from_ptr(RMut::new(adapter), TD_Opaque)
}

/// Host side of damage application (mirrors what `damage_tick` does for
/// non-scripted units, kept minimal for the PoC).
///
/// NOTE — for PoC-1 the host damage pipeline (`Outcome::Damage` →
/// `CombatEventHandler::handle_damage`) is still the authoritative path.
/// `ScriptEvent::Damage` is not currently enqueued, so this helper only
/// exists for future use when we wire scripts into the damage pipeline.
fn apply_damage(adapter: &mut WorldAdapter<'_>, victim: Entity, amount: f32, _kind: DamageKind) {
    // Phase 1c.3: CProperty.hp is Fixed64 (Phase 1c.2); convert at boundary from f32 amount.
    // PHASE 2: amount stays f32 — full Fixed64 damage pipeline lands with Outcome::Damage redesign in Phase 2.
    let amount_fx = Fixed64::from_raw((amount * 1024.0) as i64);
    if let Some(p) = adapter.cache.cprop.get_mut(victim) {
        let new_hp = p.hp - amount_fx;
        p.hp = if new_hp < Fixed64::ZERO { Fixed64::ZERO } else { new_hp };
        return;
    }
    if let Some(u) = adapter.cache.unit.get_mut(victim) {
        u.current_hp = (u.current_hp - amount as i32).max(0);
    }
}
