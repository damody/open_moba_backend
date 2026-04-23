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

#[derive(SystemData)]
pub struct CreepRead<'a> {
    entities: Entities<'a>,
    time: Read<'a, Time>,
    dt: Read<'a, DeltaTime>,
    paths: Read<'a, BTreeMap<String, Path>>,
    check_points : Read<'a, BTreeMap<String, CheckPoint>>,
    cpropertys : ReadStorage<'a, CProperty>,
    turn_speeds: ReadStorage<'a, TurnSpeed>,
    radii: ReadStorage<'a, CollisionRadius>,
    searcher: Read<'a, Searcher>,
    buff_store: Read<'a, crate::ability_runtime::BuffStore>,
}

#[derive(SystemData)]
pub struct CreepWrite<'a> {
    creeps : WriteStorage<'a, Creep>,
    pos : WriteStorage<'a, Pos>,
    facings: WriteStorage<'a, Facing>,
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
        let tx = tw.mqtx.get(0).unwrap().clone();

        let mut outcomes = (
            &tr.entities,
            &mut tw.creeps,
            &mut tw.pos,
            &tr.cpropertys,
            &mut tw.facings,
        )
            .par_join()
            .filter(|(_e, _creep, _p, _cp, _f)| true )
            .map_init(
                || {
                    prof_span!(guard, "creep update rayon job");
                    guard
                },
                |_guard, (e, creep, pos, cp, facing)| {
                    let mut outcomes:Vec<Outcome> = Vec::new();
                    if cp.hp <= 0. {
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
                                    match creep.status {
                                        CreepStatus::PreWalk => {
                                            tx.try_send(OutboundMsg::new_s("td/all/res", "creep", "M", json!({
                                                "id": e.id(),
                                                "x": target_point.x,
                                                "y": target_point.y,
                                                "facing": facing.0,
                                            })));
                                            next_status = CreepStatus::Walk;
                                        }
                                        CreepStatus::Walk => {
                                            // Root / stun：本 tick 完全不前進（閉包提早返回 → 此 creep 本 tick 無 outcomes）
                                            if tr.buff_store.is_rooted(e) {
                                                return outcomes;
                                            }
                                            // Slow buff（Ice 塔命中）：從 BuffStore 讀取
                                            // payload.factor；無則不減速（1.0）
                                            let slow_mult = tr.buff_store
                                                .get(e, "slow")
                                                .and_then(|b| b.payload.get("factor").and_then(|v| v.as_f64()))
                                                .map(|f| (f as f32).clamp(0.01, 1.0))
                                                .unwrap_or(1.0);
                                            let step = cp.msd * slow_mult * dt;
                                            let diff = target_point.sub(&pos.0);
                                            let dist_sq = diff.magnitude_squared();
                                            if dist_sq < 0.01 {
                                                // 已抵達 waypoint
                                                creep.pidx += 1;
                                                if let Some(t) = path.check_points.get(creep.pidx) {
                                                    tx.try_send(OutboundMsg::new_s("td/all/res", "creep", "M", json!({
                                                        "id": e.id(),
                                                        "x": t.pos.x,
                                                        "y": t.pos.y,
                                                        "facing": facing.0,
                                                    })));
                                                }
                                            } else {
                                                // 先轉向目標
                                                let desired = diff.y.atan2(diff.x);
                                                let turn_rate = tr.turn_speeds.get(e)
                                                    .map(|t| t.0)
                                                    .unwrap_or(std::f32::consts::FRAC_PI_2);
                                                let old_facing = facing.0;
                                                facing.0 = rotate_toward(facing.0, desired, turn_rate * dt);
                                                // 面向變化 > 3° 就廣播 F 事件
                                                if (facing.0 - old_facing).abs() > 0.05 {
                                                    tx.try_send(OutboundMsg::new_s("td/all/res", "entity", "F",
                                                        json!({"id": e.id(), "facing": facing.0})));
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
                                                            if let Some(t) = path.check_points.get(creep.pidx) {
                                                                tx.try_send(OutboundMsg::new_s("td/all/res", "creep", "M", json!({
                                                                    "id": e.id(),
                                                                    "x": t.pos.x,
                                                                    "y": t.pos.y,
                                                                    "facing": facing.0,
                                                                })));
                                                            }
                                                        } else {
                                                            blocked = true;
                                                        }
                                                    }
                                                    if blocked {
                                                        // 凍結前端 lerp（action="stall"），避免視覺上穿過其他單位。
                                                        tx.try_send(OutboundMsg::new_s("td/all/res", "creep", "stall", json!({
                                                            "id": e.id(),
                                                            "x": pos.0.x,
                                                            "y": pos.0.y,
                                                            "facing": facing.0,
                                                        })));
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
                |(mut outcomes_a),
                 (mut outcomes_b)| {
                    outcomes_a.append(&mut outcomes_b);
                    outcomes_a
                },
            );
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
