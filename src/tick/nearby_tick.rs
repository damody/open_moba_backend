pub use omoba_core::runtime::native::tick::nearby_tick::{NearbyRead, NearbyWrite};

pub type Sys = super::CoreTick<omoba_core::runtime::native::tick::nearby_tick::Sys>;
