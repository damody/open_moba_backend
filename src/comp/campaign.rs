use specs::storage::VecStorage;
use specs::{Component, Entity};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 戰役組件 - 管理戰役狀態和進度
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Campaign {
    pub id: String,
    pub name: String,
    pub hero_id: String,
    pub description: String,
    pub difficulty: CampaignDifficulty,
    
    // 進度狀態
    pub current_stage: String,
    pub completed_stages: Vec<String>,
    pub stage_scores: HashMap<String, StageScore>,
    pub total_score: i32,
    pub total_stars: i32,
    
    // 解鎖條件
    pub unlock_requirements: Vec<String>,
    pub is_unlocked: bool,
    
    // 統計資料
    pub play_time: f32,
    pub deaths: i32,
    pub kills: i32,
    pub damage_dealt: i32,
    pub damage_taken: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum CampaignDifficulty {
    Tutorial,  // 教學
    Easy,      // 簡單
    Normal,    // 普通
    Hard,      // 困難
    Expert,    // 專家
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StageScore {
    pub stars: i32,        // 星級評分 (0-3)
    pub score: i32,        // 具體分數
    pub best_time: f32,    // 最佳完成時間
    pub completion_count: i32, // 完成次數
}

impl Component for Campaign {
    type Storage = VecStorage<Self>;
}

/// 關卡組件 - 管理單個關卡的狀態
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Stage {
    pub id: String,
    pub name: String,
    pub stage_type: StageType,
    pub campaign_id: String,
    
    // 狀態管理
    pub is_active: bool,
    pub is_completed: bool,
    pub start_time: f32,
    pub elapsed_time: f32,
    pub time_limit: Option<f32>,
    
    // 目標系統
    pub objectives: Vec<Objective>,
    pub optional_objectives: Vec<Objective>,
    pub completed_objectives: Vec<String>,
    
    // 評分系統
    pub scoring: StageScoring,
    pub current_score: i32,
    
    // 環境設定
    pub environment: StageEnvironment,
    
    // UI 設定
    pub ui_settings: StageUiSettings,
    
    // 動態狀態
    pub paused: bool,
    pub can_pause: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum StageType {
    Training,  // 訓練關卡
    Combat,    // 戰鬥關卡  
    Puzzle,    // 解謎關卡
    Boss,      // Boss戰關卡
    Survival,  // 生存關卡
    Escort,    // 護送關卡
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Objective {
    pub id: String,
    pub description: String,
    pub objective_type: ObjectiveType,
    pub target: String,
    pub current_count: i32,
    pub target_count: i32,
    pub is_completed: bool,
    pub is_optional: bool,
    pub condition: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum ObjectiveType {
    Kill,      // 擊殺目標
    Survive,   // 生存時間
    Protect,   // 保護目標
    Reach,     // 到達位置
    Collect,   // 收集物品
    Score,     // 達到分數
    Time,      // 時間限制
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StageScoring {
    pub max_stars: i32,
    pub star_thresholds: Vec<i32>,
    pub scoring_factors: HashMap<String, i32>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StageEnvironment {
    pub weather: Option<String>,
    pub time_of_day: String,
    pub wind: Option<WindEffect>,
    pub visibility: f32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WindEffect {
    pub direction: f32,  // 風向角度 (0-360)
    pub strength: f32,   // 風力強度
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StageUiSettings {
    pub show_minimap: bool,
    pub show_hero_stats: bool,
    pub show_ability_cooldowns: bool,
    pub show_damage_numbers: bool,
    pub enable_pause: bool,
    pub camera_mode: String,
}

impl Component for Stage {
    type Storage = VecStorage<Self>;
}

impl Campaign {
    /// 創建新的戰役實例
    pub fn new(id: String, name: String, hero_id: String) -> Self {
        Campaign {
            id,
            name,
            hero_id,
            description: String::new(),
            difficulty: CampaignDifficulty::Normal,
            current_stage: String::new(),
            completed_stages: Vec::new(),
            stage_scores: HashMap::new(),
            total_score: 0,
            total_stars: 0,
            unlock_requirements: Vec::new(),
            is_unlocked: false,
            play_time: 0.0,
            deaths: 0,
            kills: 0,
            damage_dealt: 0,
            damage_taken: 0,
        }
    }
    
    /// 從戰役資料創建戰役
    pub fn from_campaign_data(campaign_data: &crate::ue4::import_campaign::CampaignInfoJD) -> Self {
        let difficulty = match campaign_data.difficulty.as_str() {
            "tutorial" => CampaignDifficulty::Tutorial,
            "easy" => CampaignDifficulty::Easy,
            "normal" => CampaignDifficulty::Normal,
            "hard" => CampaignDifficulty::Hard,
            "expert" => CampaignDifficulty::Expert,
            _ => CampaignDifficulty::Normal,
        };
        
        Campaign {
            id: campaign_data.id.clone(),
            name: campaign_data.name.clone(),
            hero_id: campaign_data.hero_id.clone(),
            description: campaign_data.description.clone(),
            difficulty,
            current_stage: String::new(),
            completed_stages: Vec::new(),
            stage_scores: HashMap::new(),
            total_score: 0,
            total_stars: 0,
            unlock_requirements: campaign_data.unlock_requirements.clone(),
            is_unlocked: false,
            play_time: 0.0,
            deaths: 0,
            kills: 0,
            damage_dealt: 0,
            damage_taken: 0,
        }
    }
    
    /// 開始關卡
    pub fn start_stage(&mut self, stage_id: String) {
        self.current_stage = stage_id;
    }
    
    /// 完成關卡
    pub fn complete_stage(&mut self, stage_id: String, score: StageScore) {
        if !self.completed_stages.contains(&stage_id) {
            self.completed_stages.push(stage_id.clone());
        }
        
        self.stage_scores.insert(stage_id, score.clone());
        self.total_score += score.score;
        self.total_stars += score.stars;
    }
    
    /// 檢查關卡是否已完成
    pub fn is_stage_completed(&self, stage_id: &str) -> bool {
        self.completed_stages.contains(&stage_id.to_string())
    }
    
    /// 獲取關卡分數
    pub fn get_stage_score(&self, stage_id: &str) -> Option<&StageScore> {
        self.stage_scores.get(stage_id)
    }
    
    /// 更新統計資料
    pub fn add_kill(&mut self) {
        self.kills += 1;
    }
    
    pub fn add_death(&mut self) {
        self.deaths += 1;
    }
    
    pub fn add_damage_dealt(&mut self, damage: i32) {
        self.damage_dealt += damage;
    }
    
    pub fn add_damage_taken(&mut self, damage: i32) {
        self.damage_taken += damage;
    }
    
    pub fn add_play_time(&mut self, time: f32) {
        self.play_time += time;
    }
    
    /// 獲取完成進度百分比
    pub fn get_completion_percentage(&self, total_stages: i32) -> f32 {
        if total_stages > 0 {
            (self.completed_stages.len() as f32 / total_stages as f32) * 100.0
        } else {
            0.0
        }
    }
}

impl Stage {
    /// 創建新的關卡實例
    pub fn new(id: String, name: String, stage_type: StageType, campaign_id: String) -> Self {
        Stage {
            id,
            name,
            stage_type,
            campaign_id,
            is_active: false,
            is_completed: false,
            start_time: 0.0,
            elapsed_time: 0.0,
            time_limit: None,
            objectives: Vec::new(),
            optional_objectives: Vec::new(),
            completed_objectives: Vec::new(),
            scoring: StageScoring {
                max_stars: 3,
                star_thresholds: vec![100, 200, 300],
                scoring_factors: HashMap::new(),
            },
            current_score: 0,
            environment: StageEnvironment {
                weather: None,
                time_of_day: "day".to_string(),
                wind: None,
                visibility: 1.0,
            },
            ui_settings: StageUiSettings {
                show_minimap: true,
                show_hero_stats: true,
                show_ability_cooldowns: true,
                show_damage_numbers: true,
                enable_pause: true,
                camera_mode: "follow".to_string(),
            },
            paused: false,
            can_pause: true,
        }
    }
    
    /// 從戰役資料創建關卡
    pub fn from_campaign_data(stage_data: &crate::ue4::import_campaign::StageJD, campaign_id: String) -> Self {
        let stage_type = match stage_data.stage_type.as_str() {
            "training" => StageType::Training,
            "combat" => StageType::Combat,
            "puzzle" => StageType::Puzzle,
            "boss" => StageType::Boss,
            "survival" => StageType::Survival,
            "escort" => StageType::Escort,
            _ => StageType::Training,
        };
        
        let mut objectives = Vec::new();
        for obj_data in &stage_data.objectives {
            objectives.push(Objective::from_campaign_data(obj_data, false));
        }
        
        let mut optional_objectives = Vec::new();
        for obj_data in &stage_data.optional_objectives {
            optional_objectives.push(Objective::from_campaign_data(obj_data, true));
        }
        
        Stage {
            id: stage_data.id.clone(),
            name: stage_data.name.clone(),
            stage_type,
            campaign_id,
            is_active: false,
            is_completed: false,
            start_time: 0.0,
            elapsed_time: 0.0,
            time_limit: stage_data.time_limit,
            objectives,
            optional_objectives,
            completed_objectives: Vec::new(),
            scoring: StageScoring {
                max_stars: stage_data.scoring.max_stars,
                star_thresholds: stage_data.scoring.star_thresholds.clone(),
                scoring_factors: stage_data.scoring.scoring_factors.clone(),
            },
            current_score: 0,
            environment: StageEnvironment {
                weather: stage_data.environment.weather.clone(),
                time_of_day: stage_data.environment.time_of_day.clone(),
                wind: stage_data.environment.wind.as_ref().map(|w| WindEffect {
                    direction: w.direction,
                    strength: w.strength,
                }),
                visibility: stage_data.environment.visibility,
            },
            ui_settings: StageUiSettings {
                show_minimap: stage_data.ui_settings.show_minimap,
                show_hero_stats: stage_data.ui_settings.show_hero_stats,
                show_ability_cooldowns: stage_data.ui_settings.show_ability_cooldowns,
                show_damage_numbers: stage_data.ui_settings.show_damage_numbers,
                enable_pause: stage_data.ui_settings.enable_pause,
                camera_mode: stage_data.ui_settings.camera_mode.clone(),
            },
            paused: false,
            can_pause: stage_data.ui_settings.enable_pause,
        }
    }
    
    /// 開始關卡
    pub fn start(&mut self, current_time: f32) {
        self.is_active = true;
        self.start_time = current_time;
        self.elapsed_time = 0.0;
        self.paused = false;
    }
    
    /// 更新關卡時間
    pub fn update(&mut self, current_time: f32, delta_time: f32) {
        if self.is_active && !self.paused {
            self.elapsed_time = current_time - self.start_time;
        }
    }
    
    /// 暫停/恢復關卡
    pub fn toggle_pause(&mut self) {
        if self.can_pause {
            self.paused = !self.paused;
        }
    }
    
    /// 完成目標
    pub fn complete_objective(&mut self, objective_id: &str) -> bool {
        // 檢查主要目標
        for obj in &mut self.objectives {
            if obj.id == objective_id && !obj.is_completed {
                obj.is_completed = true;
                self.completed_objectives.push(objective_id.to_string());
                return true;
            }
        }
        
        // 檢查可選目標
        for obj in &mut self.optional_objectives {
            if obj.id == objective_id && !obj.is_completed {
                obj.is_completed = true;
                self.completed_objectives.push(objective_id.to_string());
                return true;
            }
        }
        
        false
    }
    
    /// 檢查是否所有主要目標都已完成
    pub fn all_objectives_completed(&self) -> bool {
        self.objectives.iter().all(|obj| obj.is_completed)
    }
    
    /// 檢查是否超時
    pub fn is_overtime(&self) -> bool {
        if let Some(limit) = self.time_limit {
            self.elapsed_time > limit
        } else {
            false
        }
    }
    
    /// 計算當前星級
    pub fn calculate_stars(&self) -> i32 {
        for (i, &threshold) in self.scoring.star_thresholds.iter().enumerate().rev() {
            if self.current_score >= threshold {
                return (i + 1) as i32;
            }
        }
        0
    }
    
    /// 添加分數
    pub fn add_score(&mut self, points: i32) {
        self.current_score += points;
    }
}

impl Objective {
    pub fn from_campaign_data(obj_data: &crate::ue4::import_campaign::ObjectiveJD, is_optional: bool) -> Self {
        let objective_type = match obj_data.objective_type.as_str() {
            "kill" => ObjectiveType::Kill,
            "survive" => ObjectiveType::Survive,
            "protect" => ObjectiveType::Protect,
            "reach" => ObjectiveType::Reach,
            "collect" => ObjectiveType::Collect,
            "score" => ObjectiveType::Score,
            "time" => ObjectiveType::Time,
            _ => ObjectiveType::Kill,
        };
        
        Objective {
            id: obj_data.id.clone(),
            description: obj_data.description.clone(),
            objective_type,
            target: obj_data.target.clone(),
            current_count: 0,
            target_count: obj_data.count.unwrap_or(1),
            is_completed: false,
            is_optional,
            condition: obj_data.condition.clone(),
        }
    }
    
    /// 更新目標進度
    pub fn update_progress(&mut self, count: i32) -> bool {
        self.current_count = count;
        let was_completed = self.is_completed;
        self.is_completed = self.current_count >= self.target_count;
        !was_completed && self.is_completed // 剛完成時返回true
    }
    
    /// 獲取完成百分比
    pub fn get_progress_percentage(&self) -> f32 {
        if self.target_count > 0 {
            (self.current_count as f32 / self.target_count as f32).min(1.0)
        } else {
            if self.is_completed { 1.0 } else { 0.0 }
        }
    }
}

impl Default for Campaign {
    fn default() -> Self {
        Campaign::new("unknown".to_string(), "Unknown Campaign".to_string(), "unknown_hero".to_string())
    }
}

impl Default for Stage {
    fn default() -> Self {
        Stage::new("unknown".to_string(), "Unknown Stage".to_string(), StageType::Training, "unknown_campaign".to_string())
    }
}