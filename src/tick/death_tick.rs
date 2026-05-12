pub use omoba_core::runtime::native::tick::death_tick::{DeathRead, DeathWrite};

pub type Sys = super::CoreTick<omoba_core::runtime::native::tick::death_tick::Sys>;
