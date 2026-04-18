use specs::storage::VecStorage;
use specs::Component;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
pub struct Gold(pub i32);

impl Component for Gold {
    type Storage = VecStorage<Self>;
}
