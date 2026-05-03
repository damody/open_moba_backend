//! Phase 3.4: drain `PendingPlayerInputs` each dispatcher tick.
//!
//! The lockstep wire (or omfx sim_runner) writes a fresh map of
//! `player_id → PlayerInput` into the resource on every TickBatch. This
//! system consumes them (clearing the resource so stale inputs don't
//! accumulate) and routes each variant to the appropriate game-side
//! handler.
//!
//! Phase 3.4 routing is intentionally a stub: every variant is logged at
//! `trace` and otherwise dropped. Phase 4 will:
//!   - `MoveTo`         → write `MoveTarget` on the player's hero entity.
//!   - `CastAbility`    → enqueue an ability-script invocation through the
//!                        existing `scripting::dispatch` boundary.
//!   - `TowerPlace` /
//!     `TowerUpgrade*` /
//!     `TowerSell`      → route through `comp::game_processor::GameProcessor`
//!                        which already implements every TD command.
//!   - `ItemUse`        → route through the inventory effect pipeline.
//!
//! The point of doing the consumer in Phase 3.4 (with a stub body) is that
//! the dispatcher always drains the resource so the kcp lockstep wire test
//! in Phase 3.5 doesn't see a leak.

use specs::{Read, Write};

use crate::comp::ecs::{Job, System};
use crate::comp::{CurrentCreepWave, PendingPlayerInputs, Time};
#[cfg(feature = "kcp")]
use crate::comp::PendingTowerSpawnQueue;

#[derive(Default)]
pub struct Sys;

#[cfg(feature = "kcp")]
impl<'a> System<'a> for Sys {
    type SystemData = (
        Write<'a, PendingPlayerInputs>,
        Write<'a, CurrentCreepWave>,
        Read<'a, Time>,
        Write<'a, PendingTowerSpawnQueue>,
    );

    const NAME: &'static str = "player_input";

    fn run(_job: &mut Job<Self>, (mut pending, mut cw, time, mut tower_q): Self::SystemData) {
        if pending.by_player.is_empty() {
            return;
        }
        let target_tick = pending.tick;
        let totaltime = time.0 as f32;
        let drained: Vec<_> = pending.by_player.drain().collect();
        log::trace!(
            "player_input_tick: draining {} inputs for tick {}",
            drained.len(),
            target_tick
        );
        for (player_id, input) in drained {
            route_input(player_id, target_tick, input, &mut cw, totaltime, &mut tower_q);
        }
    }
}

#[cfg(not(feature = "kcp"))]
impl<'a> System<'a> for Sys {
    // Non-kcp builds have an empty marker resource; nothing to drain.
    type SystemData = specs::Read<'a, PendingPlayerInputs>;

    const NAME: &'static str = "player_input";

    fn run(_job: &mut Job<Self>, _: Self::SystemData) {}
}

#[cfg(feature = "kcp")]
fn route_input(
    player_id: u32,
    tick: u32,
    input: crate::lockstep::PlayerInput,
    cw: &mut CurrentCreepWave,
    totaltime: f32,
    tower_q: &mut PendingTowerSpawnQueue,
) {
    use crate::lockstep::PlayerInputEnum;

    match input.action {
        Some(PlayerInputEnum::StartRound(_)) => {
            // TD: client pressed "Start Round". Flip is_running so creep_wave_tick
            // begins emitting the next wave. wave_start_time anchors per-creep
            // delays at the moment the round started.
            if !cw.is_running {
                cw.is_running = true;
                cw.wave_start_time = totaltime;
                log::info!(
                    "player_input_tick: pid={} tick={} StartRound → wave={} start_time={:.2}",
                    player_id, tick, cw.wave, totaltime,
                );
            } else {
                log::warn!(
                    "player_input_tick: pid={} tick={} StartRound ignored (round already running)",
                    player_id, tick,
                );
            }
        }
        Some(PlayerInputEnum::NoOp(_)) => {
            // Ack-only — keepalive heartbeat with no side effects.
        }
        Some(PlayerInputEnum::MoveTo(m)) => {
            let (x, y) = m.target.map(|v| (v.x, v.y)).unwrap_or((0, 0));
            log::trace!(
                "player_input_tick: pid={} tick={} MoveTo target_raw=({}, {})",
                player_id,
                tick,
                x,
                y
            );
            // Phase 4: lookup hero entity by player_id and write MoveTarget.
        }
        Some(PlayerInputEnum::AttackTarget(a)) => {
            log::trace!(
                "player_input_tick: pid={} tick={} AttackTarget target_id={}",
                player_id,
                tick,
                a.target_id
            );
        }
        Some(PlayerInputEnum::CastAbility(c)) => {
            log::trace!(
                "player_input_tick: pid={} tick={} CastAbility ability_index={} target_entity={:?}",
                player_id,
                tick,
                c.ability_index,
                c.target_entity
            );
            // Phase 4: route to scripting::dispatch with the player's hero
            // entity as the caster.
        }
        Some(PlayerInputEnum::TowerPlace(t)) => {
            let pos_raw = t.pos.as_ref();
            let (px, py) = pos_raw.map(|v| (v.x, v.y)).unwrap_or((0, 0));
            log::info!(
                "player_input_tick: pid={} tick={} TowerPlace kind_id={} pos_raw=({}, {})",
                player_id, tick, t.tower_kind_id, px, py,
            );
            // Defer to PendingTowerSpawnQueue: spawn_td_tower needs &mut World
            // (TowerTemplateRegistry lookup + entity creation + ScriptEvent::
            // Spawn push) which a specs `System` can't borrow. The queue is
            // drained right after dispatch on both host and replica via
            // `GameProcessor::drain_pending_tower_spawns`.
            let pos = omoba_sim::Vec2::new(
                omoba_sim::Fixed64::from_raw(px as i64),
                omoba_sim::Fixed64::from_raw(py as i64),
            );
            tower_q.requests.push(crate::comp::PendingTowerSpawn {
                kind_id: t.tower_kind_id,
                pos,
                owner_pid: player_id,
            });
        }
        Some(PlayerInputEnum::TowerUpgrade(u)) => {
            log::trace!(
                "player_input_tick: pid={} tick={} TowerUpgrade tower_entity_id={} path={} level={}",
                player_id,
                tick,
                u.tower_entity_id,
                u.path,
                u.level
            );
        }
        Some(PlayerInputEnum::TowerSell(s)) => {
            log::trace!(
                "player_input_tick: pid={} tick={} TowerSell tower_entity_id={}",
                player_id,
                tick,
                s.tower_entity_id
            );
        }
        Some(PlayerInputEnum::ItemUse(i)) => {
            log::trace!(
                "player_input_tick: pid={} tick={} ItemUse item_slot={}",
                player_id,
                tick,
                i.item_slot
            );
        }
        None => {
            log::warn!(
                "player_input_tick: pid={} tick={} input action is None (malformed proto?)",
                player_id,
                tick
            );
        }
    }
}
