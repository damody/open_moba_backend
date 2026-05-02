use super::{
    attack::*,
};
use serde::{Deserialize, Serialize};
use specs::{Component, VecStorage};
use specs::DenseVecStorage;
use std::time::Duration;
use specs::Entity;
use omoba_sim::{Fixed64, Vec2 as SimVec2};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Projectile {
    pub time_left: Fixed64,
    pub owner: Entity,
    // 如果有target就是指定技 不然就是指向技
    pub target: Option<Entity>,
    pub tpos: SimVec2,
    pub radius: Fixed64,
    pub msd: Fixed64,
    // 傷害值資訊
    pub damage_phys: Fixed64,  // 物理傷害
    pub damage_magi: Fixed64,  // 魔法傷害
    pub damage_real: Fixed64,  // 真實傷害
    /// 命中後套用的減速乘數（0.0 表不減速，0.5 表減速到 50%）
    #[serde(default)]
    pub slow_factor: Fixed64,
    /// 減速持續秒數
    #[serde(default)]
    pub slow_duration: Fixed64,
    /// 沿路 hit-test 半徑（無 target 方向性子彈用；0 = 使用預設）
    #[serde(default)]
    pub hit_radius: Fixed64,
    /// 命中後對目標施加的 stun 持續秒數（0 = 不暈眩）。
    /// 由 handle_projectile 在發射時擲骰決定（例：matchlock_gun 的 attack_stun_chance）。
    #[serde(default)]
    pub stun_duration: Fixed64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ProjectileData {
    pub id: u32,
    pub time_left: f32,
    pub owner: u32,
    // 如果有target就是指定技 不然就是指向技
    pub target: u32, // 0 就是沒有目標
    pub radius: f32,
    pub msd: f32,
    pub pos: vek::Vec2<f32>,
}

impl Component for Projectile {
    type Storage = DenseVecStorage<Self>;
}
