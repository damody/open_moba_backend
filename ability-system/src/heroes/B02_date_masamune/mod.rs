/// 伊達政宗 (B02_date_masamune) 技能模組
/// 
/// 包含伊達政宗的四個技能實作：
/// - No1: 火焰刀 (flame_blade) - 前方攻擊技能
/// - No2: 踏火無痕 (fire_dash) - 衝刺技能  
/// - No3: 火焰強襲 (flame_assault) - 範圍暈眩技能
/// - No4: 火繩銃 (matchlock_gun) - 變身技能

pub mod No1_flame_blade;
pub mod No2_fire_dash;
pub mod No3_flame_assault;
pub mod No4_matchlock_gun;

pub use No1_flame_blade::FlameBladeHandler;
pub use No2_fire_dash::FireDashHandler;
pub use No3_flame_assault::FlameAssaultHandler;
pub use No4_matchlock_gun::MatchlockGunHandler;