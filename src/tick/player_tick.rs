use crate::comp::*;
use specs::{shred, Entities, Read, ReadStorage, SystemData, Write, WriteStorage};

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
}

#[derive(Default)]
pub struct Sys;

impl<'a> System<'a> for Sys {
    type SystemData = (PlayerRead<'a>, PlayerWrite<'a>);

    const NAME: &'static str = "player";

    fn run(_job: &mut Job<Self>, (_tr, _tw): Self::SystemData) {}
}
