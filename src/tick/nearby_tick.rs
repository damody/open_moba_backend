use std::collections::hash_map;

use hashbrown::HashMap;
use specs::{
    shred::{ResourceId, World}, Entities, Join, LazyUpdate, Read, ReadExpect, ReadStorage, SystemData,
    Write, WriteStorage, ParJoin, Entity as EcsEntity,
};
use crate::comp::*;
use crate::comp::phys::*;
use specs::prelude::ParallelIterator;
use instant_distance::{Builder, Search, HnswMap};

#[derive(SystemData)]
pub struct NearbyRead<'a> {
    entities: Entities<'a>,
    uids: ReadStorage<'a, Uid>,
    creeps : ReadStorage<'a, Creep>,
    pos : ReadStorage<'a, Pos>,
}

#[derive(SystemData)]
pub struct NearbyWrite<'a> {
    entities: Entities<'a>,
    hmap: Write<'a, Vec<HnswMap<Pos, EcsEntity>>>,
    towers : WriteStorage<'a, Tower>,
    pos : ReadStorage<'a, Pos>,
}

#[derive(Default)]
pub struct Sys;

impl<'a> System<'a> for Sys {
    type SystemData = (
        NearbyRead<'a>,
        NearbyWrite<'a>,
    );

    const NAME: &'static str = "nearby";

    fn run(_job: &mut Job<Self>, (tr, mut tw): Self::SystemData) {
        let (ents, pos) = (
            &tr.entities,
            &tr.pos,
            &tr.creeps,
        )
            .par_join()
            .map_init(
                || {
                    prof_span!(guard, "nearby update rayon job");
                    guard
                },
                |_guard, (ent, pos,_)| {
                    (vec![ent], vec![*pos])
                },
            )
            .fold(
                || (Vec::new(), Vec::new()),
                |(mut ents, mut pos), (mut u, mut p)| {
                    ents.append(&mut u);
                    pos.append(&mut p);
                    (ents, pos)
                },
            )
            .reduce(
                || (Vec::new(), Vec::new()),
                |(mut ents, mut pos), (mut u, mut p)| {
                    ents.append(&mut u);
                    pos.append(&mut p);
                    (ents, pos)
                },
            );
            
        let map = Builder::default().build(pos, ents);
        (
            &tr.entities,
            &tr.pos,
            &mut tw.towers,
        )
            .par_join()
            .for_each_init(
                || {
                    prof_span!(guard, "nearby update rayon job");
                    guard
                },
                |_guard, (ent, pos, tower)| {
                    tower.nearby_creeps.clear();
                },
            );
        (
            &tr.entities,
            &tr.pos,
            &mut tw.towers,
        )
            .par_join()
            .for_each_init(
                || {
                    prof_span!(guard, "nearby update rayon job");
                    guard
                },
                |_guard, (ent, pos, tower)| {
                    let mut search = Search::default();
                    let closest_point = map.search(&pos, &mut search).next();
                    if let Some(c) = closest_point {
                        tower.nearby_creeps.push(*c.value);
                    }
                },
            );
        tw.hmap.clear();
        tw.hmap.push(map);
    }
}
