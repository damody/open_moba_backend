pub mod attack_phase;
pub mod buff_tick;
pub mod creep_tick;
pub mod creep_wave;
pub mod damage_tick;
pub mod death_tick;
pub mod hero_move_tick;
pub mod hero_tick;
pub mod item_tick;
pub mod nearby_tick;
pub mod player_input_tick;
pub mod player_tick;
pub mod projectile_tick;
pub mod regen_tick;
pub mod summon_tick;
pub mod tower_tick;
// 舊的 skill_tick / skill_system / skill_tick_refactored 已移除。
// 新的技能 dispatch 走 AbilityScript FFI trait（scripts/base_content/src/heroes/）。

pub use self::{
    creep_tick::*, creep_wave::*, damage_tick::*, death_tick::*, hero_move_tick::*, hero_tick::*,
    item_tick::*, nearby_tick::*, player_tick::*, projectile_tick::*, tower_tick::*,
};
