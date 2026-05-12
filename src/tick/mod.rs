use crate::comp::ecs::{Job, System};

pub struct CoreTick<T>(std::marker::PhantomData<T>);

impl<T> Default for CoreTick<T> {
    fn default() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<'a, T> System<'a> for CoreTick<T>
where
    T: omoba_core::comp::ecs::System<'a> + Default,
{
    const NAME: &'static str = <T as omoba_core::comp::ecs::System>::NAME;

    type SystemData = <T as omoba_core::comp::ecs::System<'a>>::SystemData;

    fn run(_job: &mut Job<Self>, data: Self::SystemData) {
        let mut core_job = omoba_core::comp::ecs::Job::<T>::default();
        <T as omoba_core::comp::ecs::System>::run(&mut core_job, data);
    }
}

pub mod attack_phase;
pub mod buff_tick;
pub mod core_creep_tick;
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
    creep_wave::*, damage_tick::*, death_tick::*, hero_move_tick::*, hero_tick::*, item_tick::*,
    nearby_tick::*, player_tick::*, projectile_tick::*, tower_tick::*,
};
