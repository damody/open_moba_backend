use specs::{
    shred::{ResourceId, World}, Entities, Join, LazyUpdate, Read, ReadExpect, ReadStorage, SystemData,
    Write, WriteStorage, ParJoin, 
};
use crate::comp::*;
use specs::prelude::ParallelIterator;
use instant_distance::{Builder, Search, HnswMap};
use specs::saveload::MarkerAllocator;
use specs::Entity as EcsEntity;

#[derive(SystemData)]
pub struct ProjectileRead<'a> {
    entities: Entities<'a>,
    time: Read<'a, Time>,
    dt: Read<'a, DeltaTime>,
    vel : ReadStorage<'a, Vel>,
    hmap: Read<'a, Vec<HnswMap<Pos, EcsEntity>>>,
}

#[derive(SystemData)]
pub struct ProjectileWrite<'a> {
    pos : WriteStorage<'a, Pos>,
    projs : WriteStorage<'a, Projectile>,
    outcomes: Write<'a, Vec<Outcome>>,
    taken_damages: Write<'a, Vec<TakenDamage>>,
}

#[derive(Default)]
pub struct Sys;

impl<'a> System<'a> for Sys {
    type SystemData = (
        ProjectileRead<'a>,
        ProjectileWrite<'a>,
    );

    const NAME: &'static str = "projectile";

    fn run(_job: &mut Job<Self>, (tr, mut tw): Self::SystemData) {
        let time = tr.time.0;
        let dt = tr.dt.0;
        //log::info!("projs count {}", tw.projs.count());
        let (mut outcomes, mut taken_damages) = (
            &tr.entities,
            &mut tw.projs,
            &tr.vel,
            &mut tw.pos,
        )
            .par_join()
            .filter(|(e, proj, v, p)| proj.time_left > 0.)
            .map_init(
                || {
                    prof_span!(guard, "projectile update rayon job");
                    guard
                },
                |_guard, (e, proj, vel, pos)| {
                    let mut outcomes:Vec<Outcome> = Vec::new();
                    let mut taken_damages:Vec<TakenDamage> = Vec::new();
                    pos.0 += vel.0 * dt;
                    proj.time_left -= dt;
                    let mut search = Search::default();
                    if let Some(map) = tr.hmap.first() {
                        let closest_point = map.search(&pos, &mut search).next();
                        if let Some(c) = closest_point {
                            if c.distance < 50. {
                                taken_damages.push(TakenDamage{ent: c.value.clone(), phys:5., magi:3., real:0. });
                                outcomes.push(Outcome::Death { pos: pos.0.clone(), ent: e.clone() });
                            }
                        }
                    }
                    (outcomes, taken_damages)
                },
            )
            .fold(
                || (Vec::new(), Vec::new()),
                |(mut all_outcomes, mut all_taken_damages), 
                    (mut outcomes, mut taken_damages)| {
                    all_outcomes.append(&mut outcomes);
                    all_taken_damages.append(&mut taken_damages);
                    (all_outcomes, all_taken_damages)
                },
            )
            .reduce(
                || (Vec::new(), Vec::new()),
                |( mut outcomes_a, mut taken_damages_a),
                 ( mut outcomes_b, mut taken_damages_b)| {
                    outcomes_a.append(&mut outcomes_b);
                    taken_damages_a.append(&mut taken_damages_b);
                    (outcomes_a, taken_damages_a)
                },
            );
        tw.taken_damages.append(&mut taken_damages);
        tw.outcomes.append(&mut outcomes);
    }
}
