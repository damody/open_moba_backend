pub mod tower_tick;
pub mod slow_buff_tick;
pub mod hero_tick;
pub mod hero_move_tick;
pub mod damage_tick;
pub mod death_tick;
pub mod creep_wave;
pub mod creep_tick;
pub mod nearby_tick;
pub mod projectile_tick;
pub mod player_tick;
pub mod item_tick;
// 舊的 skill_tick / skill_system / skill_tick_refactored 已移除。
// 新的技能 dispatch 走 AbilityScript FFI trait（scripts/base_content/src/heroes/）。

pub use self::{
    tower_tick::*,
    hero_tick::*,
    hero_move_tick::*,
    damage_tick::*,
    death_tick::*,
    creep_tick::*,
    creep_wave::*,
    nearby_tick::*,
    projectile_tick::*,
    player_tick::*,
    item_tick::*,
};
