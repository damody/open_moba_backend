//! P4: per-creep broadcast gating state for creep.M velocity extrapolation.
//!
//! Before P4 the server emitted `creep.M` every time a waypoint / direction /
//! collision advanced the tick — with 1000 creeps × 30 TPS that's a potential
//! 30K msg/sec worst case. The client already lerps between events, so per-tick
//! emits are strictly wasted bandwidth.
//!
//! P4 gates emission to "state changes only":
//!   1. Target waypoint changes (new checkpoint, path recalc, hit obstacle).
//!   2. Velocity changes by > 5% (Ice slow applied / removed).
//!   3. Entity first enters visible world (initial M is handled by its
//!      CreepStatus::PreWalk transition, same site as before).
//!
//! The comparison state lives here as a specs `Component`; each creep_tick
//! iteration reads the last-broadcast snapshot and decides whether to emit.

use specs::{Component, storage::VecStorage};

/// Snapshot of the fields most recently included in a `creep.M` broadcast
/// for this entity. Compared per-tick against the current target/velocity
/// to decide whether a new M event is needed.
///
/// `last_broadcast_*` are `Option<_>` so the very first emit is unconditional
/// (the entity has no prior snapshot). After the first emit the component is
/// populated with `Some(_)` and subsequent checks compare against the stored
/// values.
#[derive(Clone, Debug, Default)]
pub struct CreepMoveBroadcast {
    /// Last broadcast target waypoint (world units). `None` = never broadcast.
    pub last_target: Option<vek::Vec2<f32>>,
    /// Last broadcast velocity (world units per second).
    pub last_velocity: Option<f32>,
    /// Server tick when the last broadcast was emitted.
    pub last_start_tick: Option<u64>,
}

impl Component for CreepMoveBroadcast {
    type Storage = VecStorage<Self>;
}

impl CreepMoveBroadcast {
    /// Decide whether a new creep.M event should be emitted given the
    /// current tick's target + velocity.
    ///
    /// Rules:
    /// - No prior broadcast → always emit.
    /// - Target diverges by > 0.25 world unit (= 1 quantization step).
    /// - Velocity diverges by > 5% (relative) or > 1.0 absolute — covers
    ///   Ice slow apply (e.g. 200 → 140 = 30%) and slow-expire snap-back.
    pub fn should_emit(&self, target: vek::Vec2<f32>, velocity: f32) -> bool {
        // First emit: no prior state.
        let Some(prev_target) = self.last_target else { return true };
        let Some(prev_vel) = self.last_velocity else { return true };

        // Target changed: compare squared distance to skip sqrt.
        let dx = target.x - prev_target.x;
        let dy = target.y - prev_target.y;
        let dist_sq = dx * dx + dy * dy;
        // 0.25 world unit = Position16 quantization precision; below that
        // the wire value is identical anyway so there's nothing to send.
        if dist_sq > 0.25 * 0.25 {
            return true;
        }

        // Velocity change: > 5% relative OR > 1.0 absolute.
        // Relative handles slow/unslow (e.g. Ice 200→140); absolute handles
        // low-speed creeps where 5% is sub-pixel noise.
        let vel_diff = (velocity - prev_vel).abs();
        if vel_diff > 1.0 {
            return true;
        }
        if prev_vel.abs() > f32::EPSILON && vel_diff / prev_vel.abs() > 0.05 {
            return true;
        }

        false
    }

    /// Record that an emit just happened with the given fields.
    pub fn record(&mut self, target: vek::Vec2<f32>, velocity: f32, start_tick: u64) {
        self.last_target = Some(target);
        self.last_velocity = Some(velocity);
        self.last_start_tick = Some(start_tick);
    }
}
