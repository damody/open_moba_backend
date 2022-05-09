use super::{
    attack::*,
    uid::Uid,
};
use serde::{Deserialize, Serialize};
use specs::{Component, VecStorage};
use specs::DenseVecStorage;
use std::time::Duration;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Effect {
    Attack(Attack),
    Explode(Explosion),
    Vanish,
    Stick,
    Possess,
    Bonk, // Knock/dislodge/change objects on hit
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Projectile {
    pub time_left: f32,
    pub owner: Option<Uid>,
}

impl Component for Projectile {
    type Storage = VecStorage<Self>;
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum ProjectileConstructor {
    Arrow {
    },
    /*
    Fireball {
        damage: f32,
        radius: f32,
        energy_regen: f32,
        min_falloff: f32,
    },
    Frostball {
        damage: f32,
        radius: f32,
        min_falloff: f32,
    },
    Poisonball {
        damage: f32,
        radius: f32,
        min_falloff: f32,
    },
    NecroticSphere {
        damage: f32,
        radius: f32,
        min_falloff: f32,
    },
    ClayRocket {
        damage: f32,
        radius: f32,
        knockback: f32,
        min_falloff: f32,
    },
    Snowball {
        damage: f32,
        radius: f32,
        min_falloff: f32,
    },
    ExplodingPumpkin {
        damage: f32,
        radius: f32,
        knockback: f32,
        min_falloff: f32,
    },*/
}

impl ProjectileConstructor {
    pub fn create_projectile(
        self,
        owner: Option<Uid>,
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
