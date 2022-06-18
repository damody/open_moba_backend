use super::{
    attack::*,
};
use serde::{Deserialize, Serialize};
use specs::{Component, VecStorage};
use specs::DenseVecStorage;
use std::time::Duration;
use specs::Entity as EcsEntity;
use vek::Vec2;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Projectile {
    pub time_left: f32,
    pub owner: EcsEntity,
    // 如果有target就是指定技 不然就是指向技
    pub target: Option<EcsEntity>,
    pub tpos: Vec2<f32>,
    pub radius: f32,
    pub msd: f32,
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
    pub pos: Vec2<f32>,
}

impl Component for Projectile {
    type Storage = DenseVecStorage<Self>;
}
