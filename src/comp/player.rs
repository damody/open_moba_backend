use specs::storage::VecStorage;
use specs::{Component};
use specs::Entity;
use serde::{Deserialize, Serialize};
use crate::TProperty;
use crate::TowerData;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Player {
    pub name: String,
    pub cost: f32,
    pub towers: Vec<TowerData>,
}

impl Component for Player {
    type Storage = VecStorage<Self>;
}
