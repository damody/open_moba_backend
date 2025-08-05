/// 雜賀孫市 (B01_saika_magoichi) 技能模組
/// 
/// 包含雜賀孫市的四個技能實作：
/// - No1: 狙擊模式 (sniper_mode) - 切換技能
/// - No2: 雜賀眾 (saika_reinforcements) - 召喚技能  
/// - No3: 雨鐵炮 (rain_iron_cannon) - 區域技能
/// - No4: 三段擊 (three_stage_technique) - 攻擊技能

pub mod No1_sniper_mode;
pub mod No2_saika_reinforcements;
pub mod No3_rain_iron_cannon;
pub mod No4_three_stage_technique;

pub use No1_sniper_mode::SniperModeHandler;
pub use No2_saika_reinforcements::SaikaReinforcementsHandler;
pub use No3_rain_iron_cannon::RainIronCannonHandler;
pub use No4_three_stage_technique::ThreeStageHandler;