use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use crate::types::*;

/// 技能配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbilityConfig {
    pub id: String,
    pub name: String,
    pub description: String,
    pub icon: Option<String>,
    
    /// 基礎屬性
    pub ability_type: AbilityType,
    pub target_type: TargetType,
    pub cast_type: CastType,
    
    /// 數值配置（按等級）
    pub levels: Vec<AbilityLevelData>,
    
    /// 技能效果配置
    pub effects: Vec<AbilityEffect>,
    
    /// 條件配置
    pub conditions: Vec<Condition>,
    
    /// 自定義參數
    pub custom_params: HashMap<String, serde_json::Value>,
}

/// 技能等級數據
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbilityLevelData {
    pub level: i32,
    pub cooldown: f32,
    pub mana_cost: f32,
    pub cast_time: f32,
    pub range: f32,
    pub damage: Option<f32>,
    pub duration: Option<f32>,
    pub radius: Option<f32>,
    pub charges: Option<i32>,
    pub custom_values: HashMap<String, f32>,
}

/// 技能類型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AbilityType {
    Active,        // 主動技能
    Passive,       // 被動技能
    Toggle,        // 切換技能
    Channeled,     // 引導技能
    Ultimate,      // 大招
}

/// 目標類型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TargetType {
    None,          // 無目標
    Unit,          // 單體目標
    Point,         // 地面點目標
    Direction,     // 方向目標
    Area,          // 範圍目標
    Self_,         // 自身
}

/// 施法類型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CastType {
    Instant,       // 瞬發
    Channeled,     // 引導
    Charged,       // 蓄力
    Toggle,        // 切換
}

/// 配置管理器
pub struct ConfigManager {
    configs: HashMap<String, AbilityConfig>,
}

impl ConfigManager {
    pub fn new() -> Self {
        Self {
            configs: HashMap::new(),
        }
    }
    
    /// 從文件載入配置
    pub fn load_from_file(&mut self, path: &str) -> Result<(), anyhow::Error> {
        let content = std::fs::read_to_string(path)?;
        
        if path.ends_with(".yaml") || path.ends_with(".yml") {
            self.load_from_yaml(&content)?;
        } else if path.ends_with(".json") {
            self.load_from_json(&content)?;
        } else {
            return Err(anyhow::anyhow!("不支援的檔案格式"));
        }
        
        Ok(())
    }
    
    /// 從YAML載入
    pub fn load_from_yaml(&mut self, content: &str) -> Result<(), anyhow::Error> {
        let configs: HashMap<String, AbilityConfig> = serde_yaml::from_str(content)?;
        
        for (id, mut config) in configs {
            config.id = id.clone();
            self.configs.insert(id, config);
        }
        
        Ok(())
    }
    
    /// 從JSON載入
    pub fn load_from_json(&mut self, content: &str) -> Result<(), anyhow::Error> {
        let configs: HashMap<String, AbilityConfig> = serde_json::from_str(content)?;
        
        for (id, mut config) in configs {
            config.id = id.clone();
            self.configs.insert(id, config);
        }
        
        Ok(())
    }
    
    /// 獲取配置
    pub fn get_config(&self, ability_id: &str) -> Option<&AbilityConfig> {
        self.configs.get(ability_id)
    }
    
    /// 註冊配置
    pub fn register_config(&mut self, config: AbilityConfig) {
        self.configs.insert(config.id.clone(), config);
    }
    
    /// 獲取所有配置
    pub fn get_all_configs(&self) -> &HashMap<String, AbilityConfig> {
        &self.configs
    }
}

impl Default for ConfigManager {
    fn default() -> Self {
        Self::new()
    }
}

impl AbilityConfig {
    /// 獲取指定等級的數據
    pub fn get_level_data(&self, level: i32) -> Option<&AbilityLevelData> {
        self.levels.iter().find(|data| data.level == level)
    }
    
    /// 獲取最大等級
    pub fn max_level(&self) -> i32 {
        self.levels.iter().map(|data| data.level).max().unwrap_or(1)
    }
    
    /// 獲取自定義參數
    pub fn get_custom_param<T>(&self, key: &str) -> Option<T>
    where
        T: serde::de::DeserializeOwned,
    {
        self.custom_params.get(key)
            .and_then(|value| serde_json::from_value(value.clone()).ok())
    }
}

impl AbilityLevelData {
    /// 獲取自定義數值
    pub fn get_custom_value(&self, key: &str) -> Option<f32> {
        self.custom_values.get(key).copied()
    }
}