use std::collections::hash_map;

use hashbrown::HashMap;
use specs::{
    shred, Entities, Join, LazyUpdate, Read, ReadExpect, ReadStorage, SystemData,
    Write, WriteStorage, ParJoin, Entity, World,
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
    units : ReadStorage<'a, Unit>,
    heroes : ReadStorage<'a, Hero>,
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
        {//unit update (包含所有單位：creeps, units)
            // 收集所有 Unit 實體
            let (unit_ents, unit_pos) = (
                &tr.entities,
                &tr.pos,
                &tr.units,
            )
                .par_join()
                .map_init(
                    || {
                        prof_span!(guard, "unit nearby update rayon job");
                        guard
                    },
                    |_guard, (ent, pos, _)| {
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
                
            // 收集所有 Creep 實體（保持向後兼容）
            let (creep_ents, creep_pos) = (
                &tr.entities,
                &tr.pos,
                &tr.creeps,
            )
                .par_join()
                .map_init(
                    || {
                        prof_span!(guard, "creep nearby update rayon job");
                        guard
                    },
                    |_guard, (ent, pos, _)| {
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
                
            // 合併所有實體到 creep 索引中（向後兼容）— 走 CollisionIndex::rebuild_from
            // NOTE: Searcher / spatial index uses f32 internally for instant_distance lib compat.
            // Cache rebuilt per tick from authoritative Pos; entries sorted by Entity id below
            // for deterministic insertion order across par_join. Final distance check in caller is Fixed32.
            let mut combined: Vec<(Entity, vek::Vec2<f32>)> = unit_ents.iter().zip(unit_pos.iter())
                .map(|(e, p)| { let (x, y) = p.xy_f32(); (*e, vek::Vec2::new(x, y)) })
                .chain(creep_ents.iter().zip(creep_pos.iter()).map(|(e, p)| { let (x, y) = p.xy_f32(); (*e, vek::Vec2::new(x, y)) }))
                .collect();
            // Determinism: par_join collect order is non-deterministic; sort by Entity id to
            // ensure cross-host insertion order into the spatial index is identical.
            combined.sort_by_key(|(e, _)| (e.id(), e.gen().id()));
            tw.searcher.creep.rebuild_from(combined);

            log::debug!("Updated searcher index: {} units, {} creeps", unit_ents.len(), creep_ents.len());
        }
        {// hero update — 每 tick 重建（英雄會移動）
            let (hero_ents, hero_pos) = (
                &tr.entities,
                &tr.pos,
                &tr.heroes,
            )
                .par_join()
                .map_init(
                    || {
                        prof_span!(guard, "hero nearby update rayon job");
                        guard
                    },
                    |_guard, (ent, pos, _)| {
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

            // NOTE: Searcher / spatial index uses f32 internally for instant_distance lib compat.
            // Cache rebuilt per tick from authoritative Pos; entries sorted by Entity id below
            // for deterministic insertion order across par_join. Final distance check in caller is Fixed32.
            let mut hero_items: Vec<(Entity, vek::Vec2<f32>)> = hero_ents.iter().zip(hero_pos.iter())
                .map(|(e, p)| { let (x, y) = p.xy_f32(); (*e, vek::Vec2::new(x, y)) })
                .collect();
            // Determinism: par_join collect order is non-deterministic; sort by Entity id.
            hero_items.sort_by_key(|(e, _)| (e.id(), e.gen().id()));
            tw.searcher.hero.rebuild_from(hero_items);
        }
        if tw.searcher.tower.is_dirty() {
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
            if tw.searcher.tower.is_dirty() {
                let time1 = Instant::now();
                // NOTE: Searcher / spatial index uses f32 internally for instant_distance lib compat.
            // Cache rebuilt per tick from authoritative Pos; entries sorted by Entity id below
            // for deterministic insertion order across par_join. Final distance check in caller is Fixed32.
                let mut tower_items: Vec<(Entity, vek::Vec2<f32>)> = ents.iter().zip(pos.iter())
                    .map(|(e, p)| { let (x, y) = p.xy_f32(); (*e, vek::Vec2::new(x, y)) })
                    .collect();
                // Determinism: par_join collect order is non-deterministic; sort by Entity id.
                tower_items.sort_by_key(|(e, _)| (e.id(), e.gen().id()));
                tw.searcher.tower.rebuild_from(tower_items);
                let time2 = Instant::now();
                let elpsed = time2.duration_since(time1);
                log::info!("build tower Sort pos time {:?}", elpsed);
            }
        }
    }
}
