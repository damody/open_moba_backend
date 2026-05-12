pub use omoba_core::runtime::native::tick::player_tick::{PlayerRead, PlayerWrite};

pub type Sys = super::CoreTick<omoba_core::runtime::native::tick::player_tick::Sys>;
