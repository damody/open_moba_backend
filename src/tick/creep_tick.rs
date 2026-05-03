use rayon::iter::IntoParallelRefIterator;
use specs::{
    shred, Entities, Join, LazyUpdate, Read, ReadExpect, ReadStorage, SystemData,
    Write, WriteStorage, ParJoin, World,
};
use std::{thread, ops::Deref, collections::BTreeMap};
use std::ops::Sub;
use crate::comp::*;
use crate::comp::phys::MAX_COLLISION_RADIUS;
use specs::prelude::ParallelIterator;
use crate::transport::OutboundMsg;
use crossbeam_channel::Sender;
use serde_json::json;
use omoba_sim::{Fixed64, Vec2 as SimVec2, Angle};
use omoba_sim::trig::{angle_rotate_toward, atan2 as sim_atan2, fixed_rad_to_ticks, TAU_TICKS};

/// MOBA 鏡頭下肉眼無感的 facing 變化量（~15°）。舊值 0.05 (~3°) 造成過多 F event。
const FACING_BROADCAST_THRESHOLD_RAD: f32 = 0.26;

#[derive(SystemData)]
pub struct CreepRead<'a> {
    entities: Entities<'a>,
    time: Read<'a, Time>,
    dt: Read<'a, DeltaTime>,
    /// P4: server tick counter; used as `start_tick` in creep.M for client
    /// extrapolation anchor.
    tick: Read<'a, Tick>,
    paths: Read<'a, BTreeMap<String, Path>>,
    check_points : Read<'a, BTreeMap<String, CheckPoint>>,
    cpropertys : ReadStorage<'a, CProperty>,
    turn_speeds: ReadStorage<'a, TurnSpeed>,
    radii: ReadStorage<'a, CollisionRadius>,
    searcher: Read<'a, Searcher>,
    buff_store: Read<'a, crate::ability_runtime::BuffStore>,
    is_buildings: ReadStorage<'a, IsBuilding>,
}

#[derive(SystemData)]
pub struct CreepWrite<'a> {
    creeps : WriteStorage<'a, Creep>,
    pos : WriteStorage<'a, Pos>,
    facings: WriteStorage<'a, Facing>,
    facing_bcs: WriteStorage<'a, FacingBroadcast>,
    /// P4: per-creep last-broadcast snapshot for M-emit gating.
    /// Inserted lazily on first emit (component may be absent for creeps
    /// that existed before the P4 upgrade path).
    mv_broadcasts: WriteStorage<'a, CreepMoveBroadcast>,
    outcomes: Write<'a, Vec<Outcome>>,
    taken_damages: Write<'a, Vec<TakenDamage>>,
    mqtx: Write<'a, Vec<Sender<OutboundMsg>>>,
}

#[derive(Default)]
pub struct Sys;

impl<'a> System<'a> for Sys {
    type SystemData = (
        CreepRead<'a>,
        CreepWrite<'a>,
    );

    const NAME: &'static str = "creep";

