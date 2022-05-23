use specs::storage::VecStorage;
use specs::{Component, FlaggedStorage, NullStorage, Entity as  EcsEntity};
use crate::ProjectileConstructor;
use crate::Uid;

#[derive(Clone, Debug)]
pub struct Tower {
    pub lv: i32,
    pub projectile_kind: ProjectileConstructor,
    pub nearby_creeps: Vec<EcsEntity>,
}

impl Component for Tower {
    type Storage = VecStorage<Self>;
}

#[derive(Copy, Clone, Debug)]
pub struct TProperty {
    pub base_hp: i32,  // hp
    pub cur_hp: i32,  // hp
    pub max_hp: i32,  // hp
    pub base_atk_physic: f32, // 物攻
    pub base_atk_magic: f32, // 魔攻
    pub atk_physic: f32, // 物攻
    pub atk_magic: f32, // 魔攻
    pub asd: f32, // 每幾秒攻擊一次
    pub asd_count: f32,
    pub range: f32, // 射程
}

impl TProperty {
    pub fn new(hp: i32, patk: f32, matk: f32, asd: f32, range: f32) -> Self {
        Self {
            base_hp: hp,
            cur_hp: hp,
            max_hp: hp,
            base_atk_physic: patk,
            atk_physic: patk,
            base_atk_magic: matk,
            atk_magic: matk,
            asd: asd,
            asd_count: 0.,
            range: range,
        }
    }
}

impl Component for TProperty {
    type Storage = VecStorage<Self>;
}
