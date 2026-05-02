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
