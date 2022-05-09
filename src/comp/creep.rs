use specs::storage::VecStorage;
use specs::{Component, FlaggedStorage, NullStorage};
use specs::Entity as EcsEntity;
use super::uid::Uid;

#[derive(Copy, Clone, Debug)]
pub struct Creep {
    pub lv: i32,
}

impl Component for Creep {
    type Storage = VecStorage<Self>;
}

#[derive(Copy, Clone, Debug)]
pub struct CProperty {
    pub hp: f32,  // 血量
    pub msd: f32, // 移動速度
    pub def_physic: f32, // 物理防禦
    pub def_magic: f32, // 魔法防禦
}

impl Component for CProperty {
    type Storage = VecStorage<Self>;
}

#[derive(Copy, Clone, Debug)]
pub struct TakenDamage {
    pub phys: f32,
    pub magi: f32,
    pub real: f32,
    pub uid: Uid,
}
