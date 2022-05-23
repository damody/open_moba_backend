use specs::storage::VecStorage;
use specs::{Component, FlaggedStorage, NullStorage};
use specs::Entity as EcsEntity;
use super::uid::Uid;

#[derive(Clone, Debug)]
pub struct Creep {
    pub class: String,
    pub path: String,
    pub pidx: i16,
}

impl Component for Creep {
    type Storage = VecStorage<Self>;
}

#[derive(Clone, Debug)]
pub struct CProperty {
    pub hp: f32,  // 血量
    pub msd: f32, // 移動速度
    pub def_physic: f32, // 物理防禦
    pub def_magic: f32, // 魔法防禦
}

impl Component for CProperty {
    type Storage = VecStorage<Self>;
}

#[derive(Clone, Debug)]
pub struct CreepEmiter {
    pub root: Creep,
    pub property: CProperty,
}
#[derive(Clone, Debug, Default)]
pub struct CurrentCreepWave {
    pub wave: usize,
    pub path: Vec<usize>,
}
#[derive(Clone, Debug)]
pub struct CreepWave {
    pub time: f32,
    pub path_creeps: Vec<PathCreeps>,
}
#[derive(Clone, Debug)]
pub struct PathCreeps {
    pub creeps: Vec<CreepEmit>,
    pub path_name: String,
}
#[derive(Clone, Debug)]
pub struct CreepEmit {
    pub time: f32,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct TakenDamage {
    pub phys: f32,
    pub magi: f32,
    pub real: f32,
    pub ent: EcsEntity,
}
