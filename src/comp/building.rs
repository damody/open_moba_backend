//! `IsBuilding` — ZST marker component for non-mobile structures (towers,
//! future barracks / wards / traps).
//!
//! Used by `UnitStats` to skip movespeed / respawn / vision / illusion /
//! bounty modifier aggregation — buildings don't move and don't respawn.
//! Checking `has IsBuilding` is the canonical way to ask "is this a building?"
//! instead of peeking at `Tower` / future specific components.

use specs::storage::NullStorage;
use specs::Component;

#[derive(Default, Debug, Clone, Copy)]
pub struct IsBuilding;

impl Component for IsBuilding {
    type Storage = NullStorage<Self>;
}
