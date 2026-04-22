//! Stable-ABI value types that cross the host/DLL boundary.

use abi_stable::{StableAbi, std_types::ROption};

/// Opaque handle to a game entity. Host converts to/from `specs::Entity`.
#[repr(C)]
#[derive(StableAbi, Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct EntityHandle {
    pub id: u32,
    pub gen: u32,
}

impl EntityHandle {
    pub const INVALID: Self = Self { id: u32::MAX, gen: 0 };
    pub fn is_valid(&self) -> bool { self.id != u32::MAX }
}

#[repr(C)]
#[derive(StableAbi, Copy, Clone, Debug, Default)]
pub struct Vec2f {
    pub x: f32,
    pub y: f32,
}

impl Vec2f {
    pub const fn new(x: f32, y: f32) -> Self { Self { x, y } }
}

#[repr(u8)]
#[derive(StableAbi, Copy, Clone, Debug, PartialEq, Eq)]
pub enum DamageKind {
    Physical,
    Magical,
    Pure,
}

/// Passed to `on_damage_taken` as `&mut` — scripts may modify `amount`
/// (e.g. shield, damage reduction, reflect).
#[repr(C)]
#[derive(StableAbi, Clone, Debug)]
pub struct DamageInfo {
    pub attacker: ROption<EntityHandle>,
    pub amount: f32,
    pub kind: DamageKind,
}

#[repr(C)]
#[derive(StableAbi, Clone, Debug)]
pub enum Target {
    Entity(EntityHandle),
    Point(Vec2f),
    None,
}
