
use instant_distance::Point;
use specs::{
    shred, Entities, Join, LazyUpdate, Read, ReadExpect, ReadStorage,
    Write, WriteStorage, ParJoin, SystemData, World,
};
use crate::comp::*;
use crate::transport::OutboundMsg;
use crossbeam_channel::Sender;
use specs::prelude::ParallelIterator;
use vek::*;
use std::{
    time::{Duration, Instant},
};
use specs::Entity;
use omoba_sim::{Fixed32, Vec2 as SimVec2};

/// MOBA 鏡頭下肉眼無感的 facing 變化量（~15°）。舊值 0.05 (~3°) 造成過多 F event。
const FACING_BROADCAST_THRESHOLD_RAD: f32 = 0.26;

/// Per-entity SimRng op_kind for tower_tick. Phase 1de.2: replaces fastrand for
/// the no-target attack-cooldown jitter. Reordering or reusing this constant
/// across systems would invalidate replay determinism.
const OP_TOWER_NO_TARGET_JITTER: u32 = 11;

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
            serde_json::json!({ "id": id, "facing": facing }),
            ent_x, ent_y,
        )
    }
    #[cfg(not(feature = "kcp"))]
    {
        let _ = (ent_x, ent_y);
        OutboundMsg::new_s("td/all/res", "entity", "F",
            serde_json::json!({ "id": id, "facing": facing }))
    }
}

#[derive(SystemData)]
pub struct TowerRead<'a> {
    entities: Entities<'a>,
    time: Read<'a, Time>,
    dt: Read<'a, DeltaTime>,
    master_seed: Read<'a, MasterSeed>,
    tick: Read<'a, Tick>,
    pos : ReadStorage<'a, Pos>,
    searcher : Read<'a, Searcher>,
    factions: ReadStorage<'a, Faction>,
    turn_speeds: ReadStorage<'a, TurnSpeed>,
    // 有 ScriptUnitTag 的塔由腳本 on_tick 自主決策；tower_tick 只幫忙轉向
    script_tags: ReadStorage<'a, crate::scripting::ScriptUnitTag>,
}

#[derive(SystemData)]
pub struct TowerWrite<'a> {
    outcomes: Write<'a, Vec<Outcome>>,
    towers : WriteStorage<'a, Tower>,
    propertys : WriteStorage<'a, TProperty>,
    tatks : WriteStorage<'a, TAttack>,
    facings: WriteStorage<'a, Facing>,
    facing_bcs: WriteStorage<'a, FacingBroadcast>,
    mqtx: Write<'a, Vec<Sender<OutboundMsg>>>,
}

#[derive(Default)]
pub struct Sys;

impl<'a> System<'a> for Sys {
    type SystemData = (
        TowerRead<'a>,
        TowerWrite<'a>,
    );

    const NAME: &'static str = "tower";

