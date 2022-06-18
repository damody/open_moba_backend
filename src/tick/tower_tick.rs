
use instant_distance::Point;
use specs::{
    shred::{ResourceId, World}, Entities, Join, LazyUpdate, Read, ReadExpect, ReadStorage, SystemData,
    Write, WriteStorage, ParJoin, 
};
use crate::comp::*;
use specs::prelude::ParallelIterator;
use vek::*;
use std::{
    time::{Duration, Instant},
};
use specs::Entity as EcsEntity;

#[derive(SystemData)]
pub struct TowerRead<'a> {
    entities: Entities<'a>,
    time: Read<'a, Time>,
    dt: Read<'a, DeltaTime>,
    pos : ReadStorage<'a, Pos>,
    searcher : Read<'a, Searcher>,
}

#[derive(SystemData)]
pub struct TowerWrite<'a> {
    outcomes: Write<'a, Vec<Outcome>>,
    towers : WriteStorage<'a, Tower>,
    propertys : WriteStorage<'a, TProperty>,
    tatks : WriteStorage<'a, TAttack>,
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
        let mut outcomes = (
            &tr.entities,
            &mut tw.towers,
            &mut tw.propertys,
            &mut tw.tatks,
            &tr.pos,
        )
            .par_join()
            .map_init(
                || {
                    prof_span!(guard, "tower update rayon job");
                    guard
                },
                |_guard, (e, tower, pty, atk, pos)| {
                    let mut outcomes:Vec<Outcome> = Vec::new();
                    if atk.asd_count < atk.asd {
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
                        let bc: Vec<EcsEntity> = tower.block_creeps.iter().filter(|e| rm_ids.contains(&e)).map(|e| *e).collect();
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
                    if atk.asd_count >= atk.asd {
                        let time2 = Instant::now();
                        let elpsed = time2.duration_since(time1);
                        if elpsed.as_secs_f32() < 0.05 {
                            let search_n = 1.max(pty.mblock) as usize;
                            let (creeps, near_creeps) = 
                                tr.searcher.creep.SearchNN_XY2(pos.0, atk.range, atk.range+30., search_n);
                            if creeps.len() > 0 {
                                // 如果需要阻檔的話才要記錄最近的單位
                                if pty.mblock > 0 {
                                    tower.nearby_creeps.clear();
                                    for e in creeps.iter() {
                                        tower.nearby_creeps.push(NearbyEnt { ent: e.e, dis: e.dis });
                                    }
                                }
                                atk.asd_count -= atk.asd;
                                outcomes.push(Outcome::ProjectileLine2 { pos: pos.0.clone(), source: Some(e.clone()), target: Some(creeps[0].e) });
                            } else {
                                if near_creeps.len() == 0 {
                                    atk.asd_count = atk.asd - 0.3 - fastrand::u8(..) as f32 * 0.001;
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


