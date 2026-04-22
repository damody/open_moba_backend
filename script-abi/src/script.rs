//! `UnitScript` — the sabi_trait each unit type (hero/tower/creep) implements.
//! All hooks have default no-op impls; scripts override only what they need.

use abi_stable::{
    sabi_trait,
    std_types::{ROption, RStr},
};
use crate::types::*;
use crate::world::GameWorldDyn;

#[sabi_trait]
pub trait UnitScript: Send + Sync {
    /// Unit identifier used by host to dispatch (must match `script` field
    /// in the unit's config entry).
    fn unit_id(&self) -> RStr<'_>;

    /// Called once when the entity is spawned.
    #[sabi(last_prefix_field)]
    fn on_spawn(&self, _e: EntityHandle, _w: &mut GameWorldDyn<'_>) {}

    /// Called every tick for entities with `ScriptUnitTag`. Scripts use this
    /// to drive active behaviour (e.g. towers: find target → spawn projectile).
    /// `dt` is the tick delta in seconds.
    fn on_tick(&self, _e: EntityHandle, _dt: f32, _w: &mut GameWorldDyn<'_>) {}

    /// Called when the entity dies. `killer` = the killing entity if known.
    fn on_death(
        &self,
        _e: EntityHandle,
        _killer: ROption<EntityHandle>,
        _w: &mut GameWorldDyn<'_>,
    ) {
    }

    /// Called on the victim before damage is applied. Script may mutate
    /// `info.amount` to implement shields / damage reduction / reflect.
    fn on_damage_taken(
        &self,
        _e: EntityHandle,
        _info: &mut DamageInfo,
        _w: &mut GameWorldDyn<'_>,
    ) {
    }

    /// Called on the attacker after `on_damage_taken` has resolved the
    /// final amount. Useful for lifesteal, on-hit effects.
    fn on_damage_dealt(
        &self,
        _attacker: EntityHandle,
        _victim: EntityHandle,
        _final_amount: f32,
        _w: &mut GameWorldDyn<'_>,
    ) {
    }

    /// Called on the caster when a skill is activated.
    fn on_skill_cast(
        &self,
        _caster: EntityHandle,
        _skill_id: RStr<'_>,
        _target: Target,
        _w: &mut GameWorldDyn<'_>,
    ) {
    }

    /// Called on the attacker at the moment an attack connects.
    /// Tower-style scripts usually live here (splash, pierce, crit).
    fn on_attack_hit(
        &self,
        _attacker: EntityHandle,
        _victim: EntityHandle,
        _w: &mut GameWorldDyn<'_>,
    ) {
    }
}
