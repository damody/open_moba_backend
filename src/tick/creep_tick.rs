use rayon::iter::IntoParallelRefIterator;
use specs::{
    shred::{ResourceId, World}, Entities, Join, LazyUpdate, Read, ReadExpect, ReadStorage, SystemData,
    Write, WriteStorage, ParJoin, 
};
use crate::comp::*;
use specs::prelude::ParallelIterator;
use crate::uid::{Uid, UidAllocator};
use specs::saveload::MarkerAllocator;

#[derive(SystemData)]
pub struct CreepRead<'a> {
    entities: Entities<'a>,
    time: Read<'a, Time>,
    dt: Read<'a, DeltaTime>,
    uids: ReadStorage<'a, Uid>,
    creeps : ReadStorage<'a, Creep>,
    pos : ReadStorage<'a, Pos>,
}

#[derive(SystemData)]
pub struct CreepWrite<'a> {
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
            &tr.uids,
            &tr.creeps,
            &tr.pos,
            &tw.cpropertys,
        )
            .par_join()
            .filter(|(e, u, t, p, cp)| true )
            .map_init(
                || {
                    prof_span!(guard, "creep update rayon job");
                    guard
                },
                |_guard, (_, uid, creep, pos, cp)| {
                    let mut outcomes:Vec<Outcome> = Vec::new();
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
