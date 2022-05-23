use super::{
    attack::*,
    uid::Uid,
};
use serde::{Deserialize, Serialize};
use specs::{Component, VecStorage};
use specs::DenseVecStorage;
use std::time::Duration;
use specs::Entity as EcsEntity;

#[derive(Clone, Debug)]
pub struct Projectile {
    pub time_left: f32,
    pub owner: EcsEntity,
}

impl Component for Projectile {
    type Storage = DenseVecStorage<Self>;
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum ProjectileConstructor {
    Arrow {
    },
}

impl ProjectileConstructor {
    pub fn create_projectile(
        self,
        owner: EcsEntity,
        damage: f32,
        range: f32,
    ) -> Projectile {
        use ProjectileConstructor::*;
        match self {
            Arrow {
            } => {
                Projectile {
                    time_left: 15.,
                    owner,
                }
            },
        }
    }

}
