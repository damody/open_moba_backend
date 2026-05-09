use std::collections::hash_map;

use crate::comp::phys::*;
use crate::comp::*;
use crate::transport::OutboundMsg;
use crossbeam_channel::Sender;
use hashbrown::HashMap;
use specs::prelude::ParallelIterator;
use specs::{
    shred, Entities, Entity, Join, LazyUpdate, ParJoin, Read, ReadExpect, ReadStorage, SystemData,
    World, Write, WriteStorage,
};
use std::time::{Duration, Instant};
use voracious_radix_sort::RadixSort;

#[derive(SystemData)]
pub struct PlayerRead<'a> {
    entities: Entities<'a>,
    time: Read<'a, Time>,
    dt: Read<'a, DeltaTime>,
    towers: WriteStorage<'a, Tower>,
    creeps: ReadStorage<'a, Creep>,
    pos: ReadStorage<'a, Pos>,
    searcher: Write<'a, Searcher>,
}

#[derive(SystemData)]
pub struct PlayerWrite<'a> {
    entities: Entities<'a>,
    mqtx: Write<'a, Vec<Sender<OutboundMsg>>>,
}

#[derive(Default)]
pub struct Sys;

impl<'a> System<'a> for Sys {
    type SystemData = (PlayerRead<'a>, PlayerWrite<'a>);

    const NAME: &'static str = "player";

    fn run(_job: &mut Job<Self>, (tr, mut tw): Self::SystemData) {}
}
