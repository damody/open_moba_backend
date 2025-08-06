use std::collections::HashMap;
use crate::comp::*;

/// 技能管理器
pub struct AbilityManager;

impl AbilityManager {
    /// 獲取技能的基礎資訊
    pub fn get_ability_info(ability_id: &str) -> Option<AbilityInfo> {
        match ability_id {
            "sniper_mode" => Some(AbilityInfo {
                id: "sniper_mode".to_string(),
                name: "狙擊模式".to_string(),
                description: "切換為狙擊模式，增加射程和傷害，但降低攻速和移速".to_string(),
                ability_type: AbilityType::Toggle,
                max_level: 4,
                mana_cost: vec![0.0, 0.0, 0.0, 0.0],
                cooldown: vec![0.0, 0.0, 0.0, 0.0],
                cast_range: 0.0,
                effects: Self::get_sniper_mode_effects(),
            }),
            "saika_reinforcements" => Some(AbilityInfo {
                id: "saika_reinforcements".to_string(),
                name: "雜賀眾".to_string(),
                description: "召喚雜賀鐵炮兵支援戰鬥".to_string(),
                ability_type: AbilityType::Active,
                max_level: 3,
                mana_cost: vec![80.0, 90.0, 100.0],
                cooldown: vec![30.0, 25.0, 20.0],
                cast_range: 800.0,
                effects: Self::get_saika_reinforcements_effects(),
            }),
            "rain_iron_cannon" => Some(AbilityInfo {
                id: "rain_iron_cannon".to_string(),
                name: "雨鐵砲".to_string(),
                description: "對目標區域造成大範圍傷害".to_string(),
                ability_type: AbilityType::Active,
                max_level: 3,
                mana_cost: vec![120.0, 140.0, 160.0],
                cooldown: vec![60.0, 50.0, 40.0],
                cast_range: 1200.0,
                effects: Self::get_rain_iron_cannon_effects(),
            }),
            "three_stage_technique" => Some(AbilityInfo {
                id: "three_stage_technique".to_string(),
                name: "三段擊".to_string(),
                description: "對單一目標連續進行三次攻擊".to_string(),
                ability_type: AbilityType::Active,
                max_level: 4,
                mana_cost: vec![60.0, 70.0, 80.0, 90.0],
                cooldown: vec![15.0, 14.0, 13.0, 12.0],
                cast_range: 600.0,
                effects: Self::get_three_stage_technique_effects(),
            }),
            _ => None,
        }
    }

    /// 驗證技能施放條件
    pub fn validate_cast_conditions(
        skill: &Skill,
        input: &SkillInput,
        ability_info: &AbilityInfo,
    ) -> Result<(), String> {
        // 檢查冷卻時間
        if !skill.is_ready() {
            return Err("技能尚在冷卻中".to_string());
        }

        // 檢查法力消耗（如果有法力系統）
        // TODO: 實現法力檢查

        // 檢查施放範圍
        if ability_info.cast_range > 0.0 {
            if let Some(target_pos) = input.target_position {
                // 需要位置資訊來檢查範圍
                // TODO: 實現範圍檢查
            }
        }

        Ok(())
    }

    /// 計算技能傷害
    pub fn calculate_skill_damage(
        base_damage: f32,
        skill_level: u32,
        caster_stats: &CProperty,
        caster_attack: &TAttack,
    ) -> f32 {
        let level_bonus = (skill_level - 1) as f32 * 0.2; // 每級+20%
        let attack_bonus = caster_attack.atk_physic.v * 0.5; // 50%攻擊力加成
        
        base_damage * (1.0 + level_bonus) + attack_bonus
    }