    fn run(_job: &mut Job<Self>, (tr, mut tw): Self::SystemData) {
        let time = tr.time.0;
        // Phase 1c.4: dt is Fixed32 throughout battle tick.
        let dt: Fixed32 = tr.dt.0;
        // Lossy projection retained ONLY for facing-radian arithmetic + Searcher boundary.
        // TODO Phase 1e: drop when Searcher / Facing migrate to Fixed32 / Angle natively.
        let dt_f = dt.to_f32_for_render();
        // Phase 1de.2: SimRng seed inputs hoisted into Copy locals for the par_join closure.
        let master_seed: u64 = tr.master_seed.0;
        let tick: u32 = tr.tick.0 as u32;
        let time1 = Instant::now();
        let tx = tw.mqtx.get(0).cloned();
        let mut outcomes = (
            &tr.entities,
            &mut tw.towers,
            &mut tw.propertys,
            &mut tw.tatks,
            &tr.pos,
            &mut tw.facings,
            &mut tw.facing_bcs,
        )
            .par_join()
            .map_init(
                || {
                    prof_span!(guard, "tower update rayon job");
                    guard
                },
                |_guard, (e, tower, pty, atk, pos, facing, facing_bc)| {
                    let mut outcomes:Vec<Outcome> = Vec::new();
                    // TODO Phase 1[d]: drop f32 boundary projection when tower battle tick goes Fixed32/Angle-native.
                    let (pos_x_f, pos_y_f) = pos.xy_f32();
                    let pos_vek = vek::Vec2::new(pos_x_f, pos_y_f);

                    // 腳本塔：開火/asd_count 由 on_tick 自管；但「找目標 + 轉向」仍由 host 做。
                    // 非腳本塔：host 管全部（累計 asd、找目標、轉向、開火）。
                    let is_scripted = tr.script_tags.get(e).is_some();
                    if !is_scripted && atk.asd_count < atk.asd.val() {
                        atk.asd_count += dt;
                    }
                    if pty.mblock > 0 {
                        // 確認所有檔的怪死了沒
                        let mut rm_ids = vec![];
                        for bc in tower.block_creeps.iter() {
                            if let Some(p) = tr.pos.get(*bc) {
                            } else {
                                rm_ids.push(bc);
                            }
                        }
                        let bc: Vec<Entity> = tower.block_creeps.iter().filter(|e| rm_ids.contains(&e)).map(|e| *e).collect();
                        tower.block_creeps = bc;
                        pty.block = tower.block_creeps.len() as i32;
                    }
                    if pty.mblock > pty.block {
                        // 試試看會不會阻檔
                        let size_sq: Fixed32 = pty.size * pty.size;
                        for nc in tower.nearby_creeps.iter() {
                            if tower.block_creeps.contains(&nc.ent) {
                                // 已經阻檔了
                            } else {
                                if let Some(p) = tr.pos.get(nc.ent) {
                                    // 距離平方 in Fixed32 — 與 size_sq (Fixed32) 直接比較。
                                    let diff = p.0 - pos.0;
                                    if diff.length_squared() < size_sq {
                                        tower.block_creeps.push(nc.ent);
                                        outcomes.push(Outcome::CreepStop { source: e, target: nc.ent });
                                    }
                                }
                            }
                        }
                    }
                    // 找目標 + 轉向：
                    //   - 腳本塔：每 tick 都做（host 負責平滑旋轉、對齊到 script 選的目標）
                    //   - 非腳本塔：asd_count 就緒才做（效能優化）
                    let do_seek = is_scripted || atk.asd_count >= atk.asd.val();
                    if do_seek {
                        let time2 = Instant::now();
                        let elpsed = time2.duration_since(time1);
                        if elpsed.as_secs_f32() < 0.05 {
                            let search_n = 1.max(pty.mblock).max(6) as usize;
                            // TODO Phase 1e: Searcher Fixed32 — drop conversions when search_nn_two_radii goes native.
                            let range_f = atk.range.val().to_f32_for_render();
                            let (creeps, near_creeps) =
                                tr.searcher.creep.search_nn_two_radii(pos_vek, range_f, range_f + 30., search_n);

                            // faction filter：若本塔有 Faction，則只攻擊敵對 creep
                            let my_faction = tr.factions.get(e);
                            let hostile_creeps: Vec<_> = creeps
                                .iter()
                                .filter(|ci| match (my_faction, tr.factions.get(ci.e)) {
                                    (Some(mf), Some(tf)) => mf.is_hostile_to(tf),
                                    // 無 Faction 的塔（玩家建的防禦塔）沿用舊行為，攻擊所有 creep
                                    (None, _) => true,
                                    // 目標無 Faction（舊資料）沿用舊行為
                                    (_, None) => true,
                                })
                                .collect();

                            if !hostile_creeps.is_empty() {
                                if pty.mblock > 0 {
                                    tower.nearby_creeps.clear();
                                    for c in hostile_creeps.iter() {
                                        // TODO Phase 1e: DisIndex.dis is still f32 (Searcher boundary).
                                        let dis_fx = Fixed32::from_raw((c.dis * 1024.0) as i32);
                                        tower.nearby_creeps.push(NearbyEnt { ent: c.e, dis: dis_fx });
                                    }
                                }
                                // 轉向目標：算出 desired angle，旋轉 facing
                                let target_entity = hostile_creeps[0].e;
                                let target_pos = tr.pos.get(target_entity)
                                    .map(|p| { let (x, y) = p.xy_f32(); vek::Vec2::new(x, y) })
                                    .unwrap_or(pos_vek);
                                let diff = target_pos - pos_vek;
                                if diff.magnitude_squared() > 0.01 {
                                    let desired = diff.y.atan2(diff.x);
                                    let turn = tr.turn_speeds.get(e).map(|t| t.0.to_f32_for_render())
                                        .unwrap_or(std::f32::consts::FRAC_PI_2);
                                    let cur_rad = facing.rad_f32();
                                    let new_rad = rotate_toward(cur_rad, desired, turn * dt_f);
                                    *facing = Facing::from_rad_f32(new_rad);

                                    // 廣播 facing 變化：和「上次廣播」差 > 15° 才送。
                                    // 必須比較 last_broadcast 而不是 per-tick old_facing —
                                    // 否則每 tick 旋轉量 (~3°) 永遠 < 15° 永遠不發。
                                    let needs_emit = match facing_bc.0 {
                                        None => true,  // 第一次必發（client 原預設 0 → 校正）
                                        Some(last) => (new_rad - last).abs() > FACING_BROADCAST_THRESHOLD_RAD,
                                    };
                                    if needs_emit {
                                        facing_bc.0 = Some(new_rad);
                                        if let Some(ref t) = tx {
                                            let _ = t.try_send(make_entity_facing(e.id(), new_rad, pos_x_f, pos_y_f));
                                        }
                                    }

                                    // 腳本塔：host 只負責轉向，不自動開火（腳本 on_tick 全權決定）
                                    if is_scripted {
                                        return outcomes;
                                    }

                                    // MOBA 塔：角度對齊就發單體 homing 彈
                                    if normalize_angle(desired - new_rad).abs() < MOVE_ANGLE_THRESHOLD {
                                        atk.asd_count -= atk.asd.val();
                                        outcomes.push(Outcome::ProjectileLine2 {
                                            pos: pos.0,
                                            source: Some(e.clone()),
                                            target: Some(target_entity),
                                        });
                                    }
                                    // 角度太大 → 繼續轉，本 tick 不開火
                                }
                            } else {
                                if !is_scripted && near_creeps.len() == 0 {
                                    // 0.3 ≈ 307/1024 raw; jitter raw ∈ [0, 256) ≈ 0..0.25.
                                    // Phase 1de.2: deterministic per-(tower, tick) jitter via SimRng.
                                    let mut rng = omoba_sim::SimRng::from_master_entity(
                                        master_seed, tick, e.id(), OP_TOWER_NO_TARGET_JITTER,
                                    );
                                    let jitter = Fixed32::from_raw((rng.next_u32() % 256) as i32);
                                    atk.asd_count = atk.asd.val() - Fixed32::from_raw(307) - jitter;
                                }
                            }
                        }
                    }
                    (outcomes)
                },
            )
            .fold(
                || Vec::new(),
                |(mut all_outcomes), (mut outcomes)| {
                    all_outcomes.append(&mut outcomes);
                    all_outcomes
                },
            )
            .reduce(
                || Vec::new(),
                |( mut outcomes_a),
                 ( mut outcomes_b)| {
                    outcomes_a.append(&mut outcomes_b);
                    outcomes_a
                },
            );
        let time2 = Instant::now();
        let elpsed = time2.duration_since(time1);
        //log::info!("tower update1 time {:?}", elpsed);
        tw.outcomes.append(&mut outcomes);
    }
}


