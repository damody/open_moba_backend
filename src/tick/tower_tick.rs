pub use omoba_core::runtime::native::tick::tower_tick::{TowerRead, TowerWrite};

pub type Sys = super::CoreTick<omoba_core::runtime::native::tick::tower_tick::Sys>;
