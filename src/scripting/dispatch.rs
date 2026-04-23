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
    types::{DamageInfo, DamageKind, EntityHandle, Target},
    world::{GameWorld, GameWorldDyn, GameWorld_TO},
};
use specs::{Entity, Join, World, WorldExt};
use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::transport::OutboundMsg;
use super::event::{ScriptEvent, ScriptEventQueue, SkillTarget};
use super::registry::ScriptRegistry;
use super::tag::ScriptUnitTag;
use super::world_adapter::WorldAdapter;

/// Main entry point — call once per tick, AFTER all parallel tick systems
/// have finished and BEFORE `world.maintain()`.
///
/// 每 tick 先對所有有 `ScriptUnitTag` 的 entity 派發 `on_tick`，然後 drain
/// `ScriptEventQueue` 處理其他 hooks（`AttackHit`, `Death` 等）。
pub fn run_script_dispatch(
    world: &mut World,
    registry: &ScriptRegistry,
    rng_seed: u64,
    dt: f32,
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
    let mut adapter = WorldAdapter::new(world, rng_seed, mqtx);

    // Dispatch queued events first（Spawn / AttackHit / Damage / Death / ...）
    // 這樣新 spawn 的塔 on_spawn 能先初始化 stats，第一次 on_tick 看得到正確值
    for ev in events {
        dispatch_one(&mut adapter, registry, ev);
    }

    // Dispatch on_tick for every tagged entity（塔主動行為）
    for (ent, uid) in &tagged {
        let Some(script) = registry.get(uid) else { continue };
        let handle = WorldAdapter::entity_to_handle(*ent);
        let mut world_dyn = world_dyn_of(&mut adapter);
        let r = catch_unwind(AssertUnwindSafe(|| {
            script.on_tick(handle, dt, &mut world_dyn);
        }));
        if r.is_err() {
            log::error!("[scripting] panic in on_tick of {}", uid);
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
                amount,
                kind,
            };

            // 1) victim.on_damage_taken (may mutate info.amount)
            if let Some(uid) = script_id_of(adapter.world, victim) {
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
                if let Some(uid) = script_id_of(adapter.world, att) {
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
            apply_damage(adapter, victim, info.amount, info.kind);
        }

        ScriptEvent::SkillCast { caster, skill_id, target } => {
            // Silence 檢查：施法者若有 silence/stun buff，跳過整個 cast
            {
                let store = adapter.world.read_resource::<crate::ability_runtime::BuffStore>();
                if store.is_silenced(caster) {
                    log::info!(
                        "[scripting] skill '{}' by entity {} blocked — silenced/stunned",
                        skill_id,
                        caster.id()
                    );
                    return;
                }
            }

            let caster_handle = WorldAdapter::entity_to_handle(caster);
            let target_abi = match target {
                SkillTarget::Entity(e) => Target::Entity(WorldAdapter::entity_to_handle(e)),
                SkillTarget::Point(x, y) => Target::Point(omb_script_abi::types::Vec2f { x, y }),
                SkillTarget::None => Target::None,
            };

            // 取 caster 英雄身上該技能的等級（未習得則預設 1 讓腳本至少 fire）
            let level: u8 = {
                let heroes = adapter.world.read_storage::<crate::comp::Hero>();
                heroes
                    .get(caster)
                    .and_then(|h| h.ability_levels.get(&skill_id).copied())
                    .map(|lv| lv.max(1) as u8)
                    .unwrap_or(1)
            };

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
                let level_data_json = def
                    .get_level_data(level)
                    .and_then(|ld| serde_json::to_string(ld).ok())
                    .unwrap_or_else(|| "{}".to_string());

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
                    }
                    Ok(_) => {}
                    Err(_) => {
                        log::error!("[scripting] panic in AbilityScript::execute of {}", skill_id);
                    }
                }
            } else {
                log::debug!(
                    "[scripting] SkillCast '{}' has no registered AbilityScript handler",
                    skill_id
                );
            }
        }

        ScriptEvent::AttackHit { attacker, victim } => {
            let victim_handle = WorldAdapter::entity_to_handle(victim);
            with_script(adapter, registry, attacker, |script, handle, world_dyn| {
                script.on_attack_hit(handle, victim_handle, world_dyn);
            });
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
            with_script(adapter, registry, e, |script, handle, world_dyn| {
                script.on_health_gained(handle, amount, world_dyn);
            });
        }

        ScriptEvent::ManaGained { e, amount } => {
            with_script(adapter, registry, e, |script, handle, world_dyn| {
                script.on_mana_gained(handle, amount, world_dyn);
            });
        }

        ScriptEvent::SpentMana { caster, cost, ability_id } => {
            let id_clone = ability_id.clone();
            with_script(adapter, registry, caster, move |script, handle, world_dyn| {
                script.on_spent_mana(handle, cost, (&*id_clone).into(), world_dyn);
            });
        }

        ScriptEvent::HealReceived { target, amount, source } => {
            let source_opt = match source.map(WorldAdapter::entity_to_handle) {
                Some(h) => RSome(h),
                None => RNone,
            };
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
                SkillTarget::Point(x, y) => Target::Point(omb_script_abi::types::Vec2f { x, y }),
                SkillTarget::None => Target::None,
            };
            with_script(adapter, registry, e, move |script, handle, world_dyn| {
                script.on_order(handle, (&*kind_clone).into(), target_abi, world_dyn);
            });
        }
    }
}

/// Look up the `ScriptUnitTag` for an entity, returning its `unit_id`.
fn script_id_of(world: &World, e: Entity) -> Option<String> {
    let tags = world.read_storage::<ScriptUnitTag>();
    tags.get(e).map(|t| t.unit_id.clone())
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
    let Some(uid) = script_id_of(adapter.world, entity) else { return };
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
    use crate::comp::CProperty;
    {
        let mut store = adapter.world.write_storage::<CProperty>();
        if let Some(p) = store.get_mut(victim) {
            p.hp = (p.hp - amount).max(0.0);
            return;
        }
    }
    use crate::comp::Unit;
    let mut store = adapter.world.write_storage::<Unit>();
    if let Some(u) = store.get_mut(victim) {
        u.current_hp = (u.current_hp - amount as i32).max(0);
    }
}
