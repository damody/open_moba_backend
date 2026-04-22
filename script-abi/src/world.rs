//! `GameWorld` — the sabi_trait that gives scripts read/write access to the
//! host's ECS. Host implements this on a `WorldAdapter<'a>` that wraps
//! `&'a mut specs::World`.
//!
//! Methods are non-generic (FFI constraint). Adding a component exposure
//! means adding methods here.

use abi_stable::{
    RMut, sabi_trait,
    std_types::{ROption, RStr, RVec},
};
use crate::types::*;

/// Type alias for the borrowed-mutable dyn form of `GameWorld` — this is
/// what hooks receive. Using this uniformly avoids sprinkling the pointer
/// generic across every hook signature.
pub type GameWorldDyn<'a> = GameWorld_TO<'a, RMut<'a, ()>>;

#[sabi_trait]
pub trait GameWorld: Send {
    // ---- Query ----
    fn get_pos(&self, e: EntityHandle) -> ROption<Vec2f>;
    fn get_hp(&self, e: EntityHandle) -> ROption<f32>;
    fn get_max_hp(&self, e: EntityHandle) -> ROption<f32>;
    fn is_alive(&self, e: EntityHandle) -> bool;
    fn faction_of(&self, e: EntityHandle) -> ROption<RStr<'_>>;
    fn unit_id_of(&self, e: EntityHandle) -> ROption<RStr<'_>>;
    fn query_enemies_in_range(
        &self,
        center: Vec2f,
        radius: f32,
        of: EntityHandle,
    ) -> RVec<EntityHandle>;

    // ---- Mutate ----
    fn set_pos(&mut self, e: EntityHandle, p: Vec2f);
    fn deal_damage(
        &mut self,
        target: EntityHandle,
        amount: f32,
        kind: DamageKind,
        source: ROption<EntityHandle>,
    );
    fn heal(&mut self, target: EntityHandle, amount: f32);
    fn add_buff(&mut self, target: EntityHandle, buff_id: RStr<'_>, duration: f32);
    fn remove_buff(&mut self, target: EntityHandle, buff_id: RStr<'_>);
    fn spawn_projectile(
        &mut self,
        from: Vec2f,
        to: EntityHandle,
        speed: f32,
        dmg: f32,
        owner: EntityHandle,
    ) -> EntityHandle;
    fn despawn(&mut self, e: EntityHandle);

    // ---- Non-state side effects ----
    fn play_vfx(&mut self, id: RStr<'_>, at: Vec2f);
    fn play_sfx(&mut self, id: RStr<'_>, at: Vec2f);

    // ---- Deterministic RNG (host-seeded) ----
    /// Returns uniform float in [0, 1). Deterministic across replays.
    fn rand_f32(&mut self) -> f32;

    // ---- Log (forwarded to host's log4rs) ----
    fn log_info(&self, msg: RStr<'_>);
    fn log_warn(&self, msg: RStr<'_>);
    fn log_error(&self, msg: RStr<'_>);
}