    fn run(_job: &mut Job<Self>, (tr, mut tw): Self::SystemData) {
        let time = tr.time.0;
        let dt = tr.dt.0;
        // Legacy f32 view of dt for CProperty.msd (still f32; Phase 1c).
        let dt_f = dt.to_f32_for_render();
        let server_tick = tr.tick.0;
        // omfx sim_runner does not wire a transport; fall back to a sink sender
        // so broadcast sites silently no-op (try_send returns disconnected, ignored).
        let tx = tw.mqtx.get(0).cloned().unwrap_or_else(|| {
            let (tx, _rx) = crossbeam_channel::unbounded::<OutboundMsg>();
            tx
        });

        // P4 emit candidates collected from the par_join pass, keyed by entity.
        // Carries current (target, velocity, start_pos, facing) — the gating +
        // record update happens serially below so we can touch mv_broadcasts
        // without fighting borrow rules inside the parallel closure.
        // take Fixed64 payloads — redesign in Phase 2 KCP tag rework.
        struct MoveCandidate {
            entity: specs::Entity,
            target: vek::Vec2<f32>,
            velocity: f32,
            start_pos: vek::Vec2<f32>,
            facing: f32,
        }

        let (mut outcomes, move_candidates) = (
            &tr.entities,
            &mut tw.creeps,
            &mut tw.pos,
            &tr.cpropertys,
            &mut tw.facings,
            &mut tw.facing_bcs,
        )
            .par_join()
            .filter(|(_e, _creep, _p, _cp, _f, _fb)| true )
            .map_init(
                || {
                    prof_span!(guard, "creep update rayon job");
                    guard
                },
                |_guard, (e, creep, pos, cp, facing, facing_bc)| {
                    let mut outcomes:Vec<Outcome> = Vec::new();
                    let mut cands: Vec<MoveCandidate> = Vec::new();
                    // Inline boundary helpers — must be called with explicit `&*pos` /
                    // `&*facing` to avoid capturing the storage refs as borrows that
                    // would block subsequent mutations of pos.0 / facing.0.
                    #[inline(always)]
                    fn p_to_f(p: SimVec2) -> vek::Vec2<f32> {
                        vek::Vec2::new(p.x.to_f32_for_render(), p.y.to_f32_for_render())
                    }
                    #[inline(always)]
                    fn a_to_rad(a: Angle) -> f32 {
                        (a.ticks() as f32 / TAU_TICKS as f32) * std::f32::consts::TAU
                    }

                    if cp.hp <= Fixed64::ZERO {
                        // [DEBUG-STRESS] creep_tick 看到的 hp 值（應該與 handle_damage 寫入後的 hp 一致）
                        log::info!("☠️ creep_tick sees hp<=0: name={} hp={:.1} mhp={:.1} ent={}",
                            creep.name,
                            cp.hp.to_f32_for_render(),
                            cp.mhp.to_f32_for_render(),
                            e.id());
                        outcomes.push(Outcome::Death { pos: pos.0, ent: e.clone() });
                    } else {
                        if let Some(path) = tr.paths.get(&creep.path) {
                            if let Some(_b) = creep.block_tower {
                                // 被檔住了
                            } else if creep.pidx >= path.check_points.len() {
                                // TD 模式：走到 path 終點 → 漏怪事件（GameProcessor 扣 PlayerLives）
                                // 設定 status=Leaked 避免下個 tick 重複觸發
                                if !matches!(creep.status, CreepStatus::Leaked) {
                                    outcomes.push(Outcome::CreepLeaked { ent: e.clone() });
                                    creep.status = CreepStatus::Leaked;
                                }
                            } else {
                                if let Some(p) = path.check_points.get(creep.pidx) {
                                    // CheckPoint.pos is still vek::Vec2<f32> (Phase 1c will migrate
                                    // path data to Fixed64). Bridge once per iteration.
                                    let target_point_f: vek::Vec2<f32> = p.pos;
                                    let target_point: SimVec2 = SimVec2::new(
                                        Fixed64::from_raw((target_point_f.x * omoba_sim::fixed::SCALE as f32) as i64),
                                        Fixed64::from_raw((target_point_f.y * omoba_sim::fixed::SCALE as f32) as i64),
                                    );
                                    let mut next_status = creep.status.clone();
                                    // P4: compute effective move speed once per tick — shared
                                    // between the movement step and the M emit candidate.
                                    let stats = crate::ability_runtime::UnitStats::from_refs(
                                        &*tr.buff_store,
                                        tr.is_buildings.get(e).is_some(),
                                    );
                                    let effective_msd = stats.final_move_speed(cp.msd, e);
                                    match creep.status {
                                        CreepStatus::PreWalk => {
                                            // First emit on spawn / PreWalk → unconditional candidate.
                                            cands.push(MoveCandidate {
                                                entity: e, target: target_point_f,
                                                velocity: effective_msd.to_f32_for_render(),
                                                start_pos: p_to_f(pos.0), facing: a_to_rad(facing.0),
                                            });
                                            next_status = CreepStatus::Walk;
                                        }
                                        CreepStatus::Walk => {
                                            // Root / stun：本 tick 完全不前進（閉包提早返回 → 此 creep 本 tick 無 outcomes）
                                            if tr.buff_store.is_rooted(e) {
                                                return (outcomes, cands);
                                            }
                                            // Phase 1c.3: effective_msd is Fixed64 (UnitStats migrated).
                                            // step = effective_msd × dt (Fixed64 × Fixed64).
                                            let step = effective_msd * dt;
                                            let diff = target_point - pos.0;
                                            let dist_sq = diff.length_squared();
                                            // 0.01 in Fixed64 raw = round(0.01 * 1024) = 10
                                            let arrived_eps_sq = Fixed64::from_raw(10);
                                            if dist_sq < arrived_eps_sq {
                                                // 已抵達 waypoint — pidx advances, new waypoint
                                                // triggers an M candidate (target change).
                                                creep.pidx += 1;
                                                if let Some(t) = path.check_points.get(creep.pidx) {
                                                    cands.push(MoveCandidate {
                                                        entity: e, target: t.pos,
                                                        velocity: effective_msd.to_f32_for_render(),
                                                        start_pos: p_to_f(pos.0), facing: a_to_rad(facing.0),
                                                    });
                                                }
                                            } else {
                                                // 先轉向目標
                                                let desired_angle: Angle = sim_atan2(diff.y, diff.x);
                                                let turn_rate = tr.turn_speeds.get(e)
                                                    .map(|t| t.0)
                                                    .unwrap_or(Fixed64::from_raw(1608)); // π/2 rad/s default
                                                let max_step_ticks = fixed_rad_to_ticks(turn_rate * dt);
                                                facing.0 = angle_rotate_toward(facing.0, desired_angle, max_step_ticks);
                                                let new_facing_rad = a_to_rad(facing.0);
                                                // 廣播 facing 變化：和「上次廣播」差 > 15° 才送。
                                                let needs_emit = match facing_bc.0 {
                                                    None => true,
                                                    Some(last) => (new_facing_rad - last).abs() > FACING_BROADCAST_THRESHOLD_RAD,
                                                };
                                                if needs_emit {
                                                    facing_bc.0 = Some(new_facing_rad);
                                                }

                                                // 角度對齊（<30°）才移動 — Angle ticks comparison.
                                                let diff_ticks = (desired_angle.ticks() - facing.0.ticks()).rem_euclid(TAU_TICKS);
                                                let signed_diff_ticks = if diff_ticks > TAU_TICKS / 2 {
                                                    diff_ticks - TAU_TICKS
                                                } else {
                                                    diff_ticks
                                                };
                                                if signed_diff_ticks.abs() < MOVE_ANGLE_THRESHOLD_TICKS {
                                                    let radius = tr.radii.get(e).map(|r| r.0).unwrap_or(Fixed64::from_i32(20));
                                                    let self_entity = e;
                                                    // NOTE: Searcher uses f32 internally for instant_distance lib compat.
                                                    // Final distance check in caller is Fixed64.
                                                    let radius_f = radius.to_f32_for_render();
                                                    let hits = |p_sim: SimVec2| -> bool {
                                                        let q_r = radius_f + MAX_COLLISION_RADIUS;
                                                        let p_vek = vek::Vec2::new(
                                                            p_sim.x.to_f32_for_render(),
                                                            p_sim.y.to_f32_for_render(),
                                                        );
                                                        for di in tr.searcher.search_collidable(p_vek, q_r, 16) {
                                                            if di.e == self_entity { continue; }
                                                            let Some(other_r) = tr.radii.get(di.e).map(|cr| cr.0) else { continue };
                                                            let touch = radius + other_r;
                                                            let touch_f = touch.to_f32_for_render();
                                                            if di.dis < touch_f * touch_f {
                                                                return true;
                                                            }
                                                        }
                                                        false
                                                    };
                                                    // 記錄本 tick 是否因為碰撞而停住
                                                    let mut blocked = false;
                                                    if dist_sq > step * step {
                                                        let v = diff.normalized() * step;
                                                        let full = pos.0 + v;
                                                        if !hits(full) {
                                                            pos.0 = full;
                                                        } else {
                                                            let only_x = SimVec2::new(pos.0.x + v.x, pos.0.y);
                                                            let only_y = SimVec2::new(pos.0.x, pos.0.y + v.y);
                                                            if !hits(only_x) {
                                                                pos.0 = only_x;
                                                            } else if !hits(only_y) {
                                                                pos.0 = only_y;
                                                            } else {
                                                                blocked = true;
                                                            }
                                                        }
                                                    } else {
                                                        if !hits(target_point) {
                                                            pos.0 = target_point;
                                                            creep.pidx += 1;
                                                            // Reached waypoint mid-step: advance and
                                                            // emit M for the NEXT waypoint (target change).
                                                            if let Some(t) = path.check_points.get(creep.pidx) {
                                                                cands.push(MoveCandidate {
                                                                    entity: e, target: t.pos,
                                                                    velocity: effective_msd.to_f32_for_render(),
                                                                    start_pos: p_to_f(pos.0), facing: a_to_rad(facing.0),
                                                                });
                                                            }
                                                        } else {
                                                            blocked = true;
                                                        }
                                                    }
                                                    if blocked {
                                                        // 凍結前端 lerp（action="stall"），避免視覺上穿過其他單位。
                                                    } else {
                                                        // Not a waypoint advance, not blocked — but
                                                        // still consider emitting if velocity changed
                                                        // (slow applied/removed). Gating pass below
                                                        // compares to last broadcast and drops if same.
                                                        cands.push(MoveCandidate {
                                                            entity: e, target: target_point_f,
                                                            velocity: effective_msd.to_f32_for_render(),
                                                            start_pos: p_to_f(pos.0), facing: a_to_rad(facing.0),
                                                        });
                                                    }
                                                }
                                                // 角度太大：只轉向、本 tick 不位移
                                            }
                                        }
                                        CreepStatus::Stop => {
                                            next_status = CreepStatus::PreWalk;
                                        }
                                        CreepStatus::Leaked => {
                                            // 不該發生（Leaked 狀態在外層已被 pidx>=len 分支攔下）
                                        }
                                    }
                                    creep.status = next_status;
                                } else {
                                    // creep 到終點了
                                    outcomes.push(Outcome::Death { pos: pos.0, ent: e });
                                }
                            }
                        }
                    }
                    (outcomes, cands)
                },
            )
            .fold(
                || (Vec::new(), Vec::<MoveCandidate>::new()),
                |(mut all_outcomes, mut all_cands), (mut outcomes, mut cands)| {
                    all_outcomes.append(&mut outcomes);
                    all_cands.append(&mut cands);
                    (all_outcomes, all_cands)
                },
            )
            .reduce(
                || (Vec::new(), Vec::<MoveCandidate>::new()),
                |(mut outcomes_a, mut cands_a),
                 (mut outcomes_b, mut cands_b)| {
                    outcomes_a.append(&mut outcomes_b);
                    cands_a.append(&mut cands_b);
                    (outcomes_a, cands_a)
                },
            );

        // P4 serial emit-gating pass: compare each candidate against the
        // entity's last-broadcast snapshot (CreepMoveBroadcast component).
        // Emit creep.M only if target diverged OR velocity changed > 5% /
        // > 1.0 absolute OR the entity has no prior snapshot. Update the
        // component after emit so next tick's compare uses the new baseline.
        for cand in move_candidates.into_iter() {
            let need_emit = match tw.mv_broadcasts.get(cand.entity) {
                Some(bcast) => bcast.should_emit(cand.target, cand.velocity),
                None => true, // first-ever candidate for this entity
            };
            if !need_emit { continue; }

            // Phase 5.2: legacy 0x02 GameEvent producer cut. Lockstep TickBatch
            // (0x10) carries authoritative pos; client renders from sim.

            // Update (or insert) the broadcast snapshot so subsequent ticks
            // compare against fresh baseline. specs::WriteStorage::insert
            // returns Err only on invalid entity — safe to ignore.
            let mut snap = tw.mv_broadcasts.get(cand.entity).cloned().unwrap_or_default();
            snap.record(cand.target, cand.velocity, server_tick);
            let _ = tw.mv_broadcasts.insert(cand.entity, snap);
        }

        tw.outcomes.append(&mut outcomes);
        // 傷害計算 - 改為生成 Damage 事件
        for td in tw.taken_damages.iter() {
            if let Some(cp) = tr.cpropertys.get(td.ent) {
                // 記錄攻擊前狀態
                let hp_before = cp.hp;
                let max_hp = cp.mhp;

                // Phase 1c.4: cp.* / td.* / Outcome::Damage.{phys,magi,real} 全 Fixed64。
                let phys_raw = td.phys - cp.def_physic;
                let phys_damage: Fixed64 = if phys_raw < Fixed64::ZERO { Fixed64::ZERO } else { phys_raw };
                let magi_raw = td.magi - cp.def_magic;
                let magi_damage: Fixed64 = if magi_raw < Fixed64::ZERO { Fixed64::ZERO } else { magi_raw };
                let total_damage: Fixed64 = phys_damage + magi_damage;

                // 獲取目標名稱用於日誌
                let target_name = if let Some(creep) = tw.creeps.get(td.ent) {
                    creep.name.clone()
                } else {
                    // 暫時使用實體 ID，因為沒有在 Read 結構中包含 Hero
                    format!("Entity({:?})", td.ent.id())
                };

                if total_damage > Fixed64::ZERO {
                    // Phase 1c.4: Outcome::Damage.pos is SimVec2 (Phase 1c.2).
                    let target_pos = tw.pos.get(td.ent)
                        .map(|p| p.0)
                        .unwrap_or(SimVec2::ZERO);

                    // 生成傷害事件（日誌將在 state.rs 中統一處理）
                    tw.outcomes.push(Outcome::Damage {
                        pos: target_pos,
                        phys: phys_damage,
                        magi: magi_damage,
                        real: Fixed64::ZERO,
                        source: td.source, // 使用正確的攻擊者
                        target: td.ent,
                        predeclared: false, // melee / on-touch damage — never pre-declared
                    });
                } else if td.phys > Fixed64::ZERO || td.magi > Fixed64::ZERO {
                    // 只有在有原始傷害但被完全防禦時才顯示
                    log::info!("🛡️ {} | Damage BLOCKED: Phys {:.1} vs Def {:.1}, Magi {:.1} vs Def {:.1}",
                        target_name,
                        td.phys.to_f32_for_render(), cp.def_physic.to_f32_for_render(),
                        td.magi.to_f32_for_render(), cp.def_magic.to_f32_for_render()
                    );
                }
            }
        }
        tw.taken_damages.clear();
    }
}
