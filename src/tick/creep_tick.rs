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
}

#[derive(SystemData)]
pub struct CreepWrite<'a> {
    creeps : WriteStorage<'a, Creep>,
    pos : WriteStorage<'a, Pos>,
    cpropertys : WriteStorage<'a, CProperty>,
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
            &mut tw.cpropertys,
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
        // 傷害計算
        let mut hpmap = BTreeMap::new();
        for td in tw.taken_damages.iter() {
            if let Some(cp) = tw.cpropertys.get_mut(td.ent) {
                cp.hp -= (td.phys - cp.def_physic).max(0.);
                cp.hp -= (td.magi - cp.def_magic).max(0.);
                cp.hp = cp.hp.max(0.);
                hpmap.insert(td.ent.id(), cp.hp);
            }
        }
        for (e, hp) in hpmap.iter() {
            tx.try_send(MqttMsg::new_s("td/all/res", "creep", "Hp", json!({
                "id": e,
                "hp": hp,
            })));
        } 
        tw.taken_damages.clear();
    }
}
