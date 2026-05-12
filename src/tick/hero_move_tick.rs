pub use omoba_core::runtime::native::tick::hero_move_tick::{
    advance_with_collision, HeroMoveRead, HeroMoveWrite,
};

pub type Sys = super::CoreTick<omoba_core::runtime::native::tick::hero_move_tick::Sys>;
