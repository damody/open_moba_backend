use specs::storage::VecStorage;
use specs::{Component, FlaggedStorage, NullStorage, saveload};
use specs::Entity;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum CreepStatus {
    Walk,
    Stop,
    PreWalk,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Creep {
    pub name: String,
    pub path: String,
    pub pidx: usize,
    pub block_tower: Option<Entity>,
    pub status: CreepStatus,
}

impl Component for Creep {
    type Storage = VecStorage<Self>;
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CProperty {
    pub hp: f32,  // 目前血量
    pub mhp: f32,  // 最大血量
    pub msd: f32, // 移動速度
    pub def_physic: f32, // 物理防禦
    pub def_magic: f32, // 魔法防禦
}

impl Component for CProperty {
    type Storage = VecStorage<Self>;
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CreepEmiter {
    pub root: Creep,
    pub property: CProperty,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct CurrentCreepWave {
    pub wave: usize,
    pub path: Vec<usize>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CreepWave {
    pub time: f32,
    pub path_creeps: Vec<PathCreeps>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PathCreeps {
    pub creeps: Vec<CreepEmit>,
    pub path_name: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CreepEmit {
    pub time: f32,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TakenDamage {
    pub phys: f32,
    pub magi: f32,
    pub real: f32,
    pub ent: Entity,
    pub source: Entity,  // 攻擊者
}
