use serde::{Deserialize, Serialize};
use specs::Entity;
use std::collections::HashMap;
use crate::types::*;

/// 技能實例 - 代表一個已裝備/學習的技能
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbilityInstance {
    pub id: String,
    pub ability_id: String,
    pub owner: Entity,
    pub current_level: i32,
    pub max_level: i32,
    
    /// 運行時狀態
    pub state: AbilityState,
    pub cooldown_remaining: f32,
    pub charges: i32,
    pub max_charges: i32,
    pub last_used_time: f32,
    
    /// 特殊狀態
    pub is_toggled: bool,
    pub toggle_state: bool,
    pub is_channeling: bool,
    pub channel_time_remaining: f32,
    
    /// 運行時數據
    pub runtime_data: HashMap<String, serde_json::Value>,
}

/// 技能狀態
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AbilityState {
    Ready,
    Cooldown,
    Channeling,
    Disabled,
    NoCharges,
    InsufficientMana,
}

/// 技能輸入請求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbilityRequest {
    pub caster: Entity,
    pub ability_id: String,
    pub target: Option<Entity>,
    pub target_position: Option<vek::Vec2<f32>>,
    pub additional_params: HashMap<String, serde_json::Value>,
}

/// 技能事件
#[derive(Debug, Clone)]
pub enum AbilityEvent {
    AbilityUsed {
        caster: Entity,
        ability_id: String,
        result: AbilityResult,
    },
    AbilityCooldownStarted {
        caster: Entity,
        ability_id: String,
        duration: f32,
    },
    AbilityCooldownEnded {
        caster: Entity,
        ability_id: String,
    },
    AbilityLevelUp {
        caster: Entity,
        ability_id: String,
        new_level: i32,
    },
}

impl AbilityInstance {
    pub fn new(ability_id: String, owner: Entity) -> Self {
        Self {
            id: format!("{}_{}", ability_id, owner.id()),
            ability_id,
            owner,
            current_level: 0,
            max_level: 4,
            state: AbilityState::Disabled,
            cooldown_remaining: 0.0,
            charges: 1,
            max_charges: 1,
            last_used_time: 0.0,
            is_toggled: false,
            toggle_state: false,
            is_channeling: false,
            channel_time_remaining: 0.0,
            runtime_data: HashMap::new(),
        }
    }
    
    /// 檢查技能是否可用
    pub fn is_ready(&self) -> bool {
        matches!(self.state, AbilityState::Ready) 
            && self.current_level > 0 
            && self.cooldown_remaining <= 0.0
            && (self.charges > 0 || self.is_toggled)
    }
    
    /// 檢查技能是否在冷卻中
    pub fn is_on_cooldown(&self) -> bool {
        self.cooldown_remaining > 0.0
    }
    
    /// 檢查是否為切換技能且已啟動
    pub fn is_toggle_active(&self) -> bool {
        self.is_toggled && self.toggle_state
    }
    
    /// 使用技能
    pub fn use_ability(&mut self, cooldown: f32, current_time: f32) -> bool {
        if !self.is_ready() {
            return false;
        }
        
        if self.is_toggled {
            self.toggle_state = !self.toggle_state;
            if !self.toggle_state {
                // 關閉切換技能不消耗冷卻
                return true;
            }
        } else {
            self.charges = (self.charges - 1).max(0);
        }
        
        self.cooldown_remaining = cooldown;
        self.last_used_time = current_time;
        self.update_state();
        
        true
    }
    
    /// 更新技能狀態
    pub fn update(&mut self, dt: f32, current_time: f32) {
        // 更新冷卻時間
        if self.cooldown_remaining > 0.0 {
            self.cooldown_remaining = (self.cooldown_remaining - dt).max(0.0);
        }
        
        // 更新引導時間
        if self.is_channeling && self.channel_time_remaining > 0.0 {
            self.channel_time_remaining = (self.channel_time_remaining - dt).max(0.0);
            if self.channel_time_remaining <= 0.0 {
                self.is_channeling = false;
            }
        }
        
        // 更新技能層數（如果有恢復機制）
        // 這裡可以加入層數恢復邏輯
        
        self.update_state();
    }
    
    /// 更新技能狀態
    fn update_state(&mut self) {
        if self.current_level <= 0 {
            self.state = AbilityState::Disabled;
        } else if self.is_channeling {
            self.state = AbilityState::Channeling;
        } else if self.cooldown_remaining > 0.0 {
            self.state = AbilityState::Cooldown;
        } else if self.charges <= 0 && !self.is_toggled {
            self.state = AbilityState::NoCharges;
        } else {
            self.state = AbilityState::Ready;
        }
    }
    
    /// 升級技能
    pub fn level_up(&mut self) -> bool {
        if self.current_level < self.max_level {
            self.current_level += 1;
            self.update_state();
            true
        } else {
            false
        }
    }
    
    /// 設置運行時數據
    pub fn set_runtime_data<T>(&mut self, key: &str, value: T) 
    where
        T: Serialize,
    {
        if let Ok(json_value) = serde_json::to_value(value) {
            self.runtime_data.insert(key.to_string(), json_value);
        }
    }
    
    /// 獲取運行時數據
    pub fn get_runtime_data<T>(&self, key: &str) -> Option<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        self.runtime_data.get(key)
            .and_then(|value| serde_json::from_value(value.clone()).ok())
    }
    
    /// 開始引導
    pub fn start_channeling(&mut self, duration: f32) {
        self.is_channeling = true;
        self.channel_time_remaining = duration;
        self.update_state();
    }
    
    /// 中斷引導
    pub fn interrupt_channeling(&mut self) {
        self.is_channeling = false;
        self.channel_time_remaining = 0.0;
        self.update_state();
    }
}

impl AbilityRequest {
    pub fn new(caster: Entity, ability_id: String) -> Self {
        Self {
            caster,
            ability_id,
            target: None,
            target_position: None,
            additional_params: HashMap::new(),
        }
    }
    
    pub fn with_target(mut self, target: Entity) -> Self {
        self.target = Some(target);
        self
    }
    
    pub fn with_position(mut self, position: vek::Vec2<f32>) -> Self {
        self.target_position = Some(position);
        self
    }
    
    pub fn with_param<T>(mut self, key: &str, value: T) -> Self 
    where
        T: Serialize,
    {
        if let Ok(json_value) = serde_json::to_value(value) {
            self.additional_params.insert(key.to_string(), json_value);
        }
        self
    }
}