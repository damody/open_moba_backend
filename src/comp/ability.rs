use specs::storage::VecStorage;
use specs::{Component, Entity};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 技能組件 - 包含技能的基礎資訊和狀態
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Ability {
    pub id: String,
    pub name: String,
    pub description: String,
    pub ability_type: AbilityType,
    pub key_binding: String,
    
    // 當前狀態
    pub current_level: i32,
    pub max_level: i32,
    pub cooldown_remaining: f32,
    pub is_on_cooldown: bool,
    
    // 基礎屬性
    pub cooldown: Vec<f32>,     // 各等級冷卻時間
    pub mana_cost: Vec<i32>,    // 各等級法力消耗
    pub cast_range: Vec<f32>,   // 各等級施法距離
    pub cast_time: f32,         // 施法時間
    
    // 效果參數
    pub effects: HashMap<String, serde_json::Value>,
    
    // 技能互動
    pub dispellable: bool,      // 是否可驅散  
    pub pierces_immunity: bool, // 是否穿透魔免
    pub affects_buildings: bool, // 是否影響建築
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum AbilityType {
    Active,    // 主動技能
    Passive,   // 被動技能
    Ultimate,  // 終極技能
    Item,      // 物品技能
}

impl Component for Ability {
    type Storage = VecStorage<Self>;
}

impl Ability {
    /// 創建新的技能實例
    pub fn new(id: String, name: String, ability_type: AbilityType) -> Self {
        Ability {
            id,
            name,
            description: String::new(),
            ability_type,
            key_binding: String::new(),
            current_level: 0,
            max_level: 4,
            cooldown_remaining: 0.0,
            is_on_cooldown: false,
            cooldown: vec![0.0; 4],
            mana_cost: vec![0; 4],
            cast_range: vec![0.0; 4],
            cast_time: 0.0,
            effects: HashMap::new(),
            dispellable: true,
            pierces_immunity: false,
            affects_buildings: false,
        }
    }
    
    /// 從戰役資料創建技能
    pub fn from_campaign_data(ability_data: &crate::ue4::import_campaign::AbilityJD) -> Self {
        let ability_type = match ability_data.ability_type.as_str() {
            "active" => AbilityType::Active,
            "passive" => AbilityType::Passive,
            "ultimate" => AbilityType::Ultimate,
            "item" => AbilityType::Item,
            _ => AbilityType::Active,
        };
        
        Ability {
            id: ability_data.id.clone(),
            name: ability_data.name.clone(),
            description: ability_data.description.clone(),
            ability_type,
            key_binding: ability_data.key_binding.clone(),
            current_level: 0,
            max_level: ability_data.cooldown.len() as i32,
            cooldown_remaining: 0.0,
            is_on_cooldown: false,
            cooldown: ability_data.cooldown.clone(),
            mana_cost: ability_data.mana_cost.clone(),
            cast_range: ability_data.cast_range.clone(),
            cast_time: ability_data.cast_time,
            effects: ability_data.effects.clone(),
            dispellable: ability_data.dispellable,
            pierces_immunity: ability_data.pierces_immunity,
            affects_buildings: ability_data.affects_buildings,
        }
    }
    
    /// 獲取當前等級的冷卻時間
    pub fn get_cooldown(&self) -> f32 {
        if self.current_level > 0 && (self.current_level as usize) <= self.cooldown.len() {
            self.cooldown[(self.current_level - 1) as usize]
        } else {
            0.0
        }
    }
    
    /// 獲取當前等級的法力消耗
    pub fn get_mana_cost(&self) -> i32 {
        if self.current_level > 0 && (self.current_level as usize) <= self.mana_cost.len() {
            self.mana_cost[(self.current_level - 1) as usize]
        } else {
            0
        }
    }
    
    /// 獲取當前等級的施法距離
    pub fn get_cast_range(&self) -> f32 {
        if self.current_level > 0 && (self.current_level as usize) <= self.cast_range.len() {
            self.cast_range[(self.current_level - 1) as usize]
        } else {
            0.0
        }
    }
    
    /// 獲取指定效果的數值
    pub fn get_effect_value(&self, effect_name: &str) -> Option<&serde_json::Value> {
        self.effects.get(effect_name)
    }
    
    /// 獲取指定效果在當前等級的數值（如果是陣列）
    pub fn get_effect_value_at_level(&self, effect_name: &str) -> Option<serde_json::Value> {
        if let Some(effect) = self.effects.get(effect_name) {
            match effect {
                serde_json::Value::Array(arr) => {
                    if self.current_level > 0 && (self.current_level as usize) <= arr.len() {
                        Some(arr[(self.current_level - 1) as usize].clone())
                    } else {
                        None
                    }
                },
                _ => Some(effect.clone()),
            }
        } else {
            None
        }
    }
    
    /// 檢查技能是否可以使用
    pub fn can_cast(&self, current_mana: i32) -> Result<(), String> {
        if self.current_level <= 0 {
            return Err("Ability not learned".to_string());
        }
        
        if self.is_on_cooldown {
            return Err(format!("Ability on cooldown: {:.1}s", self.cooldown_remaining));
        }
        
        let required_mana = self.get_mana_cost();
        if current_mana < required_mana {
            return Err(format!("Not enough mana: {}/{}", current_mana, required_mana));
        }
        
        Ok(())
    }
    
    /// 開始冷卻
    pub fn start_cooldown(&mut self) {
        self.cooldown_remaining = self.get_cooldown();
        self.is_on_cooldown = self.cooldown_remaining > 0.0;
    }
    
    /// 更新冷卻時間
    pub fn update_cooldown(&mut self, delta_time: f32) {
        if self.is_on_cooldown {
            self.cooldown_remaining -= delta_time;
            if self.cooldown_remaining <= 0.0 {
                self.cooldown_remaining = 0.0;
                self.is_on_cooldown = false;
            }
        }
    }
    
    /// 升級技能
    pub fn level_up(&mut self) -> Result<(), String> {
        if self.current_level >= self.max_level {
            return Err("Ability is already at maximum level".to_string());
        }
        
        self.current_level += 1;
        Ok(())
    }
    
    /// 重置冷卻時間（用於道具或特殊效果）
    pub fn reset_cooldown(&mut self) {
        self.cooldown_remaining = 0.0;
        self.is_on_cooldown = false;
    }
    
    /// 獲取冷卻百分比
    pub fn get_cooldown_percentage(&self) -> f32 {
        if !self.is_on_cooldown {
            return 0.0;
        }
        
        let max_cooldown = self.get_cooldown();
        if max_cooldown > 0.0 {
            (self.cooldown_remaining / max_cooldown).min(1.0)
        } else {
            0.0
        }
    }
    
    /// 檢查是否為被動技能
    pub fn is_passive(&self) -> bool {
        matches!(self.ability_type, AbilityType::Passive)
    }
    
    /// 檢查是否為終極技能
    pub fn is_ultimate(&self) -> bool {
        matches!(self.ability_type, AbilityType::Ultimate)
    }
    
    /// 檢查是否為主動技能
    pub fn is_active(&self) -> bool {
        matches!(self.ability_type, AbilityType::Active | AbilityType::Ultimate)
    }
}

impl Default for Ability {
    fn default() -> Self {
        Ability::new("unknown".to_string(), "Unknown Ability".to_string(), AbilityType::Active)
    }
}

/// 技能效果組件 - 表示正在生效的技能效果
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AbilityEffect {
    pub ability_id: String,
    pub caster_entity: Option<Entity>,
    pub target_entity: Option<Entity>,
    pub effect_type: String,  // buff, debuff, damage, heal
    pub duration: f32,
    pub remaining_time: f32,
    pub stacks: i32,          // 疊加層數
    pub effect_data: HashMap<String, serde_json::Value>,
}

impl Component for AbilityEffect {
    type Storage = VecStorage<Self>;
}

impl AbilityEffect {
    pub fn new(ability_id: String, duration: f32) -> Self {
        AbilityEffect {
            ability_id,
            caster_entity: None,
            target_entity: None,
            effect_type: "buff".to_string(),
            duration,
            remaining_time: duration,
            stacks: 1,
            effect_data: HashMap::new(),
        }
    }
    
    /// 更新效果持續時間
    pub fn update(&mut self, delta_time: f32) -> bool {
        self.remaining_time -= delta_time;
        self.remaining_time > 0.0
    }
    
    /// 檢查效果是否過期
    pub fn is_expired(&self) -> bool {
        self.remaining_time <= 0.0
    }
    
    /// 獲取剩餘時間百分比
    pub fn get_time_percentage(&self) -> f32 {
        if self.duration > 0.0 {
            (self.remaining_time / self.duration).max(0.0).min(1.0)
        } else {
            0.0
        }
    }
}