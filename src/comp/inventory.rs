use specs::storage::VecStorage;
use specs::Component;
use serde::{Deserialize, Serialize};

pub const INVENTORY_SLOTS: usize = 6;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ItemInstance {
    pub item_id: String,
    pub cooldown_remaining: f32,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Inventory {
    pub slots: [Option<ItemInstance>; INVENTORY_SLOTS],
}

impl Inventory {
    pub fn new() -> Self {
        Inventory::default()
    }

    pub fn first_free_slot(&self) -> Option<usize> {
        self.slots.iter().position(|s| s.is_none())
    }

    pub fn find_item(&self, item_id: &str) -> Option<usize> {
        self.slots
            .iter()
            .position(|s| s.as_ref().map(|i| i.item_id == item_id).unwrap_or(false))
    }

    pub fn items(&self) -> impl Iterator<Item = (usize, &ItemInstance)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.as_ref().map(|x| (i, x)))
    }
}

impl Component for Inventory {
    type Storage = VecStorage<Self>;
}
