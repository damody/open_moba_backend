pub mod tower_tick;
pub mod hero_tick;
pub mod damage_tick;
pub mod death_tick;
pub mod creep_wave;
pub mod creep_tick;
pub mod nearby_tick;
pub mod projectile_tick;
pub mod player_tick;
pub mod skill_tick;
pub mod skill_tick_refactored;
pub mod skill_system;
// pub mod ability_tick;  // 舊的ability系統已停用
// pub mod new_ability_tick;  // 移除，整合到skill_tick中

pub use self::{
    tower_tick::*,
    hero_tick::*,
    damage_tick::*,
    death_tick::*,
    creep_tick::*,
    creep_wave::*,
    nearby_tick::*,
    projectile_tick::*,
    player_tick::*,
    skill_tick::*,
    // ability_tick::*,  // 舊的ability系統已停用
    // new_ability_tick::*,  // 移除，整合到skill_tick中
};