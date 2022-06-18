use specs::storage::VecStorage;
use specs::{Component, FlaggedStorage, NullStorage, Entity as  EcsEntity};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Tower {
    pub nearby_creeps: Vec<NearbyEnt>,
    pub block_creeps: Vec<EcsEntity>,
}
impl Tower {
    pub fn new() -> Self {
        Self {
            nearby_creeps: vec![],
            block_creeps: vec![],
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct NearbyEnt {
    pub ent: EcsEntity,
    pub dis: f32,
}

impl Component for Tower {
    type Storage = VecStorage<Self>;
}

#[derive(Copy, Clone, Debug, Deserialize, Serialize)]
pub struct TAttack {
    pub atk_physic: f32, // 物攻
    pub asd: f32, // 每幾秒攻擊一次
    pub asd_count: f32,
    pub range: f32, // 射程
    pub bullet_speed: f32,
}

impl TAttack {
    pub fn new(atk: f32, asd: f32, range: f32, bullet_speed: f32) -> Self {
        Self {
            atk_physic: atk,
            asd: asd,
            asd_count: asd,
            range: range,
            bullet_speed: bullet_speed,
        }
    }
}

impl Component for TAttack {
    type Storage = VecStorage<Self>;
}


#[derive(Copy, Clone, Debug, Deserialize, Serialize)]
pub struct TProperty {
    pub base_hp: i32,  // hp
    pub cur_hp: i32,  // hp
    pub max_hp: i32,  // hp
    pub block: i32, // 目前檔幾人
    pub mblock: i32, // 最大檔幾人
    pub size: f32, // 阻檔半徑
}

impl TProperty {
    pub fn new(hp: i32, block: i32, size: f32) -> Self {
        Self {
            base_hp: hp,
            cur_hp: hp,
            max_hp: hp,
            block: 0,
            mblock: block,
            size: size,
        }
    }
}

impl Component for TProperty {
    type Storage = VecStorage<Self>;
}
