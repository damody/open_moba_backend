
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

#[derive(SystemData)]
pub struct TowerRead<'a> {
    entities: Entities<'a>,
    time: Read<'a, Time>,
    dt: Read<'a, DeltaTime>,
    pos : ReadStorage<'a, Pos>,
    searcher : Read<'a, Searcher>,
    factions: ReadStorage<'a, Faction>,
    turn_speeds: ReadStorage<'a, TurnSpeed>,
    tower_kinds: ReadStorage<'a, TowerKind>,
}

#[derive(SystemData)]
pub struct TowerWrite<'a> {
    outcomes: Write<'a, Vec<Outcome>>,
    towers : WriteStorage<'a, Tower>,
    propertys : WriteStorage<'a, TProperty>,
    tatks : WriteStorage<'a, TAttack>,
    facings: WriteStorage<'a, Facing>,
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
        let dt = tr.dt.0;
        let time1 = Instant::now();
        let tx = tw.mqtx.get(0).cloned();
        let mut outcomes = (
            &tr.entities,
            &mut tw.towers,
            &mut tw.propertys,
            &mut tw.tatks,
            &tr.pos,
            &mut tw.facings,
        )
            .par_join()
            .map_init(
                || {
                    prof_span!(guard, "tower update rayon job");
                    guard
                },
                |_guard, (e, tower, pty, atk, pos, facing)| {
                    let mut outcomes:Vec<Outcome> = Vec::new();
                    if atk.asd_count < atk.asd.val() {
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
                        let size = pty.size*pty.size;
                        for nc in tower.nearby_creeps.iter() {
                            if tower.block_creeps.contains(&nc.ent) {
                                // 已經阻檔了
                            } else {
                                if let Some(p) = tr.pos.get(nc.ent) {
                                    if p.0.distance_squared(pos.0) < size {
                                        tower.block_creeps.push(nc.ent);
                                        outcomes.push(Outcome::CreepStop { source: e, target: nc.ent });
                                    }
                                }
                            }
                        }
                    }
                    if atk.asd_count >= atk.asd.val() {
                        let time2 = Instant::now();
                        let elpsed = time2.duration_since(time1);
                        if elpsed.as_secs_f32() < 0.05 {
                            let search_n = 1.max(pty.mblock).max(6) as usize;
                            let (creeps, near_creeps) =
                                tr.searcher.creep.SearchNN_XY2(pos.0, atk.range.val(), atk.range.val()+30., search_n);

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
                                        tower.nearby_creeps.push(NearbyEnt { ent: c.e, dis: c.dis });
                                    }
                                }
                                // 轉向目標：算出 desired angle，旋轉 facing，只有對齊才能開火
                                let target_entity = hostile_creeps[0].e;
                                let target_pos = tr.pos.get(target_entity).map(|p| p.0).unwrap_or(pos.0);
                                let diff = target_pos - pos.0;
                                if diff.magnitude_squared() > 0.01 {
                                    let desired = diff.y.atan2(diff.x);
                                    let turn = tr.turn_speeds.get(e).map(|t| t.0)
                                        .unwrap_or(std::f32::consts::FRAC_PI_2);
                                    let old_facing = facing.0;
                                    facing.0 = rotate_toward(facing.0, desired, turn * dt);

                                    // 廣播 facing 變化（僅當變化 > 3° 才送）
                                    if let Some(ref t) = tx {
                                        if (facing.0 - old_facing).abs() > 0.05 {
                                            let _ = t.try_send(OutboundMsg::new_s("td/all/res", "entity", "F",
                                                serde_json::json!({"id": e.id(), "facing": facing.0})));
                                        }
                                    }

                                    let kind = tr.tower_kinds.get(e).copied();
                                    let is_tack = matches!(kind, Some(crate::comp::TowerKind::Tack));
                                    let can_fire = is_tack || normalize_angle(desired - facing.0).abs() < MOVE_ANGLE_THRESHOLD;
                                    if can_fire {
                                        atk.asd_count -= atk.asd.val();
                                        if is_tack {
                                            // Tack Shooter：八方向放射針（無 target，飛到 range 邊界）
                                            // 途中第一個打到的敵人消失
                                            let range = atk.range.val();
                                            let shots = kind.map(|k| k.template().projectiles_per_shot as usize).unwrap_or(8);
                                            let count = shots.max(1) as i32;
                                            for i in 0..count {
                                                let angle = std::f32::consts::TAU * (i as f32) / (count as f32);
                                                let dir = Vec2::new(angle.cos(), angle.sin());
                                                let end = pos.0 + dir * range;
                                                outcomes.push(Outcome::ProjectileDirectional {
                                                    pos: pos.0.clone(),
                                                    source: Some(e.clone()),
                                                    end_pos: end,
                                                });
                                            }
                                        } else {
                                            outcomes.push(Outcome::ProjectileLine2 {
                                                pos: pos.0.clone(),
                                                source: Some(e.clone()),
                                                target: Some(target_entity),
                                            });
                                        }
                                    }
                                    // 角度太大 → 繼續轉，本 tick 不開火
                                }
                            } else {
                                if near_creeps.len() == 0 {
                                    atk.asd_count = atk.asd.val() - 0.3 - fastrand::u8(..) as f32 * 0.001;
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


