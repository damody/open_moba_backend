use specs::storage::VecStorage;
use specs::Component;
use serde::{Deserialize, Serialize};

/// 擊殺獎勵：金錢與經驗值
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
pub struct Bounty {
    pub gold: i32,
    pub exp: i32,
}

impl Component for Bounty {
    type Storage = VecStorage<Self>;
}
