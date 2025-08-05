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
use crate::MqttMsg;
use crossbeam_channel::Sender;

#[derive(SystemData)]
pub struct PlayerRead<'a> {
    entities: Entities<'a>,
    time: Read<'a, Time>,
    dt: Read<'a, DeltaTime>,
    towers : WriteStorage<'a, Tower>,
    creeps : ReadStorage<'a, Creep>,
    pos : ReadStorage<'a, Pos>,
    searcher: Write<'a, Searcher>,
}

#[derive(SystemData)]
pub struct PlayerWrite<'a> {
    entities: Entities<'a>,
    mqtx: Write<'a, Vec<Sender<MqttMsg>>>,
}

#[derive(Default)]
pub struct Sys;

impl<'a> System<'a> for Sys {
    type SystemData = (
        PlayerRead<'a>,
        PlayerWrite<'a>,
    );

    const NAME: &'static str = "player";

    fn run(_job: &mut Job<Self>, (tr, mut tw): Self::SystemData) {

    }
}
