use rayon::iter::IntoParallelRefIterator;
use specs::{
    shred::{ResourceId, World}, Entities, Join, LazyUpdate, Read, ReadExpect, ReadStorage, SystemData,
    Write, WriteStorage, ParJoin, 
};
use std::{thread, ops::Deref, collections::BTreeMap};
use std::ops::Sub;
use crate::comp::*;
use crate::uid::{Uid, UidAllocator};
use specs::prelude::ParallelIterator;
use specs::saveload::MarkerAllocator;

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
        let mut outcomes = (
            &tr.entities,
            &mut tw.creeps,
            &mut tw.pos,
            &mut tw.cpropertys,
        )
            .par_join()
            .filter(|(e, t, p, cp)| true )
            .map_init(
                || {
                    prof_span!(guard, "creep update rayon job");
                    guard
                },
                |_guard, (e, creep, pos, cp)| {
                    let mut outcomes:Vec<Outcome> = Vec::new();
                    if let Some(path) = tr.paths.get(&creep.path) {
                        if let Some(p) = path.check_points.get(creep.pidx) {
                            let target_point = p.pos;
                            if target_point.distance_squared(pos.0) > (cp.msd*cp.msd) {
                                let mut v = target_point.sub(&pos.0);
                                v.normalize();
                                v = v * cp.msd * dt;
                                pos.0 = pos.0 + v;
                            } else {
                                pos.0 = target_point;
                                creep.pidx += 1;
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
        for td in tw.taken_damages.iter() {
            if let Some(cp) = tw.cpropertys.get_mut(td.ent) {
                cp.hp -= (td.phys - cp.def_physic).max(0.);
                cp.hp -= (td.magi - cp.def_magic).max(0.);
                cp.hp = cp.hp.max(0.);
            }
        }
        tw.taken_damages.clear();
    }
}
