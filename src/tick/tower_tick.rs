
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
#[derive(SystemData)]
pub struct TowerRead<'a> {
    entities: Entities<'a>,
    time: Read<'a, Time>,
    dt: Read<'a, DeltaTime>,
    towers : ReadStorage<'a, Tower>,
    pos : ReadStorage<'a, Pos>,
    searcher : Read<'a, Searcher>,
}

#[derive(SystemData)]
pub struct TowerWrite<'a> {
    outcomes: Write<'a, Vec<Outcome>>,
    propertys : WriteStorage<'a, TProperty>,
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
            &tr.towers,
            &mut tw.propertys,
            &tr.pos,
        )
            .par_join()
            .map_init(
                || {
                    prof_span!(guard, "tower update rayon job");
                    guard
                },
                |_guard, (e, tower, property, pos)| {
                    let mut outcomes:Vec<Outcome> = Vec::new();
                    if property.asd_count < property.asd {
                        property.asd_count += dt;
                    }
                    if property.asd_count >= property.asd {
                        let time2 = Instant::now();
                        let elpsed = time2.duration_since(time1);
                        if elpsed.as_secs_f32() < 0.05 {
                            let (creeps, near_creeps) = tr.searcher.creep.SearchNN_XY2(pos.0, property.range, property.range+30., 1);
                            if creeps.len() > 0 {
                                property.asd_count -= property.asd;
                                outcomes.push(Outcome::ProjectileLine2 { pos: pos.0.clone(), source: Some(e.clone()), target: Some(creeps[0].e) });
                            } else {
                                if near_creeps.len() == 0 {
                                    property.asd_count = property.asd - 0.3 - fastrand::u8(..) as f32 * 0.001;
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
        log::info!("tower update1 time {:?}", elpsed);
        tw.outcomes.append(&mut outcomes);
    }
}


