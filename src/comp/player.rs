use crate::TowerData;
use serde::{Deserialize, Serialize};
use specs::storage::VecStorage;
use specs::Component;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Player {
    pub name: String,
    pub cost: f32,
    pub towers: Vec<TowerData>,
}

impl Component for Player {
    type Storage = VecStorage<Self>;
}
