use specs::storage::VecStorage;
use specs::{Component, Entity};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 英雄組件 - 包含英雄的基礎屬性和成長數據
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Hero {
    pub id: String,
    pub name: String,
    pub title: String,
    pub background: String,
    
    // 基礎屬性
    pub strength: i32,
    pub agility: i32,
    pub intelligence: i32,
    pub primary_attribute: AttributeType,
    
    // 當前等級和經驗
    pub level: i32,
    pub experience: i32,
    pub experience_to_next: i32,
    
    // 技能相關
    pub abilities: Vec<String>,  // ability IDs
    pub ability_levels: HashMap<String, i32>,  // 技能等級
    pub skill_points: i32,       // 可用技能點
    
    // 升級數據
    pub level_growth: LevelGrowth,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum AttributeType {
    Strength,
    Agility,
    Intelligence,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LevelGrowth {
    pub strength_per_level: f32,
    pub agility_per_level: f32,
    pub intelligence_per_level: f32,
    pub damage_per_level: f32,
    pub hp_per_level: f32,
    pub mana_per_level: f32,
}

impl Component for Hero {
    type Storage = VecStorage<Self>;
}

impl Hero {
    /// 創建新的英雄實例
    pub fn new(id: String, name: String, title: String) -> Self {
        Hero {
            id,
            name,
            title,
            background: String::new(),
            strength: 1,
            agility: 1,
            intelligence: 1,
            primary_attribute: AttributeType::Strength,
            level: 1,
            experience: 0,
            experience_to_next: 100,
            abilities: Vec::new(),
            ability_levels: HashMap::new(),
            skill_points: 0,
            level_growth: LevelGrowth::default(),
        }
    }
    
    /// 從戰役資料創建英雄
    pub fn from_campaign_data(hero_data: &crate::ue4::import_campaign::HeroJD) -> Self {
        let primary_attribute = match hero_data.primary_attribute.as_str() {
            "strength" => AttributeType::Strength,
            "agility" => AttributeType::Agility,
            "intelligence" => AttributeType::Intelligence,
            _ => AttributeType::Strength,
        };
        
        let mut ability_levels = HashMap::new();
        for ability_id in &hero_data.abilities {
            ability_levels.insert(ability_id.clone(), 0);
        }
        
        Hero {
            id: hero_data.id.clone(),
            name: hero_data.name.clone(),
            title: hero_data.title.clone(),
            background: hero_data.background.clone(),
            strength: hero_data.strength,
            agility: hero_data.agility,
            intelligence: hero_data.intelligence,
            primary_attribute,
            level: 1,
            experience: 0,
            experience_to_next: 100,
            abilities: hero_data.abilities.clone(),
            ability_levels,
            skill_points: 1, // 初始技能點
            level_growth: LevelGrowth {
                strength_per_level: hero_data.level_growth.strength_per_level,
                agility_per_level: hero_data.level_growth.agility_per_level,
                intelligence_per_level: hero_data.level_growth.intelligence_per_level,
                damage_per_level: hero_data.level_growth.damage_per_level,
                hp_per_level: hero_data.level_growth.hp_per_level,
                mana_per_level: hero_data.level_growth.mana_per_level,
            },
        }
    }
    
    /// 獲取當前總屬性值（基礎 + 等級成長）
    pub fn get_total_strength(&self) -> f32 {
        self.strength as f32 + (self.level - 1) as f32 * self.level_growth.strength_per_level
    }
    
    pub fn get_total_agility(&self) -> f32 {
        self.agility as f32 + (self.level - 1) as f32 * self.level_growth.agility_per_level
    }
    
    pub fn get_total_intelligence(&self) -> f32 {
        self.intelligence as f32 + (self.level - 1) as f32 * self.level_growth.intelligence_per_level
    }
    
    /// 獲取主屬性值
    pub fn get_primary_attribute_value(&self) -> f32 {
        match self.primary_attribute {
            AttributeType::Strength => self.get_total_strength(),
            AttributeType::Agility => self.get_total_agility(),
            AttributeType::Intelligence => self.get_total_intelligence(),
        }
    }
    
    /// 計算基礎攻擊力
    pub fn get_base_damage(&self) -> f32 {
        let base = self.get_primary_attribute_value();
        let level_bonus = (self.level - 1) as f32 * self.level_growth.damage_per_level;
        base + level_bonus
    }
    
    /// 計算最大生命值
    pub fn get_max_hp(&self) -> f32 {
        let str_bonus = self.get_total_strength() * 22.0; // 每點力量 +22 HP
        let level_bonus = (self.level - 1) as f32 * self.level_growth.hp_per_level;
        200.0 + str_bonus + level_bonus // 基礎 200 HP
    }
    
    /// 計算最大法力值
    pub fn get_max_mana(&self) -> f32 {
        let int_bonus = self.get_total_intelligence() * 13.0; // 每點智力 +13 MP
        let level_bonus = (self.level - 1) as f32 * self.level_growth.mana_per_level;
        75.0 + int_bonus + level_bonus // 基礎 75 MP
    }
    
    /// 計算攻擊速度倍數
    pub fn get_attack_speed_multiplier(&self) -> f32 {
        let agi_bonus = self.get_total_agility() * 0.01; // 每點敏捷 +1% 攻速
        1.0 + agi_bonus
    }
    
    /// 計算移動速度
    pub fn get_move_speed(&self) -> f32 {
        let agi_bonus = self.get_total_agility() * 0.05; // 每點敏捷 +0.05% 移速（微量）
        300.0 * (1.0 + agi_bonus / 100.0) // 基礎移速 300
    }
    
    /// 計算暴擊率
    pub fn get_crit_chance(&self) -> f32 {
        // 雜賀孫市作為敏捷英雄，敏捷提供暴擊率
        let agi_crit = self.get_total_agility() * 0.001; // 每點敏捷 +0.1% 暴擊率
        let base_crit = 0.05; // 基礎 5% 暴擊率
        (base_crit + agi_crit).min(0.75) // 暴擊率上限 75%
    }
    
    /// 計算閃避率
    pub fn get_dodge_chance(&self) -> f32 {
        // 高敏捷英雄有少量閃避率
        let agi_dodge = (self.get_total_agility() as f32 - 20.0).max(0.0) * 0.0005; // 超過20敏捷後每點 +0.05% 閃避
        agi_dodge.min(0.25) // 閃避率上限 25%
    }
    
    /// 增加經驗值
    pub fn add_experience(&mut self, exp: i32) -> bool {
        self.experience += exp;
        
        if self.experience >= self.experience_to_next {
            self.level_up();
            true
        } else {
            false
        }
    }
    
    /// 升級處理
    pub fn level_up(&mut self) {
        if self.level < 25 { // 最大等級限制
            self.level += 1;
            self.skill_points += 1;
            self.experience -= self.experience_to_next;
            
            // 計算下一級所需經驗 (指數增長)
            self.experience_to_next = (100.0 * (1.2_f32.powi(self.level - 1))) as i32;
        }
    }
    
    /// 學習技能
    pub fn learn_ability(&mut self, ability_id: &str) -> Result<(), String> {
        if self.skill_points <= 0 {
            return Err("No skill points available".to_string());
        }
        
        if !self.abilities.contains(&ability_id.to_string()) {
            return Err("Hero doesn't have this ability".to_string());
        }
        
        let current_level = *self.ability_levels.get(ability_id).unwrap_or(&0);
        if current_level >= 4 { // 最大技能等級
            return Err("Ability is already at maximum level".to_string());
        }
        
        // 終極技能等級限制
        if ability_id.starts_with("R") && current_level >= (self.level / 6).max(1) {
            return Err("Hero level too low for ultimate upgrade".to_string());
        }
        
        self.ability_levels.insert(ability_id.to_string(), current_level + 1);
        self.skill_points -= 1;
        
        Ok(())
    }
    
    /// 獲取技能等級
    pub fn get_ability_level(&self, ability_id: &str) -> i32 {
        *self.ability_levels.get(ability_id).unwrap_or(&0)
    }
    
    /// 檢查是否可以使用技能
    pub fn can_use_ability(&self, ability_id: &str) -> bool {
        self.get_ability_level(ability_id) > 0
    }
}

impl Default for LevelGrowth {
    fn default() -> Self {
        LevelGrowth {
            strength_per_level: 2.8,
            agility_per_level: 2.4,
            intelligence_per_level: 2.0,
            damage_per_level: 2.5,
            hp_per_level: 60.0,
            mana_per_level: 26.0,
        }
    }
}

impl Default for Hero {
    fn default() -> Self {
        Hero::new("unknown".to_string(), "Unknown Hero".to_string(), "The Nameless".to_string())
    }
}