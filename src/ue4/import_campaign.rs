use serde::{Deserialize, Serialize};

// ===== 戰役系統資料結構 =====
// 用於載入完整戰役資料，包含單位、技能、任務和地圖

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct CampaignData {
    pub entity: EntityData,
    pub ability: AbilityData, 
    pub mission: MissionData,
    pub map: super::import_map::CreepWaveData,  // 重用地圖資料結構
}

// ===== 單位資料結構 =====
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct EntityData {
    pub heroes: Vec<HeroJD>,
    pub enemies: Vec<EnemyJD>,
    pub creeps: Vec<CreepJD>,
    pub neutrals: Vec<NeutralJD>,
    pub summons: Vec<SummonJD>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct HeroJD {
    pub id: String,
    pub name: String,
    pub title: String,
    pub background: String,
    
    // 基礎屬性
    pub strength: i32,
    pub agility: i32,
    pub intelligence: i32,
    pub primary_attribute: String,
    
    // 戰鬥屬性
    pub attack_range: f32,
    pub base_damage: i32,
    pub base_armor: f32,
    pub base_hp: i32,
    pub base_mana: i32,
    pub move_speed: f32,
    
    // 技能引用
    pub abilities: Vec<String>,  // ability IDs
    
    // 升級數據
    pub level_growth: LevelGrowthJD,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct LevelGrowthJD {
    pub strength_per_level: f32,
    pub agility_per_level: f32,
    pub intelligence_per_level: f32,
    pub damage_per_level: f32,
    pub hp_per_level: f32,
    pub mana_per_level: f32,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct EnemyJD {
    pub id: String,
    pub name: String,
    pub enemy_type: String,  // melee, ranged, caster, boss
    
    // 戰鬥屬性
    pub hp: i32,
    pub armor: f32,
    pub magic_resistance: f32,
    pub damage: i32,
    pub attack_range: f32,
    pub move_speed: f32,
    
    // AI 行為
    pub ai_type: String,     // aggressive, defensive, patrol
    pub abilities: Vec<String>,
    
    // 獎勵
    pub exp_reward: i32,
    pub gold_reward: i32,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct CreepJD {
    pub id: String,
    pub name: String,
    pub hp: i32,
    pub armor: f32,
    pub damage: i32,
    pub move_speed: f32,
    pub gold_reward: i32,
    pub bounty_type: String,  // normal, siege, super
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct NeutralJD {
    pub id: String,
    pub name: String,
    pub hp: i32,
    pub armor: f32,
    pub damage: i32,
    pub move_speed: f32,
    pub respawn_time: f32,
    pub abilities: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct SummonJD {
    pub id: String,
    pub name: String,
    pub hp: i32,
    pub damage: i32,
    pub duration: f32,
    pub move_speed: f32,
    pub summoner_ability: String,  // 召喚技能ID
}

// ===== 技能資料結構 =====
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct AbilityData {
    pub abilities: std::collections::HashMap<String, AbilityJD>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct AbilityJD {
    pub id: String,
    pub name: String,
    pub description: String,
    pub ability_type: String,  // active, passive, ultimate
    pub key_binding: String,   // Q, W, E, R, T, etc.
    
    // 基礎屬性
    pub cooldown: Vec<f32>,    // 各等級冷卻時間
    #[serde(rename = "manaCost")]
    pub mana_cost: Vec<i32>,   // 各等級法力消耗
    pub cast_range: Vec<f32>,  // 各等級施法距離
    pub cast_time: f32,        // 施法時間
    
    // 效果參數
    pub effects: std::collections::HashMap<String, serde_json::Value>,
    
    // 技能互動
    pub dispellable: bool,     // 是否可驅散
    pub pierces_immunity: bool, // 是否穿透魔免
    pub affects_buildings: bool, // 是否影響建築
}

// ===== 任務資料結構 =====
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct MissionData {
    pub campaign: CampaignInfoJD,
    pub stages: Vec<StageJD>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct CampaignInfoJD {
    pub id: String,
    pub name: String,
    pub hero_id: String,
    pub description: String,
    pub difficulty: String,   // tutorial, easy, normal, hard
    pub unlock_requirements: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct StageJD {
    pub id: String,
    pub name: String,
    pub stage_type: String,   // training, combat, puzzle, boss
    pub time_limit: Option<f32>,
    
    // 目標設定
    pub objectives: Vec<ObjectiveJD>,
    pub optional_objectives: Vec<ObjectiveJD>,
    
    // 評分系統
    pub scoring: ScoringJD,
    
    // 環境設定
    pub environment: EnvironmentJD,
    
    // UI 設定
    pub ui_settings: UiSettingsJD,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ObjectiveJD {
    pub id: String,
    pub description: String,
    pub objective_type: String,  // kill, survive, protect, reach
    pub target: String,          // 目標對象或位置
    pub count: Option<i32>,      // 數量要求
    pub condition: Option<String>, // 額外條件
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ScoringJD {
    pub max_stars: i32,
    pub star_thresholds: Vec<i32>,  // 星級門檻分數
    pub scoring_factors: std::collections::HashMap<String, i32>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct EnvironmentJD {
    pub weather: Option<String>,   // sunny, rainy, foggy
    pub time_of_day: String,       // day, night, dawn, dusk
    pub wind: Option<WindJD>,      // 風向效果（影響投射物）
    pub visibility: f32,           // 視野範圍倍數
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct WindJD {
    pub direction: f32,  // 風向角度 (0-360)
    pub strength: f32,   // 風力強度
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct UiSettingsJD {
    pub show_minimap: bool,
    pub show_hero_stats: bool,
    pub show_ability_cooldowns: bool,
    pub show_damage_numbers: bool,
    pub enable_pause: bool,
    pub camera_mode: String,  // fixed, follow, free
}

// ===== 載入函數 =====
impl CampaignData {
    /// 從指定目錄載入完整戰役資料
    /// 
    /// # Arguments
    /// * `campaign_path` - 戰役資料夾路徑 (例如: "Story/B01_1/")
    /// 
    /// # Returns
    /// * `Result<CampaignData, Box<dyn std::error::Error>>` - 載入結果
    pub fn load_from_path(campaign_path: &str) -> Result<CampaignData, Box<dyn std::error::Error>> {
        use crate::json_preprocessor::JsonPreprocessor;
        
        let entity_path = format!("{}/entity.json", campaign_path);
        let ability_path = format!("{}/ability.json", campaign_path);
        let mission_path = format!("{}/mission.json", campaign_path);
        let map_path = format!("{}/map.json", campaign_path);
        
        let entity: EntityData = JsonPreprocessor::read_json_with_comments(&entity_path)?;
        let ability: AbilityData = JsonPreprocessor::read_json_with_comments(&ability_path)?;
        let mission: MissionData = JsonPreprocessor::read_json_with_comments(&mission_path)?;
        let map: super::import_map::CreepWaveData = JsonPreprocessor::read_json_with_comments(&map_path)?;
        
        Ok(CampaignData {
            entity,
            ability,
            mission,
            map,
        })
    }
    
    /// 獲取指定英雄的資料
    pub fn get_hero(&self, hero_id: &str) -> Option<&HeroJD> {
        self.entity.heroes.iter().find(|h| h.id == hero_id)
    }
    
    /// 獲取指定技能的資料
    pub fn get_ability(&self, ability_id: &str) -> Option<&AbilityJD> {
        self.ability.abilities.get(ability_id)
    }
    
    /// 獲取指定關卡的資料
    pub fn get_stage(&self, stage_id: &str) -> Option<&StageJD> {
        self.mission.stages.iter().find(|s| s.id == stage_id)
    }
    
    /// 驗證戰役資料完整性
    pub fn validate(&self) -> Result<(), String> {
        // 檢查英雄技能引用
        for hero in &self.entity.heroes {
            for ability_id in &hero.abilities {
                if !self.ability.abilities.contains_key(ability_id) {
                    return Err(format!("Hero {} references unknown ability: {}", hero.id, ability_id));
                }
            }
        }
        
        // 檢查敵人技能引用
        for enemy in &self.entity.enemies {
            for ability_id in &enemy.abilities {
                if !self.ability.abilities.contains_key(ability_id) {
                    return Err(format!("Enemy {} references unknown ability: {}", enemy.id, ability_id));
                }
            }
        }
        
        // 檢查關卡英雄引用
        if let Some(hero) = self.entity.heroes.iter().find(|h| h.id == self.mission.campaign.hero_id) {
            // 英雄存在，檢查通過
        } else {
            return Err(format!("Campaign references unknown hero: {}", self.mission.campaign.hero_id));
        }
        
        Ok(())
    }
}