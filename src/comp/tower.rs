use std::collections::BTreeMap;
use specs::storage::VecStorage;
use specs::{Component, FlaggedStorage, NullStorage, Entity as  Entity};
use serde::{Deserialize, Serialize};
use vek::Vec2;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Tower {
    pub nearby_creeps: Vec<NearbyEnt>,
    pub block_creeps: Vec<Entity>,
    pub buffs: Vec<TModify>,
}
impl Tower {
    pub fn new() -> Self {
        Self {
            nearby_creeps: vec![],
            block_creeps: vec![],
            buffs: vec![],
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct NearbyEnt {
    pub ent: Entity,
    pub dis: f32,
}

impl Component for Tower {
    type Storage = VecStorage<Self>;
}

#[derive(Copy, Clone, Debug, Deserialize, Serialize)]
pub struct TAttack {
    pub atk_physic: Vf32, // 物攻
    pub asd: Vf32, // 攻速/每幾秒攻擊一次
    pub range: Vf32, // 射程
    pub asd_count: f32,
    pub bullet_speed: f32,
}

impl TAttack {
    pub fn new(atk: f32, asd: f32, range: f32, bullet_speed: f32) -> Self {
        Self {
            atk_physic: atk.into(),
            asd: asd.into(),
            asd_count: asd,
            range: range.into(),
            bullet_speed: bullet_speed,
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
    pub size: f32, // 阻檔半徑
}

impl TProperty {
    pub fn new(hp: f32, block: i32, size: f32) -> Self {
        Self {
            hp: hp.into(),
            block: 0,
            mblock: block,
            size: size,
        }
    }
}

impl Component for TProperty {
    type Storage = VecStorage<Self>;
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TAbility {
    pub name: String,
    pub values: BTreeMap<String, Vec<f32>>,
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
    Duration(f32),
    Infinite,
    PosAura(Vec2<f32>, f32),
    TowerAura(Entity, f32),
    CreepAura(Entity, f32),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TModify {
    pub n: String,
    pub dt: DurationType,
    pub mt: ModifyType,
    pub v: f32,
}


#[derive(Copy, Clone, Debug, Deserialize, Serialize)]
pub struct Vf32 {
    pub bv: f32,
    pub v: f32,
}
impl From<Vf32> for f32 {
    fn from(v: Vf32) -> Self {
        v.v
    }
}
impl From<f32> for Vf32 {
    fn from(v: f32) -> Self {
        Vf32 { bv: v, v: v }
    }
}
impl Vf32 {
    pub fn new(v: f32) -> Vf32 {
        Vf32 {
            bv: v,
            v: v,
        }
    }
    pub fn val(&mut self) -> f32 {
        self.v
    }
    //還原
    pub fn reset(&mut self) -> &mut Vf32 {
        self.v = self.bv;
        self
    }
    //暫時乘上
    pub fn mul(&mut self, v: f32) -> &mut Vf32 {
        self.v *= v;
        self
    }
    //暫時加上
    pub fn add(&mut self, v: f32) -> &mut Vf32 {
        self.v += v;
        self
    }
    // v += mv*v
    pub fn add_mul(&mut self, v: f32) -> &mut Vf32 {
        self.v += self.bv * v;
        self
    }
    pub fn clamp(&mut self, minv: f32, maxv: f32) -> &mut Vf32 {
        self.v = self.v.min(maxv).max(minv);
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