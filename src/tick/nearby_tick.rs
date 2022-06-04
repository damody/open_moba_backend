use std::collections::hash_map;

use hashbrown::HashMap;
use specs::{
    shred::{ResourceId, World}, Entities, Join, LazyUpdate, Read, ReadExpect, ReadStorage, SystemData,
    Write, WriteStorage, ParJoin, Entity as EcsEntity,
};
use crate::comp::*;
use crate::comp::phys::*;
use specs::prelude::ParallelIterator;
use std::{
    time::{Duration, Instant},
};
use voracious_radix_sort::{RadixSort};

#[derive(SystemData)]
pub struct NearbyRead<'a> {
    entities: Entities<'a>,
    time: Read<'a, Time>,
    creeps : ReadStorage<'a, Creep>,
    pos : ReadStorage<'a, Pos>,
}

#[derive(SystemData)]
pub struct NearbyWrite<'a> {
    entities: Entities<'a>,
    towers : WriteStorage<'a, Tower>,
    searcher: Write<'a, Searcher>,
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
        {//creep update
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
            tw.searcher.creep.xpos.clear();
            tw.searcher.creep.ypos.clear();
            for (i, p) in pos.iter().enumerate() {
                tw.searcher.creep.xpos.push(PosXIndex { e: ents[i], p: p.0.clone() });
                tw.searcher.creep.ypos.push(PosYIndex { e: ents[i], p: p.0.clone() });
            }
            tw.searcher.creep.xpos.voracious_mt_sort(4);
            tw.searcher.creep.ypos.voracious_mt_sort(4);
        }
        if tw.searcher.tower.needsort {
            let (ents, pos) = (
                &tr.entities,
                &tr.pos,
                &tw.towers,
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
            if tw.searcher.tower.needsort {
                tw.searcher.tower.needsort = false;
                let time1 = Instant::now();
                tw.searcher.tower.xpos.clear();
                tw.searcher.tower.ypos.clear();
                for (i, p) in pos.iter().enumerate() {
                    tw.searcher.tower.xpos.push(PosXIndex { e: ents[i], p: p.0.clone() });
                    tw.searcher.tower.ypos.push(PosYIndex { e: ents[i], p: p.0.clone() });
                }
                tw.searcher.tower.xpos.voracious_mt_sort(4);
                tw.searcher.tower.ypos.voracious_mt_sort(4);
                let time2 = Instant::now();
                let elpsed = time2.duration_since(time1);
                log::info!("build tower Sort pos time {:?}", elpsed);
            }
        }
        /*
        // 更新 Sort2 nearby_creeps
        let time1 = Instant::now();
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
        for (ent, pos, creeps) in
        (
            &tr.entities,
            &tr.pos,
            &tr.creeps,
        )
            .join(){
                if let Some(c) = tw.searcher.tower.SearchNN_XY(pos.0, 100., 1).get(0) {
                    if let Some(t) = tw.towers.get_mut(c.e) {
                        if let Some(p) = tr.pos.get(c.e) {
                            if t.range*t.range > c.dis {
                                let creep_dis = c.dis;
                                t.nearby_creeps.push(NearbyEnt { ent: ent, dis: creep_dis });
                            }
                        }
                    }
                }
            };
        for (ent, pos, tower) in
        (
            &tr.entities,
            &tr.pos,
            &tw.towers,
        )
            .join(){
            if ent.id() < 1000 && tower.nearby_creeps.len() > 0 {
                log::info!("{} {}", ent.id(), tower.nearby_creeps.len());
            }
        };
        let time2 = Instant::now();
        let elpsed = time2.duration_since(time1);
        log::info!("Sort2 search time {:?}", elpsed);
        // Sort1 更新 nearby_creeps
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
        for (ent, pos, creeps) in
        (
            &tr.entities,
            &tr.pos,
            &tr.creeps,
        )
            .join(){
                if let Some(c) = tw.searcher.tower.SearchNN_X(pos.0, 100., 1).get(0) {
                    if let Some(t) = tw.towers.get_mut(c.e) {
                        if let Some(p) = tr.pos.get(c.e) {
                            if t.range*t.range > c.dis {
                                let creep_dis = c.dis;
                                t.nearby_creeps.push(NearbyEnt { ent: ent, dis: creep_dis });
                            }
                        }
                    }
                }
            };
            for (ent, pos, tower) in
            (
                &tr.entities,
                &tr.pos,
                &tw.towers,
            )
                .join(){
                if ent.id() < 1000 && tower.nearby_creeps.len() > 0 {
                    log::info!("{} {}", ent.id(), tower.nearby_creeps.len());
                }
            };
        let time2 = Instant::now();
        let elpsed = time2.duration_since(time1);
        log::info!("Sort1 search time {:?}", elpsed);

        */
    }
}
