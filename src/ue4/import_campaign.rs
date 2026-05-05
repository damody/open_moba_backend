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

/// Generated story hero entry — 全部 stats 都在 templates.lua generated data，
/// 此結構只剩 id（campaign 引用哪個 hero template）。可選保留
/// abilities 做 per-campaign override（例：訓練關只給 hero 一招）。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct HeroJD {
    pub id: String,
    /// 留 abilities 是因為 campaign 可能想 override templates.lua 的預設 4 ability
    /// 集合（例：訓練關只給 1 招），#[serde(default)] 沒寫就走 hero_abilities() lookup。
    #[serde(default)]
    pub abilities: Vec<String>,
}

/// 兼容舊 story source 的 level_growth nested struct。
/// **新流程不再從 story entity 讀此欄位**，改從 templates.lua
/// `heroes[i].level_growth` 讀，但結構體仍保留供 schema 兼容。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct LevelGrowthJD {
    pub strength_per_level: f32,
    pub agility_per_level: f32,
    pub intelligence_per_level: f32,
    pub damage_per_level: f32,
    pub hp_per_level: f32,
    pub mana_per_level: f32,
}

/// Generated story enemy entry — 全部 stats 在 templates.lua，只剩 id + abilities override。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct EnemyJD {
    pub id: String,
    #[serde(default)]
    pub abilities: Vec<String>,
}

/// Generated story creep entry — 全部 stats 在 templates.lua，只剩 id。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct CreepJD {
    pub id: String,
}

/// Generated story neutral entry — 全部 stats 在 templates.lua，只剩 id + abilities override。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct NeutralJD {
    pub id: String,
    #[serde(default)]
    pub abilities: Vec<String>,
}

/// Generated story summon entry — 全部 stats 在 templates.lua，只剩 id + summoner_ability
/// （tying campaign-specific：哪個技能召出此單位）。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct SummonJD {
    pub id: String,
    /// 哪個 ability 召出本 unit — 仍 campaign-specific
    #[serde(default)]
    pub summoner_ability: String,
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
    /// Load shipped campaign data from `omoba-template-ids` generated Rust data.
    /// Runtime gameplay does not read JSON or Lua content source files.
    pub fn load_generated(story_id: &str) -> Result<CampaignData, Box<dyn std::error::Error>> {
        let story = omoba_template_ids::story_by_name(story_id)
            .ok_or_else(|| format!("unknown generated story '{}'", story_id))?;
        Self::from_generated_story(story)
    }

    pub fn from_generated_story(
        story: &omoba_template_ids::GeneratedStory,
    ) -> Result<CampaignData, Box<dyn std::error::Error>> {
        let mut entity_value = story_value_to_json(story.entity);
        let ability_value = story_value_to_json(story.ability);
        let mut mission_value = story_value_to_json(story.mission);
        let mut map_value = story_value_to_json(story.map);
        normalize_entity_value(&mut entity_value);
        normalize_mission_value(&mut mission_value);
        normalize_map_value(&mut map_value);

        let entity: EntityData = serde_json::from_value(entity_value)?;
        let ability: AbilityData = serde_json::from_value(ability_value)?;
        let mission: MissionData = serde_json::from_value(mission_value)?;
        let map: super::import_map::CreepWaveData = serde_json::from_value(map_value)?;

        Ok(CampaignData {
            entity,
            ability,
            mission,
            map,
        })
    }

    /// Legacy JSON loader for migration tooling only. Runtime should use `load_generated`.
    /// 
    /// # Arguments
    /// * `campaign_path` - legacy JSON story folder path
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

fn story_value_to_json(value: omoba_template_ids::StoryValue) -> serde_json::Value {
    match value {
        omoba_template_ids::StoryValue::Null => serde_json::Value::Null,
        omoba_template_ids::StoryValue::Bool(value) => serde_json::Value::Bool(value),
        omoba_template_ids::StoryValue::Number(value) => json_number(value),
        omoba_template_ids::StoryValue::String(value) => serde_json::Value::String(value.to_string()),
        omoba_template_ids::StoryValue::Array(values) => {
            serde_json::Value::Array(values.iter().copied().map(story_value_to_json).collect())
        }
        omoba_template_ids::StoryValue::Object(values) => {
            let mut map = serde_json::Map::new();
            for (key, value) in values.iter().copied() {
                map.insert(key.to_string(), story_value_to_json(value));
            }
            serde_json::Value::Object(map)
        }
    }
}

fn normalize_entity_value(value: &mut serde_json::Value) {
    for key in ["heroes", "enemies", "creeps", "neutrals", "summons"] {
        ensure_array_field(value, key);
    }
}

fn normalize_mission_value(value: &mut serde_json::Value) {
    if let Some(campaign) = value.get_mut("campaign") {
        ensure_array_field(campaign, "unlock_requirements");
    }
    ensure_array_field(value, "stages");
    if let Some(stages) = value.get_mut("stages").and_then(serde_json::Value::as_array_mut) {
        for stage in stages {
            ensure_array_field(stage, "objectives");
            ensure_array_field(stage, "optional_objectives");
            if let Some(scoring) = stage.get_mut("scoring") {
                ensure_array_field(scoring, "star_thresholds");
            }
        }
    }
}

fn normalize_map_value(value: &mut serde_json::Value) {
    for key in ["Path", "Creep", "CheckPoint", "Tower", "CreepWave", "Structures", "BlockedRegions"] {
        ensure_array_field(value, key);
    }
    if let Some(waves) = value.get_mut("CreepWave").and_then(serde_json::Value::as_array_mut) {
        for wave in waves {
            ensure_array_field(wave, "Detail");
            if let Some(details) = wave.get_mut("Detail").and_then(serde_json::Value::as_array_mut) {
                for detail in details {
                    ensure_array_field(detail, "Creeps");
                }
            }
        }
    }
    if let Some(regions) = value.get_mut("BlockedRegions").and_then(serde_json::Value::as_array_mut) {
        for region in regions {
            ensure_array_field(region, "Points");
        }
    }
}

fn ensure_array_field(value: &mut serde_json::Value, key: &str) {
    let Some(object) = value.as_object_mut() else { return; };
    match object.get_mut(key) {
        Some(field) if field.as_object().is_some_and(serde_json::Map::is_empty) => {
            *field = serde_json::Value::Array(Vec::new());
        }
        None => {
            object.insert(key.to_string(), serde_json::Value::Array(Vec::new()));
        }
        _ => {}
    }
}

fn json_number(value: f64) -> serde_json::Value {
    if value.fract() == 0.0 && value >= i64::MIN as f64 && value <= i64::MAX as f64 {
        serde_json::Value::Number(serde_json::Number::from(value as i64))
    } else {
        serde_json::Number::from_f64(value)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null)
    }
}
