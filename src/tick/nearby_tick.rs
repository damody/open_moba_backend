use std::collections::hash_map;

use hashbrown::HashMap;
use specs::{
    shred::{ResourceId, World}, Entities, Join, LazyUpdate, Read, ReadExpect, ReadStorage, SystemData,
    Write, WriteStorage, ParJoin, Entity,
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
    }
}