    fn get_sniper_mode_effects() -> Vec<SkillLevelEffect> {
        vec![
            SkillLevelEffect {
                level: 1,
                effects: HashMap::from([
                    ("range_bonus".to_string(), 200.0),
                    ("damage_bonus".to_string(), 0.25),
                    ("attack_speed_bonus".to_string(), -0.3),
                    ("move_speed_bonus".to_string(), -0.5),
                    ("accuracy_bonus".to_string(), 0.1),
                ]),
            },
            SkillLevelEffect {
                level: 2,
                effects: HashMap::from([
                    ("range_bonus".to_string(), 250.0),
                    ("damage_bonus".to_string(), 0.30),
                    ("attack_speed_bonus".to_string(), -0.3),
                    ("move_speed_bonus".to_string(), -0.5),
                    ("accuracy_bonus".to_string(), 0.15),
                ]),
            },
            SkillLevelEffect {
                level: 3,
                effects: HashMap::from([
                    ("range_bonus".to_string(), 300.0),
                    ("damage_bonus".to_string(), 0.35),
                    ("attack_speed_bonus".to_string(), -0.3),
                    ("move_speed_bonus".to_string(), -0.5),
                    ("accuracy_bonus".to_string(), 0.20),
                ]),
            },
            SkillLevelEffect {
                level: 4,
                effects: HashMap::from([
                    ("range_bonus".to_string(), 350.0),
                    ("damage_bonus".to_string(), 0.40),
                    ("attack_speed_bonus".to_string(), -0.3),
                    ("move_speed_bonus".to_string(), -0.5),
                    ("accuracy_bonus".to_string(), 0.25),
                ]),
            },
        ]
    }

    fn get_saika_reinforcements_effects() -> Vec<SkillLevelEffect> {
        vec![
            SkillLevelEffect {
                level: 1,
                effects: HashMap::from([
                    ("summon_count".to_string(), 1.0),
                    ("summon_duration".to_string(), 30.0),
                    ("summon_damage".to_string(), 50.0),
                ]),
            },
            SkillLevelEffect {
                level: 2,
                effects: HashMap::from([
                    ("summon_count".to_string(), 2.0),
                    ("summon_duration".to_string(), 35.0),
                    ("summon_damage".to_string(), 60.0),
                ]),
            },
            SkillLevelEffect {
                level: 3,
                effects: HashMap::from([
                    ("summon_count".to_string(), 3.0),
                    ("summon_duration".to_string(), 40.0),
                    ("summon_damage".to_string(), 70.0),
                ]),
            },
        ]
    }

    fn get_rain_iron_cannon_effects() -> Vec<SkillLevelEffect> {
        vec![
            SkillLevelEffect {
                level: 1,
                effects: HashMap::from([
                    ("base_damage".to_string(), 150.0),
                    ("area_radius".to_string(), 300.0),
                ]),
            },
            SkillLevelEffect {
                level: 2,
                effects: HashMap::from([
                    ("base_damage".to_string(), 200.0),
                    ("area_radius".to_string(), 350.0),
                ]),
            },
            SkillLevelEffect {
                level: 3,
                effects: HashMap::from([
                    ("base_damage".to_string(), 250.0),
                    ("area_radius".to_string(), 400.0),
                ]),
            },
        ]
    }

    fn get_three_stage_technique_effects() -> Vec<SkillLevelEffect> {
        vec![
            SkillLevelEffect {
                level: 1,
                effects: HashMap::from([
                    ("damage_per_hit".to_string(), 75.0),
                    ("hit_count".to_string(), 3.0),
                ]),
            },
            SkillLevelEffect {
                level: 2,
                effects: HashMap::from([
                    ("damage_per_hit".to_string(), 100.0),
                    ("hit_count".to_string(), 3.0),
                ]),
            },
            SkillLevelEffect {
                level: 3,
                effects: HashMap::from([
                    ("damage_per_hit".to_string(), 125.0),
                    ("hit_count".to_string(), 3.0),
                ]),
            },
            SkillLevelEffect {
                level: 4,
                effects: HashMap::from([
                    ("damage_per_hit".to_string(), 150.0),
                    ("hit_count".to_string(), 3.0),
                ]),
            },
        ]
    }
}

/// 技能資訊結構
#[derive(Debug, Clone)]
pub struct AbilityInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub ability_type: AbilityType,
    pub max_level: u32,
    pub mana_cost: Vec<f32>,
    pub cooldown: Vec<f32>,
    pub cast_range: f32,
    pub effects: Vec<SkillLevelEffect>,
}

/// 技能等級效果
#[derive(Debug, Clone)]
pub struct SkillLevelEffect {
    pub level: u32,
    pub effects: HashMap<String, f32>,
}

/// 技能類型
#[derive(Debug, Clone)]
pub enum AbilityType {
    Active,    // 主動技能
    Passive,   // 被動技能
    Toggle,    // 切換技能
}