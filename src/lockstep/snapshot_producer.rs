//! Phase 5.3: deterministic ECS world snapshot for observer rejoin.
//!
//! Produces a compact bincode-serialized subset of the authoritative specs
//! `World` (entity id + pos + vel + facing + hp/mhp + kind tag) along with
//! `master_seed` and `tick` so an observer joining mid-game can bootstrap
//! its sim_runner state then play forward via subsequent TickBatches.
//!
//! Run from the dispatcher tick loop (in `State::tick()`) every
//! `SNAPSHOT_INTERVAL_TICKS` ticks (= 30 s @ 30 Hz dispatcher). The output
//! bytes are stored in the `SnapshotStore` resource; the KCP transport's
//! 0x16 SnapshotResp handler reads from a shared `Arc<Mutex<SnapshotStore>>`.
//!
//! # Schema versioning
//!
//! `WorldSnapshot::schema_version` is pinned at `SCHEMA_VERSION = 1`. The
//! omfx-side LockstepClient checks this against its compiled-in expected
//! version before applying the bytes; mismatches fall back to playing from
//! the current tick without bootstrapping. **Add fields to the end of
//! `EntitySnapshot` only, and bump SCHEMA_VERSION when doing so**, because
//! bincode is position-sensitive.
//!
//! Phase 5.3 ships **server-side write only** — the omfx observer
//! consumer is logged-only for now (deserialize + apply is a Phase 5+
//! followup once observer mode is actually exercised).

use specs::{Join, World, WorldExt};
use serde::{Deserialize, Serialize};

use crate::comp::creep::CProperty;
use crate::comp::facing::Facing;
use crate::comp::hero::Hero;
use crate::comp::phys::{Pos, Vel};
use crate::comp::projectile::Projectile;
use crate::comp::resources::{MasterSeed, Tick};
use crate::comp::tower::Tower;

/// On-wire schema version. Bump when adding/reordering fields in
/// `EntitySnapshot` or `WorldSnapshot`. Clients refuse to apply mismatched
/// versions and fall back to no-bootstrap rejoin.
pub const SCHEMA_VERSION: u32 = 1;

/// Entity type tag — matches the omfx-side `EntityKind` discriminants so
/// observer rejoin can dispatch each entity to the correct sprite/render
/// path without re-querying the script registry. Order pinned for bincode.
#[repr(u8)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum EntityKindTag {
    Other = 0,
    Hero = 1,
    Tower = 2,
    Creep = 3,
    Projectile = 4,
}

/// Per-entity state shipped in a snapshot.
///
/// **Add fields at the end, never reorder existing ones** — bincode is
/// position-sensitive. Bump `SCHEMA_VERSION` on any change.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EntitySnapshot {
    pub id: u32,
    pub pos_x_raw: i64,
    pub pos_y_raw: i64,
    pub vel_x_raw: i64,
    pub vel_y_raw: i64,
    pub facing_ticks: i32,
    pub hp_raw: i64,
    pub mhp_raw: i64,
    pub kind: EntityKindTag,
}

/// Top-level snapshot frame. `master_seed` lets a rejoining observer
/// re-seed its `SimRng` streams to match the authoritative server.
#[derive(Serialize, Deserialize, Debug)]
pub struct WorldSnapshot {
    pub schema_version: u32,
    pub tick: u32,
    pub master_seed: u64,
    pub entities: Vec<EntitySnapshot>,
}

