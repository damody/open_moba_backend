/// 技能系統模組
/// 
/// 負責管理遊戲中的技能系統，包括技能處理、效果管理、輸入處理等

pub mod processor;
pub mod effects;
pub mod input_handler;
pub mod abilities;

pub use processor::SkillProcessor;
pub use effects::EffectManager;
pub use input_handler::InputHandler;
pub use abilities::AbilityManager;

use specs::{
    Entities, Read, ReadStorage, Write, WriteStorage, SystemData,
};
use crate::comp::*;

/// 技能系統讀取資源
#[derive(SystemData)]
pub struct SkillRead<'a> {
    pub entities: Entities<'a>,
    pub time: Read<'a, Time>,
    pub dt: Read<'a, DeltaTime>,
    pub heroes: ReadStorage<'a, Hero>,
    pub units: ReadStorage<'a, Unit>,
    pub abilities: Read<'a, std::collections::BTreeMap<String, Ability>>,
    pub positions: ReadStorage<'a, Pos>,
    pub factions: ReadStorage<'a, Faction>,
    pub properties: ReadStorage<'a, CProperty>,
    pub attacks: ReadStorage<'a, TAttack>,
}

/// 技能系統寫入資源
#[derive(SystemData)]
pub struct SkillWrite<'a> {
    pub outcomes: Write<'a, Vec<Outcome>>,
    pub skills: WriteStorage<'a, Skill>,
    pub skill_effects: WriteStorage<'a, SkillEffect>,
    pub skill_inputs: Write<'a, Vec<SkillInput>>,
    pub damage_instances: Write<'a, Vec<DamageInstance>>,
}