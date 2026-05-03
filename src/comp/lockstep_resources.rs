//! Phase 3.4: ECS resources owned by the lockstep input pipeline.
//!
//! Sits in `comp::` (not `lockstep::`) because:
//!   1. The host omb dispatcher consumes them every tick, regardless of
//!      whether the kcp lockstep transport is active.
//!   2. The omfx sim_runner worker thread also writes to them (after
//!      converting `TickBatch` proto inputs to host `PlayerInput` types),
//!      so they must live in a module that's compiled in *all* feature
//!      configurations. `lockstep::` is gated behind `#[cfg(feature = "kcp")]`.
//!
//! `PlayerInput` (the prost-generated proto type re-exported from
//! `lockstep::PlayerInput`) is only available under `feature = "kcp"`. To
//! keep this module always-compiled, we store inputs as opaque
//! `serde_json::Value` here is a non-starter (lossy + serializing an empty
//! oneof is awkward). Instead the resource holds a feature-gated typed
//! payload and the consumer system also feature-gates. Non-kcp builds get
//! an empty resource that nothing writes to or reads from.

#[cfg(feature = "kcp")]
use std::collections::HashMap;

#[cfg(feature = "kcp")]
use crate::lockstep::PlayerInput;

use omoba_sim::Vec2 as SimVec2;

/// Per-tick collection of player inputs decoded from the most recent
/// `TickBatch`. Cleared by the consumer system every tick. `tick` records
/// the lockstep tick number these inputs target — currently used only
/// for diagnostic logging / desync tracing.
#[cfg(feature = "kcp")]
#[derive(Default)]
pub struct PendingPlayerInputs {
    /// `player_id → PlayerInput` for the current lockstep tick. Each
    /// `TickBatch` write replaces this map wholesale (one input per player
    /// per tick is the lockstep contract).
    pub by_player: HashMap<u32, PlayerInput>,
    /// Lockstep tick that the inputs target. The consumer system uses this
    /// for log context only — the actual side effects target whatever tick
    /// the dispatcher is currently running.
    pub tick: u32,
}

/// Non-kcp build: empty marker so dispatcher / system code can read/write
/// the resource without a compile-time feature gate everywhere.
#[cfg(not(feature = "kcp"))]
#[derive(Default)]
pub struct PendingPlayerInputs;

/// Phase 2.1: deferred tower-spawn requests originating from
/// `PlayerInputEnum::TowerPlace`. The lockstep `player_input_tick::Sys` only
/// has SystemData access (no `&mut World`), but `spawn_td_tower` needs
/// `&mut World` to query `TowerTemplateRegistry` + create entities + push
/// `ScriptEvent::Spawn`. So we queue here and drain in `tick()` (host) /
/// `sim_runner` (replica) right after the dispatcher runs.
///
/// Invariant: must be drained every tick on both host and replica so the two
/// sims stay deterministically equivalent. `comp::GameProcessor::
/// drain_pending_tower_spawns` is the single shared drain entry point.
#[derive(Default)]
pub struct PendingTowerSpawnQueue {
    pub requests: Vec<PendingTowerSpawn>,
}

#[derive(Clone, Debug)]
pub struct PendingTowerSpawn {
    pub kind_id: u32,
    pub pos: SimVec2,
    pub owner_pid: u32,
}

/// Phase 5.3: latest serialized world snapshot for observer rejoin.
///
/// Updated every `SNAPSHOT_INTERVAL_TICKS` dispatcher ticks (= 30 s @ 30 Hz).
/// Used by the KCP transport's 0x16 SnapshotResp handler to bootstrap
/// observer clients connecting mid-game; the observer applies the bytes to
/// its sim_runner then plays forward via subsequent TickBatches.
///
/// `bytes` is bincode-serialized via `omoba_sim::snapshot::serialize` over a
/// stable subset of components (`id` + `pos` + `vel` + `facing` + `hp`/`mhp`
/// + `kind`). Schema is pinned via `WorldSnapshot::schema_version` inside
/// `lockstep::snapshot_producer` — bumping the on-wire format requires
/// coordinating both ends.
///
/// Empty bytes (`tick = 0`) means no snapshot has been saved yet; KCP handler
/// returns it as-is and the observer falls back to playing from `current_tick`.
#[derive(Default)]
pub struct SnapshotStore {
    /// Tick the snapshot was captured at. `0` = no snapshot yet.
    pub tick: u32,
    /// bincode-serialized `WorldSnapshot` (entities + Pos / Vel / Facing /
    /// CProperty subset + master_seed + tick + schema_version).
    pub bytes: Vec<u8>,
}
