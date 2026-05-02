//! Phase 3.4: deterministic ECS state-hash producer.
//!
//! Walks the authoritative specs `World` and feeds a stable subset of entity
//! state (entity id + `Pos.x/y` raw + `CProperty.hp` raw) through
//! `omoba_sim::state_hash::hash_sorted_by_id` to produce a single u64 used
//! by lockstep clients for desync detection.
//!
//! # Why not done inside `TickBroadcaster`?
//!
//! `TickBroadcaster::run()` is a tokio task; the specs `World` is `!Send`
//! (some script-related storages hold non-`Send` interiors). Producer runs
//! in `State::tick()` (the dispatcher thread), publishes the hash through
//! a `crossbeam_channel`, and the broadcaster `try_recv`s the latest value
//! at its 60Hz cadence.
//!
//! # Determinism contract
//!
//! - `Pos.0.x.raw()` / `Pos.0.y.raw()` are `i64` Q53.10 fixed-point — the
//!   same representation server and clients run.
//! - `CProperty.hp.raw()` is the same `Fixed64` raw `i64` (clients without
//!   a `CProperty` get `0` so towers / projectiles don't drift hash output
//!   between machines that store them differently).
//! - The sort step inside `hash_sorted_by_id` makes the hash invariant of
//!   ECS storage / join order — only state values matter.
//!
//! Phase 3.4 hashes only `Pos` + `hp`. Phase 4+ may add `Facing`, `Vel`,
//! ability cooldowns, etc. — but adding fields breaks pinning so should be
//! done in a single migration.

use specs::{Join, World, WorldExt};

use omoba_sim::state_hash::hash_sorted_by_id;

use crate::comp::creep::CProperty;
use crate::comp::facing::Facing;
use crate::comp::phys::{Pos, Vel};

/// Stable subset hashed per state-hash tick. `#[derive(Hash)]` order matches
/// field declaration order; do not rearrange without bumping the protocol
/// version (clients compare against this exact byte sequence).
///
/// Phase 4 widens from `(id, pos.x, pos.y, hp)` to add velocity + facing
/// (ticks). BuffStore aggregations are still excluded — they're a Resource
/// not a per-entity Component, and the BuffStore wire payload migration is
/// scheduled for Phase 4d (76 PHASE 2 marker cleanup).
#[derive(std::hash::Hash)]
struct HashItem {
    id: u32,
    pos_x_raw: i64,
    pos_y_raw: i64,
    vel_x_raw: i64,
    vel_y_raw: i64,
    facing_ticks: i32,
    hp_raw: i64,
}

