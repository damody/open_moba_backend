
use specs::{
    shred::{ResourceId, World}, Entities, Join, LazyUpdate, Read, ReadExpect, ReadStorage, SystemData,
    Write, WriteStorage, ParJoin, 
};
use crate::comp::*;
use specs::prelude::ParallelIterator;
use vek::*;

#[derive(SystemData)]
pub struct TowerRead<'a> {
    entities: Entities<'a>,
    time: Read<'a, Time>,
    dt: Read<'a, DeltaTime>,
    uids: ReadStorage<'a, Uid>,
    towers : ReadStorage<'a, Tower>,
    pos : ReadStorage<'a, Pos>,
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
    const PHASE: Phase = Phase::Apply;

    fn run(_job: &mut Job<Self>, (tr, mut tw): Self::SystemData) {
        let time = tr.time.0;
        let dt = tr.dt.0;
        let mut outcomes = (
            &tr.entities,
            &tr.uids,
            &tr.towers,
            &mut tw.propertys,
            &tr.pos,
        )
            .par_join()
            .filter(|(e, u, t, pp, p)| t.lv > 0 )
            .map_init(
                || {
                    prof_span!(guard, "tower update rayon job");
                    guard
                },
                |_guard, (_, uid, tower, property, pos)| {
                    let mut outcomes:Vec<Outcome> = Vec::new();
                    if property.asd_count < property.asd {
                        property.asd_count += dt;
                    }
                    if property.asd_count >= property.asd && tower.nearby_creeps.len() > 0 {
                        property.asd_count -= property.asd;
                        outcomes.push(Outcome::ProjectileLine2 { pos: pos.0.clone(), source: Some(uid.clone()), target: Some(tower.nearby_creeps[0]) });
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
        tw.outcomes.append(&mut outcomes);
    }
}


