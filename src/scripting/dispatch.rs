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
use omb_script_abi::{
    types::{DamageInfo, DamageKind, EntityHandle, Target},
    world::{GameWorld, GameWorldDyn, GameWorld_TO},
};
use specs::{Entity, World, WorldExt};
use std::panic::{catch_unwind, AssertUnwindSafe};

use super::event::{ScriptEvent, ScriptEventQueue, SkillTarget};
use super::registry::ScriptRegistry;
use super::tag::ScriptUnitTag;
use super::world_adapter::WorldAdapter;

/// Main entry point — call once per tick, AFTER all parallel tick systems
/// have finished and BEFORE `world.maintain()`.
pub fn run_script_dispatch(world: &mut World, registry: &ScriptRegistry, rng_seed: u64) {
    let events = {
        let mut queue = world.write_resource::<ScriptEventQueue>();
        queue.drain()
    };
    if events.is_empty() {
        return;
    }

    // One adapter per tick; RNG is local to this dispatch pass for
    // deterministic replay (seed driven by tick counter upstream).
    let mut adapter = WorldAdapter::new(world, rng_seed);

    for ev in events {
        dispatch_one(&mut adapter, registry, ev);
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
            let target_abi = match target {
                SkillTarget::Entity(e) => Target::Entity(WorldAdapter::entity_to_handle(e)),
                SkillTarget::Point(x, y) => Target::Point(omb_script_abi::types::Vec2f { x, y }),
                SkillTarget::None => Target::None,
            };
            with_script(adapter, registry, caster, move |script, handle, world_dyn| {
                script.on_skill_cast(handle, (&*skill_id).into(), target_abi, world_dyn);
            });
        }

        ScriptEvent::AttackHit { attacker, victim } => {
            let victim_handle = WorldAdapter::entity_to_handle(victim);
            with_script(adapter, registry, attacker, |script, handle, world_dyn| {
                script.on_attack_hit(handle, victim_handle, world_dyn);
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
