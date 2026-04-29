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

/// MOBA 鏡頭下肉眼無感的 facing 變化量（~15°）。舊值 0.05 (~3°) 造成過多 F event。
const FACING_BROADCAST_THRESHOLD_RAD: f32 = 0.26;

/// Server ticks per second — keep in sync with `omb/src/main.rs` TPS const.
/// Used by creep.M to compute `arrival_tick` for client extrapolation.
const TICK_DT: f32 = 1.0 / 30.0;

/// P4 full builder: emits a creep.M with velocity + arrival_tick + start_pos +
/// start_tick so the client can extrapolate between events (see plan P4.3).
/// Non-kcp builds fall back to the same 4-field legacy JSON as before.
#[inline]
fn make_creep_move_full(
    id: u32,
    target_x: f32,
    target_y: f32,
    facing: f32,
    velocity: f32,
    start_x: f32,
    start_y: f32,
    start_tick: u64,
) -> OutboundMsg {
    #[cfg(feature = "kcp")]
    {
        use crate::state::resource_management::proto_build;
        use crate::transport::TypedOutbound;
        // arrival_tick is computed inside proto_build::creep_move_full; keep
        // the JSON shadow small — omfx reads extrapolation fields only when
        // `velocity` is present. Legacy omfx ignores unknown keys.
        let dx = target_x - start_x;
        let dy = target_y - start_y;
        let dist = (dx * dx + dy * dy).sqrt();
        let arrival_tick = if velocity > f32::EPSILON && dist > f32::EPSILON {
            start_tick + ((dist / velocity / TICK_DT).ceil() as u64)
        } else {
            start_tick
        };
        OutboundMsg::new_typed_at(
            "td/all/res", "creep", "M",
            TypedOutbound::CreepMove(proto_build::creep_move_full(
                id, target_x, target_y, facing, velocity,
                start_x, start_y, start_tick, TICK_DT,
            )),
            json!({
                "id": id,
                "x": target_x, "y": target_y,
                "facing": facing,
                "velocity": velocity,
                "start_pos": { "x": start_x, "y": start_y },
                "start_tick": start_tick,
                "arrival_tick": arrival_tick,
            }),
            start_x, start_y,
        )
    }
    #[cfg(not(feature = "kcp"))]
    {
        let _ = (velocity, start_x, start_y, start_tick);
        OutboundMsg::new_s("td/all/res", "creep", "M",
            json!({ "id": id, "x": target_x, "y": target_y, "facing": facing }))
    }
}

/// Build a creep.stall OutboundMsg (prost CreepStall under kcp).
#[inline]
fn make_creep_stall(id: u32, x: f32, y: f32, facing: f32) -> OutboundMsg {
    #[cfg(feature = "kcp")]
    {
        use crate::state::resource_management::proto_build;
        use crate::transport::TypedOutbound;
        OutboundMsg::new_typed_at(
            "td/all/res", "creep", "stall",
            TypedOutbound::CreepStall(proto_build::creep_stall(id, x, y, facing)),
            json!({ "id": id, "x": x, "y": y, "facing": facing }),
            x, y,
        )
    }
    #[cfg(not(feature = "kcp"))]
    {
        OutboundMsg::new_s("td/all/res", "creep", "stall",
            json!({ "id": id, "x": x, "y": y, "facing": facing }))
    }
}

