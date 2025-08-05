use rayon::iter::IntoParallelRefIterator;
use specs::{
    shred::{ResourceId, World}, Entities, Join, LazyUpdate, Read, ReadExpect, ReadStorage, SystemData,
    Write, WriteStorage, ParJoin, 
};
use std::{thread, ops::Deref, collections::BTreeMap};
use std::ops::Sub;
use crate::comp::*;
use specs::prelude::ParallelIterator;
use specs::saveload::MarkerAllocator;
use crate::MqttMsg;
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
}

#[derive(SystemData)]
pub struct CreepWrite<'a> {
    creeps : WriteStorage<'a, Creep>,
    pos : WriteStorage<'a, Pos>,
    outcomes: Write<'a, Vec<Outcome>>,
    taken_damages: Write<'a, Vec<TakenDamage>>,
    mqtx: Write<'a, Vec<Sender<MqttMsg>>>,
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
        )
            .par_join()
            .filter(|(e, creep, p, cp)| true )
            .map_init(
                || {
                    prof_span!(guard, "creep update rayon job");
                    guard
                },
                |_guard, (e, creep, pos, cp)| {
                    let mut outcomes:Vec<Outcome> = Vec::new();
                    if cp.hp <= 0. {
                        outcomes.push(Outcome::Death { pos: pos.0.clone(), ent: e.clone() });
                    } else {
                        if let Some(path) = tr.paths.get(&creep.path) {
                            if let Some(b) = creep.block_tower {
                                // 被檔住了
                            } else {
                                if let Some(p) = path.check_points.get(creep.pidx) {
                                    let target_point = p.pos;
                                    let mut next_status = creep.status.clone();
                                    match creep.status {
                                        CreepStatus::PreWalk => {
                                            tx.try_send(MqttMsg::new_s("td/all/res", "creep", "M", json!({
                                                "id": e.id(),
                                                "x": target_point.x,
                                                "y": target_point.y,
                                            })));
                                            next_status = CreepStatus::Walk;
                                        }
                                        CreepStatus::Walk => {
                                            if target_point.distance_squared(pos.0) > (cp.msd*cp.msd) {
                                                let mut v = target_point.sub(&pos.0);
                                                v.normalize();
                                                v = v * cp.msd * dt;
                                                pos.0 = pos.0 + v;
                                            } else {
                                                pos.0 = target_point;
                                                creep.pidx += 1;
                                                if let Some(t) = path.check_points.get(creep.pidx) {
                                                    tx.try_send(MqttMsg::new_s("td/all/res", "creep", "M", json!({
                                                        "id": e.id(),
                                                        "x": t.pos.x,
                                                        "y": t.pos.y,
                                                    })));
                                                }
                                            }
                                        }
                                        CreepStatus::Stop => {
                                            next_status = CreepStatus::PreWalk;
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