/// Computes a deterministic state hash over every entity that has a `Pos`
/// component. Entities without `Vel` / `Facing` / `CProperty` substitute zeros
/// so their absence/presence (e.g. towers vs creeps) doesn't shift the hash
/// for cosmetic-only differences.
pub fn compute_state_hash(world: &World) -> u64 {
    let entities = world.entities();
    let pos_storage = world.read_storage::<Pos>();
    let vel_storage = world.read_storage::<Vel>();
    let facing_storage = world.read_storage::<Facing>();
    let cprop_storage = world.read_storage::<CProperty>();

    let items: Vec<HashItem> = (&entities, &pos_storage)
        .join()
        .map(|(e, pos)| {
            let (vel_x_raw, vel_y_raw) = vel_storage
                .get(e)
                .map(|v| (v.0.x.raw(), v.0.y.raw()))
                .unwrap_or((0, 0));
            let facing_ticks = facing_storage
                .get(e)
                .map(|f| f.0.ticks())
                .unwrap_or(0);
            HashItem {
                id: e.id(),
                pos_x_raw: pos.0.x.raw(),
                pos_y_raw: pos.0.y.raw(),
                vel_x_raw,
                vel_y_raw,
                facing_ticks,
                hp_raw: cprop_storage.get(e).map(|c| c.hp.raw()).unwrap_or(0),
            }
        })
        .collect();

    hash_sorted_by_id(&items, |i| i.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use omoba_sim::{Fixed64, Vec2 as SimVec2};
    use specs::{Builder, World, WorldExt};

    fn make_world() -> World {
        let mut w = World::new();
        w.register::<Pos>();
        w.register::<Vel>();
        w.register::<Facing>();
        w.register::<CProperty>();
        w
    }

    fn cprop(hp: i32, mhp: i32) -> CProperty {
        CProperty {
            hp: Fixed64::from_i32(hp),
            mhp: Fixed64::from_i32(mhp),
            msd: Fixed64::ZERO,
            def_physic: Fixed64::ZERO,
            def_magic: Fixed64::ZERO,
        }
    }

    fn pos_xy(x: i32, y: i32) -> Pos {
        Pos(SimVec2 {
            x: Fixed64::from_i32(x),
            y: Fixed64::from_i32(y),
        })
    }

    #[test]
    fn empty_world_hashes_deterministically() {
        let w1 = make_world();
        let w2 = make_world();
        assert_eq!(compute_state_hash(&w1), compute_state_hash(&w2));
    }

    #[test]
    fn pos_change_changes_hash() {
        let mut w1 = make_world();
        w1.create_entity().with(pos_xy(10, 20)).build();
        let h1 = compute_state_hash(&w1);

        let mut w2 = make_world();
        w2.create_entity().with(pos_xy(11, 20)).build();
        let h2 = compute_state_hash(&w2);

        assert_ne!(h1, h2, "moving an entity must change the hash");
    }

    #[test]
    fn hp_change_changes_hash() {
        let mut w1 = make_world();
        w1.create_entity().with(pos_xy(0, 0)).with(cprop(100, 100)).build();
        let h1 = compute_state_hash(&w1);

        let mut w2 = make_world();
        w2.create_entity().with(pos_xy(0, 0)).with(cprop(99, 100)).build();
        let h2 = compute_state_hash(&w2);

        assert_ne!(h1, h2, "HP change must affect hash");
    }

    #[test]
    fn vel_change_changes_hash() {
        let mut w1 = make_world();
        w1.create_entity()
            .with(pos_xy(0, 0))
            .with(Vel(SimVec2 { x: Fixed64::from_i32(1), y: Fixed64::ZERO }))
            .build();
        let h1 = compute_state_hash(&w1);

        let mut w2 = make_world();
        w2.create_entity()
            .with(pos_xy(0, 0))
            .with(Vel(SimVec2 { x: Fixed64::from_i32(2), y: Fixed64::ZERO }))
            .build();
        let h2 = compute_state_hash(&w2);

        assert_ne!(h1, h2, "Vel change must affect hash");
    }

    #[test]
    fn facing_change_changes_hash() {
        use omoba_sim::Angle;
        let mut w1 = make_world();
        w1.create_entity()
            .with(pos_xy(0, 0))
            .with(Facing(Angle::from_ticks(0)))
            .build();
        let h1 = compute_state_hash(&w1);

        let mut w2 = make_world();
        w2.create_entity()
            .with(pos_xy(0, 0))
            .with(Facing(Angle::from_ticks(1024)))
            .build();
        let h2 = compute_state_hash(&w2);

        assert_ne!(h1, h2, "Facing change must affect hash");
    }

    #[test]
    fn missing_cproperty_uses_zero() {
        // Two worlds, one with hp=0 explicit, one without CProperty: should
        // produce the same hash because we substitute hp_raw=0 for missing.
        let mut w1 = make_world();
        w1.create_entity().with(pos_xy(5, 5)).build();
        let h1 = compute_state_hash(&w1);

        let mut w2 = make_world();
        w2.create_entity().with(pos_xy(5, 5)).with(cprop(0, 0)).build();
        let h2 = compute_state_hash(&w2);

        assert_eq!(h1, h2, "no-CProperty must hash same as hp=0");
    }
}
