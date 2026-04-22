//! `AbilityScript` — sabi_trait each DLL implements to provide a castable
//! ability (active skill, ultimate, toggle, tower attack).
//!
//! Companion to `UnitScript`:
//! - `UnitScript` reacts to unit lifecycle events (spawn, death, hit).
//! - `AbilityScript` is invoked when a specific ability is cast.
//!
//! ## Metadata vs. Logic
//! The stable ABI carries `AbilityDef` as a JSON-encoded `RString`
//! (`AbilityDefFFI::def_json`). This avoids `StableAbi` derives on
//! `HashMap<String, serde_json::Value>` and keeps the ABI simple. Host
//! `deserialize`s into `omoba_core::ability_meta::AbilityDef` once at
//! DLL load time for metadata queries (e.g. client tooltip).
//!
//! `level_data_json` passed to `execute` is the JSON-encoded
//! `AbilityLevelData` for the caster's current level — also avoids
//! StableAbi on the `extra: HashMap<String, Value>` field.

use abi_stable::{
    StableAbi, sabi_trait,
    std_types::{RBox, RResult, RStr, RString},
};
use crate::types::*;
use crate::world::GameWorldDyn;

#[sabi_trait]
pub trait AbilityScript: Send + Sync {
    /// Ability identifier (must match the `id` field in the companion
    /// `AbilityDef`). Used by host to dispatch.
    fn ability_id(&self) -> RStr<'_>;

    /// Execute the ability. Handler applies effects via `world` methods
    /// (`deal_damage`, `add_buff`, `spawn_projectile`, …) directly rather
    /// than returning an effect list — mirrors the `UnitScript` pattern.
    ///
    /// `level_data_json` is `omoba_core::ability_meta::AbilityLevelData`
    /// serialized to JSON. Handler deserializes on entry to read
    /// `cooldown`, `mana_cost`, `range`, `extra[...]`, etc.
    ///
    /// Returns `RErr(msg)` on failure (caller logs); the host still
    /// deducts cooldown/charges on `RErr` only if the handler chooses.
    #[sabi(last_prefix_field)]
    fn execute(
        &self,
        caster: EntityHandle,
        target: Target,
        level: u8,
        level_data_json: RStr<'_>,
        world: &mut GameWorldDyn<'_>,
    ) -> RResult<(), RString>;

    /// Called each host tick while at least one active effect spawned
    /// by this ability is alive. `elapsed` = seconds since the ability
    /// was cast. Default is no-op (most abilities are fire-and-forget).
    fn on_tick(
        &self,
        _caster: EntityHandle,
        _target: Target,
        _elapsed: f32,
        _world: &mut GameWorldDyn<'_>,
    ) {
    }
}

/// `AbilityDef` + the script that implements it — one entry per ability
/// in a DLL. Host registry builds a `HashMap<id, AbilityDefFFI>` at load.
///
/// `def_json` is `omoba_core::ability_meta::AbilityDef` serialized with
/// serde_json. Keeping it as a string avoids dragging serde-json /
/// HashMap through `abi_stable::StableAbi`.
#[repr(C)]
#[derive(StableAbi)]
pub struct AbilityDefFFI {
    pub def_json: RString,
    pub script: AbilityScript_TO<'static, RBox<()>>,
}
