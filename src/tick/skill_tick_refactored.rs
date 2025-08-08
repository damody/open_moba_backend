/// 重構後的技能系統
/// 
/// 將原本的大型 skill_tick.rs 拆分為多個模組以提升可維護性

use specs::{Join, System};
use crate::comp::ecs;

use crate::tick::skill_system::{
    SkillRead, SkillWrite, EffectManager, InputHandler,
};

/// 技能系統主結構
pub struct Sys;

impl Default for Sys {
    fn default() -> Self {
        Self
    }
}

impl<'a> ecs::System<'a> for Sys {
    type SystemData = (SkillRead<'a>, SkillWrite<'a>);

    const NAME: &'static str = "skill";

    fn run(_job: &mut ecs::Job<Self>, (tr, mut tw): Self::SystemData) {
        let dt = tr.dt.0;
        
        // 1. 更新所有技能的冷卻時間和狀態
        update_skill_cooldowns(&tr, &mut tw, dt);
        
        // 2. 處理技能輸入
        InputHandler::process_inputs(&tr, &mut tw);
        
        // 3. 更新技能效果
        EffectManager::update_effects(&tr, &mut tw);
    }
}

/// 更新技能冷卻時間
fn update_skill_cooldowns(tr: &SkillRead, tw: &mut SkillWrite, dt: f32) {
    let time = tr.time.0 as f32;
    
    for (_entity, skill) in (&tr.entities, &mut tw.skills).join() {
        skill.update(dt, time);
    }
}