/// Build an entity.F OutboundMsg (prost EntityFacing under kcp).
#[inline]
fn make_entity_facing(id: u32, facing: f32, ent_x: f32, ent_y: f32) -> OutboundMsg {
    #[cfg(feature = "kcp")]
    {
        use crate::state::resource_management::proto_build;
        use crate::transport::TypedOutbound;
        OutboundMsg::new_typed_at(
            "td/all/res", "entity", "F",
            TypedOutbound::EntityFacing(proto_build::entity_facing(id, facing)),
            json!({ "id": id, "facing": facing }),
            ent_x, ent_y,
        )
    }
    #[cfg(not(feature = "kcp"))]
    {
        let _ = (ent_x, ent_y);
        OutboundMsg::new_s("td/all/res", "entity", "F",
            json!({ "id": id, "facing": facing }))
    }
}

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
        let server_tick = tr.tick.0;
        let tx = tw.mqtx.get(0).unwrap().clone();

        // P4 emit candidates collected from the par_join pass, keyed by entity.
        // Carries current (target, velocity, start_pos, facing) — the gating +
        // record update happens serially below so we can touch mv_broadcasts
        // without fighting borrow rules inside the parallel closure.
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
                    if cp.hp <= 0. {
                        // [DEBUG-STRESS] creep_tick 看到的 hp 值（應該與 handle_damage 寫入後的 hp 一致）
                        log::info!("☠️ creep_tick sees hp<=0: name={} hp={} mhp={} ent={}",
                            creep.name, cp.hp, cp.mhp, e.id());
                        outcomes.push(Outcome::Death { pos: pos.0.clone(), ent: e.clone() });
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
                                    let target_point = p.pos;
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
                                                entity: e, target: target_point,
                                                velocity: effective_msd,
                                                start_pos: pos.0, facing: facing.0,
                                            });
                                            next_status = CreepStatus::Walk;
                                        }
                                        CreepStatus::Walk => {
                                            // Root / stun：本 tick 完全不前進（閉包提早返回 → 此 creep 本 tick 無 outcomes）
                                            if tr.buff_store.is_rooted(e) {
                                                return (outcomes, cands);
                                            }
                                            let step = effective_msd * dt;
                                            let diff = target_point.sub(&pos.0);
                                            let dist_sq = diff.magnitude_squared();
                                            if dist_sq < 0.01 {
                                                // 已抵達 waypoint — pidx advances, new waypoint
                                                // triggers an M candidate (target change).
                                                creep.pidx += 1;
                                                if let Some(t) = path.check_points.get(creep.pidx) {
                                                    cands.push(MoveCandidate {
                                                        entity: e, target: t.pos,
                                                        velocity: effective_msd,
                                                        start_pos: pos.0, facing: facing.0,
                                                    });
                                                }
                                            } else {
                                                // 先轉向目標
                                                let desired = diff.y.atan2(diff.x);
                                                let turn_rate = tr.turn_speeds.get(e)
                                                    .map(|t| t.0)
                                                    .unwrap_or(std::f32::consts::FRAC_PI_2);
                                                facing.0 = rotate_toward(facing.0, desired, turn_rate * dt);
                                                // 廣播 facing 變化：和「上次廣播」差 > 15° 才送。
                                                let needs_emit = match facing_bc.0 {
                                                    None => true,
                                                    Some(last) => (facing.0 - last).abs() > FACING_BROADCAST_THRESHOLD_RAD,
                                                };
                                                if needs_emit {
                                                    facing_bc.0 = Some(facing.0);
                                                    tx.try_send(make_entity_facing(e.id(), facing.0, pos.0.x, pos.0.y));
                                                }

                                                // 角度對齊（<30°）才移動
                                                let angle_diff = normalize_angle(desired - facing.0).abs();
                                                if angle_diff < MOVE_ANGLE_THRESHOLD {
                                                    let radius = tr.radii.get(e).map(|r| r.0).unwrap_or(20.0);
                                                    let self_entity = e;
                                                    let hits = |p: vek::Vec2<f32>| -> bool {
                                                        let q_r = radius + MAX_COLLISION_RADIUS;
                                                        for di in tr.searcher.search_collidable(p, q_r, 16) {
                                                            if di.e == self_entity { continue; }
                                                            let Some(other_r) = tr.radii.get(di.e).map(|cr| cr.0) else { continue };
                                                            let touch = radius + other_r;
                                                            if di.dis < touch * touch {
                                                                return true;
                                                            }
                                                        }
                                                        false
                                                    };
                                                    // 記錄本 tick 是否因為碰撞而停住，若是則廣播 M(current_pos)
                                                    // 讓前端 lerp 停下來，避免視覺上穿過其他單位。
                                                    let mut blocked = false;
                                                    if dist_sq > step * step {
                                                        let mut v = diff;
                                                        v.normalize();
                                                        v = v * step;
                                                        let full = pos.0 + v;
                                                        if !hits(full) {
                                                            pos.0 = full;
                                                        } else {
                                                            let only_x = pos.0 + vek::Vec2::new(v.x, 0.0);
                                                            let only_y = pos.0 + vek::Vec2::new(0.0, v.y);
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
                                                                    velocity: effective_msd,
                                                                    start_pos: pos.0, facing: facing.0,
                                                                });
                                                            }
                                                        } else {
                                                            blocked = true;
                                                        }
                                                    }
                                                    if blocked {
                                                        // 凍結前端 lerp（action="stall"），避免視覺上穿過其他單位。
                                                        tx.try_send(make_creep_stall(e.id(), pos.0.x, pos.0.y, facing.0));
                                                    } else {
                                                        // Not a waypoint advance, not blocked — but
                                                        // still consider emitting if velocity changed
                                                        // (slow applied/removed). Gating pass below
                                                        // compares to last broadcast and drops if same.
                                                        cands.push(MoveCandidate {
                                                            entity: e, target: target_point,
                                                            velocity: effective_msd,
                                                            start_pos: pos.0, facing: facing.0,
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

            // Fire the event with full extrapolation fields.
            let _ = tx.try_send(make_creep_move_full(
                cand.entity.id(),
                cand.target.x, cand.target.y,
                cand.facing,
                cand.velocity,
                cand.start_pos.x, cand.start_pos.y,
                server_tick,
            ));

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
                
                let phys_damage = (td.phys - cp.def_physic).max(0.);
                let magi_damage = (td.magi - cp.def_magic).max(0.);
                let total_damage = phys_damage + magi_damage;
                
                // 獲取目標名稱用於日誌
                let target_name = if let Some(creep) = tw.creeps.get(td.ent) {
                    creep.name.clone()
                } else {
                    // 暫時使用實體 ID，因為沒有在 Read 結構中包含 Hero
                    format!("Entity({:?})", td.ent.id())
                };
                
                if total_damage > 0.0 {
                    // 獲取目標位置
                    let target_pos = tw.pos.get(td.ent)
                        .map(|pos| pos.0)
                        .unwrap_or(vek::Vec2::new(0.0, 0.0));
                    
                    // 生成傷害事件（日誌將在 state.rs 中統一處理）
                    tw.outcomes.push(Outcome::Damage {
                        pos: target_pos,
                        phys: phys_damage,
                        magi: magi_damage,
                        real: 0.0,
                        source: td.source, // 使用正確的攻擊者
                        target: td.ent,
                        predeclared: false, // melee / on-touch damage — never pre-declared
                    });
                } else if td.phys > 0.0 || td.magi > 0.0 {
                    // 只有在有原始傷害但被完全防禦時才顯示
                    log::info!("🛡️ {} | Damage BLOCKED: Phys {:.1} vs Def {:.1}, Magi {:.1} vs Def {:.1}", 
                        target_name,
                        td.phys, cp.def_physic,
                        td.magi, cp.def_magic
                    );
                }
            }
        } 
        tw.taken_damages.clear();
    }
}
