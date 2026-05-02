use std::collections::BTreeMap;
use specs::storage::VecStorage;
use specs::{Component, FlaggedStorage, NullStorage, Entity as  Entity};
use serde::{Deserialize, Serialize};
use vek::Vec2;
use omoba_sim::{Fixed32, Vec2 as SimVec2};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Tower {
    pub nearby_creeps: Vec<NearbyEnt>,
    pub block_creeps: Vec<Entity>,
    pub buffs: Vec<TModify>,
    #[serde(default)]
    pub upgrade_levels: [u8; 3],
    #[serde(default)]
    pub upgrade_flags: Vec<String>,
    #[serde(default)]
    pub ultimate_cooldown: Fixed32,
}
impl Tower {
    pub fn new() -> Self {
        Self {
            nearby_creeps: vec![],
            block_creeps: vec![],
            buffs: vec![],
            upgrade_levels: [0; 3],
            upgrade_flags: vec![],
            ultimate_cooldown: Fixed32::ZERO,
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct NearbyEnt {
    pub ent: Entity,
    pub dis: Fixed32,
}

impl Component for Tower {
    type Storage = VecStorage<Self>;
}

#[derive(Copy, Clone, Debug, Deserialize, Serialize)]
pub struct TAttack {
    pub atk_physic: Vf32, // 物攻
    pub asd: Vf32, // 攻速/每幾秒攻擊一次
    pub range: Vf32, // 射程
    pub asd_count: Fixed32,
    pub bullet_speed: Fixed32,
}

impl TAttack {
    pub fn new(atk: Fixed32, asd: Fixed32, range: Fixed32, bullet_speed: Fixed32) -> Self {
        Self {
            atk_physic: Vf32::new(atk),
            asd: Vf32::new(asd),
            asd_count: asd,
            range: Vf32::new(range),
            bullet_speed,
        }
    }
}

impl Component for TAttack {
    type Storage = VecStorage<Self>;
}


#[derive(Copy, Clone, Debug, Deserialize, Serialize)]
pub struct TProperty {
    pub hp: Vf32,  // hp
    pub block: i32, // 目前檔幾人
    pub mblock: i32, // 最大檔幾人
    pub size: Fixed32, // 阻檔半徑
}

impl TProperty {
    pub fn new(hp: Fixed32, block: i32, size: Fixed32) -> Self {
        Self {
            hp: Vf32::new(hp),
            block: 0,
            mblock: block,
            size,
        }
    }
}

impl Component for TProperty {
    type Storage = VecStorage<Self>;
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TAbility {
    pub name: String,
    pub values: BTreeMap<String, Vec<Fixed32>>,
}
#[derive(Copy, Clone, Debug, Deserialize, Serialize)]
pub enum ModifyType {
    HP,
    MP,
    Attack,
    AttackSpeed,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum DurationType {
    AttackCount(i32),
    Duration(Fixed32),
    Infinite,
    PosAura(SimVec2, Fixed32),
    TowerAura(Entity, Fixed32),
    CreepAura(Entity, Fixed32),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TModify {
    pub n: String,
    pub dt: DurationType,
    pub mt: ModifyType,
    pub v: Fixed32,
}


#[derive(Copy, Clone, Debug, Deserialize, Serialize)]
pub struct Vf32 {
    pub bv: Fixed32,
    pub v: Fixed32,
}
impl Vf32 {
    pub fn new(v: Fixed32) -> Vf32 {
        Vf32 {
            bv: v,
            v,
        }
    }
    pub fn val(&mut self) -> Fixed32 {
        self.v
    }
    //還原
    pub fn reset(&mut self) -> &mut Vf32 {
        self.v = self.bv;
        self
    }
    //暫時乘上
    pub fn mul(&mut self, v: Fixed32) -> &mut Vf32 {
        self.v *= v;
        self
    }
    //暫時加上
    pub fn add(&mut self, v: Fixed32) -> &mut Vf32 {
        self.v += v;
        self
    }
    // v += bv*v
    pub fn add_mul(&mut self, v: Fixed32) -> &mut Vf32 {
        self.v += self.bv * v;
        self
    }
    pub fn clamp(&mut self, minv: Fixed32, maxv: Fixed32) -> &mut Vf32 {
        self.v = if self.v > maxv { maxv } else { self.v };
        self.v = if self.v < minv { minv } else { self.v };
        self
    }

}

#[derive(Copy, Clone, Debug, Deserialize, Serialize)]
pub struct Val<T> {
    pub bv: T,
    pub mv: T,
    pub v: T,
}

impl<T> Val<T> where 
    T: Copy + Ord + std::ops::MulAssign + std::ops::AddAssign {
    fn new(v: T) -> Val<T> {
        Val {
            bv: v,
            mv: v,
            v: v,
        }
    }
    
    //還原
    fn reset(&mut self) -> &mut Val<T> {
        self.v = self.bv;
        self
    }
    //暫時乘上
    fn mul(&mut self, v: T) -> &mut Val<T> {
        self.v *= v;
        self.v = self.v.max(self.mv);
        self
    }
    //暫時加上
    fn add(&mut self, v: T) -> &mut Val<T> {
        self.v += v;
        self.v = self.v.max(self.mv);
        self
    }
}