/// Walks the world, collects every entity with `Pos`, classifies it via
/// presence of `Hero` / `Tower` / `Projectile` / `CProperty` storages, and
/// bincode-serializes the result through `omoba_sim::snapshot::serialize`.
///
/// Returns an empty `Vec` on serialization failure; the caller treats that
/// as "no snapshot saved this tick" and the previous (possibly empty) bytes
/// stay in the `SnapshotStore`.
pub fn serialize_snapshot(world: &World) -> Vec<u8> {
    let entities = world.entities();
    let pos_storage = world.read_storage::<Pos>();
    let vel_storage = world.read_storage::<Vel>();
    let facing_storage = world.read_storage::<Facing>();
    let cprop_storage = world.read_storage::<CProperty>();
    let hero_storage = world.read_storage::<Hero>();
    let tower_storage = world.read_storage::<Tower>();
    let proj_storage = world.read_storage::<Projectile>();

    let snapshot_entities: Vec<EntitySnapshot> = (&entities, &pos_storage)
        .join()
        .map(|(e, pos)| {
            let kind = if hero_storage.get(e).is_some() {
                EntityKindTag::Hero
            } else if tower_storage.get(e).is_some() {
                EntityKindTag::Tower
            } else if proj_storage.get(e).is_some() {
                EntityKindTag::Projectile
            } else if cprop_storage.get(e).is_some() {
                EntityKindTag::Creep
            } else {
                EntityKindTag::Other
            };

            let (vel_x_raw, vel_y_raw) = vel_storage
                .get(e)
                .map(|v| (v.0.x.raw(), v.0.y.raw()))
                .unwrap_or((0, 0));
            let facing_ticks = facing_storage
                .get(e)
                .map(|f| f.0.ticks())
                .unwrap_or(0);
            let (hp_raw, mhp_raw) = cprop_storage
                .get(e)
                .map(|c| (c.hp.raw(), c.mhp.raw()))
                .unwrap_or((0, 0));

            EntitySnapshot {
                id: e.id(),
                pos_x_raw: pos.0.x.raw(),
                pos_y_raw: pos.0.y.raw(),
                vel_x_raw,
                vel_y_raw,
                facing_ticks,
                hp_raw,
                mhp_raw,
                kind,
            }
        })
        .collect();

    let snapshot = WorldSnapshot {
        schema_version: SCHEMA_VERSION,
        tick: world.read_resource::<Tick>().0 as u32,
        master_seed: world.read_resource::<MasterSeed>().0,
        entities: snapshot_entities,
    };

    omoba_sim::snapshot::serialize(&snapshot).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use omoba_sim::{Angle, Fixed64, Vec2 as SimVec2};
    use specs::{Builder, World, WorldExt};

    fn make_world() -> World {
        let mut w = World::new();
        w.register::<Pos>();
        w.register::<Vel>();
        w.register::<Facing>();
        w.register::<CProperty>();
        w.register::<Hero>();
        w.register::<Tower>();
        w.register::<Projectile>();
        w.insert(Tick(0));
        w.insert(MasterSeed::default());
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
    fn empty_world_round_trips() {
        let w = make_world();
        let bytes = serialize_snapshot(&w);
        assert!(!bytes.is_empty(), "even an empty world should serialize the WorldSnapshot envelope");
        let snap: WorldSnapshot = omoba_sim::snapshot::deserialize(&bytes)
            .expect("empty world snapshot must deserialize");
        assert_eq!(snap.schema_version, SCHEMA_VERSION);
        assert_eq!(snap.tick, 0);
        assert_eq!(snap.master_seed, MasterSeed::default().0);
        assert_eq!(snap.entities.len(), 0);

        // Re-serializing the deserialized snapshot must produce the same
        // bytes (bincode is canonical for this schema).
        let bytes2 = omoba_sim::snapshot::serialize(&snap).expect("re-serialize");
        assert_eq!(bytes, bytes2, "snapshot bytes must round-trip identically");
    }

    #[test]
    fn three_entities_preserve_count_and_seed() {
        let mut w = make_world();
        // Insert a custom MasterSeed so we can assert it round-trips.
        w.insert(MasterSeed(0x1234_5678_9ABC_DEF0));
        w.insert(Tick(123));

        // Plain entity (kind = Other since no CProperty)
        w.create_entity().with(pos_xy(1, 1)).build();
        // Creep-like (CProperty present, no Hero/Tower/Projectile)
        w.create_entity()
            .with(pos_xy(2, 2))
            .with(Vel(SimVec2 { x: Fixed64::from_i32(1), y: Fixed64::ZERO }))
            .with(Facing(Angle::from_ticks(1024)))
            .with(cprop(50, 100))
            .build();
        // Plain entity 2
        w.create_entity().with(pos_xy(3, 3)).build();

        let bytes = serialize_snapshot(&w);
        let snap: WorldSnapshot = omoba_sim::snapshot::deserialize(&bytes)
            .expect("three-entity snapshot must deserialize");

        assert_eq!(snap.schema_version, SCHEMA_VERSION);
        assert_eq!(snap.tick, 123);
        assert_eq!(snap.master_seed, 0x1234_5678_9ABC_DEF0);
        assert_eq!(snap.entities.len(), 3, "three Pos-bearing entities must survive round-trip");

        // Confirm at least one creep-classified entity has matching
        // hp/vel/facing values (find by kind).
        let creep = snap
            .entities
            .iter()
            .find(|e| e.kind == EntityKindTag::Creep)
            .expect("one Creep-tagged entity expected");
        assert_eq!(creep.hp_raw, Fixed64::from_i32(50).raw());
        assert_eq!(creep.mhp_raw, Fixed64::from_i32(100).raw());
        assert_eq!(creep.facing_ticks, 1024);
    }

    #[test]
    fn schema_version_pinned() {
        // Tripwire: any future change that bumps the on-wire schema must
        // be conscious — clients pin their expected version against this
        // constant. If you bump it, also update the omfx LockstepClient
        // observer-rejoin handler in lockstep_client.rs.
        assert_eq!(SCHEMA_VERSION, 1);
    }
}
