use specs::storage::VecStorage;
use specs::{Component, Entity};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 技能實例 - 代表一個英雄已學習的技能
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Skill {
    pub id: String,
    pub ability_id: String,        // 關聯的 ability 定義
    pub owner: Entity,             // 技能擁有者
    pub current_level: i32,        // 當前等級
    pub max_level: i32,            // 最大等級
    pub cooldown_remaining: f32,   // 剩餘冷卻時間
    pub charges: i32,              // 技能層數（如雜賀眾）
    pub max_charges: i32,          // 最大層數
    pub charge_restore_time: f32,  // 層數恢復間隔
    pub last_charge_time: f32,     // 上次恢復層數的時間
    pub is_toggled: bool,          // 是否為切換技能（如狙擊模式）
    pub toggle_state: bool,        // 切換狀態
}

/// 技能狀態 - 技能的當前狀態
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum SkillState {
    Ready,                // 可以使用
    Cooldown(f32),        // 冷卻中（剩餘時間）
    Channeling(f32),      // 引導中（剩餘時間）
    Casting(f32),         // 施法中（剩餘時間）
    NoMana,               // 法力不足
    NoCharges,            // 沒有層數
    Disabled,             // 被禁用
}

/// 技能效果實例 - 正在進行中的技能效果
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SkillEffect {
    pub id: String,
    pub skill_id: String,          // 來源技能
    pub caster: Entity,            // 施法者
    pub target: Option<Entity>,    // 目標（可選）
    pub target_pos: Option<vek::Vec2<f32>>, // 目標位置
    pub effect_type: SkillEffectType,
    pub duration: f32,             // 效果持續時間
    pub remaining_time: f32,       // 剩餘時間
    pub position: Option<vek::Vec2<f32>>, // 位置（地面技能）
    pub area_center: Option<vek::Vec2<f32>>, // 範圍中心
    pub radius: f32,               // 影響範圍
    pub tick_interval: f32,        // tick 間隔
    pub last_tick_time: f32,       // 上次 tick 時間
    pub stacks: i32,               // 疊加層數
    pub max_stacks: i32,           // 最大疊加
    pub data: SkillEffectData,     // 效果數據
}

/// 技能效果類型
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum SkillEffectType {
    Buff,          // 增益效果
    Debuff,        // 減益效果
    Damage,        // 持續傷害
    Heal,          // 持續治療
    Aura,          // 光環效果
    Transform,     // 變身效果（如狙擊模式）
    Summon,        // 召喚物
    Area,          // 地面效果
    DamageOverTime, // 持續傷害效果
    HealOverTime,   // 持續治療效果
    AreaEffect,     // 範圍效果
}

/// 技能效果數據
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SkillEffectData {
    // 屬性修改
    pub damage_bonus: f32,
    pub attack_speed_bonus: f32,
    pub move_speed_bonus: f32,
    pub range_bonus: f32,
    pub accuracy_bonus: f32,
    
    // 持續效果
    pub damage_per_second: f32,
    pub heal_per_second: f32,
    pub damage_per_tick: f32,
    pub heal_per_tick: f32,
    
    // 範圍效果
    pub area_radius: f32,
    pub affects_allies: bool,
    pub affects_enemies: bool,
    
    // 特殊效果
    pub disable_movement: bool,
    pub disable_attack: bool,
    pub invisibility: bool,
    pub magic_immunity: bool,
    
    // 自定義數據
    pub custom_data: HashMap<String, f32>,
}

/// 技能輸入 - 表示玩家的技能使用請求
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SkillInput {
    pub caster: Entity,
    pub skill_id: String,
    pub target_type: SkillTargetType,
    pub target_entity: Option<Entity>,
    pub target_position: Option<vek::Vec2<f32>>,
    pub additional_data: HashMap<String, f32>,
}

/// 技能目標類型
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum SkillTargetType {
    None,              // 無目標（自身）
    Self_,             // 自身
    Unit,              // 單位目標
    Point,             // 地面點目標
    Direction,         // 方向目標
    Area,              // 範圍目標
}

impl Component for Skill {
    type Storage = VecStorage<Self>;
}

impl Component for SkillEffect {
    type Storage = VecStorage<Self>;
}

impl Component for SkillInput {
    type Storage = VecStorage<Self>;
}

impl Skill {
    /// 創建新技能實例
    pub fn new(ability_id: String, owner: Entity) -> Self {
        Skill {
            id: format!("{}_{}", ability_id, owner.id()),
            ability_id,
            owner,
            current_level: 0,
            max_level: 4,
            cooldown_remaining: 0.0,
            charges: 1,
            max_charges: 1,
            charge_restore_time: 0.0,
            last_charge_time: 0.0,
            is_toggled: false,
            toggle_state: false,
        }
    }
    
    /// 檢查技能是否可用
    pub fn is_ready(&self) -> bool {
        self.current_level > 0 
            && self.cooldown_remaining <= 0.0 
            && (self.charges > 0 || self.is_toggled)
    }
    
    /// 獲取技能狀態
    pub fn get_state(&self) -> SkillState {
        if self.current_level <= 0 {
            return SkillState::Disabled;
        }
        
        if self.cooldown_remaining > 0.0 {
            return SkillState::Cooldown(self.cooldown_remaining);
        }
        
        if self.charges <= 0 && !self.is_toggled {
            return SkillState::NoCharges;
        }
        
        SkillState::Ready
    }
    
    /// 使用技能
    pub fn use_skill(&mut self, cooldown: f32) -> bool {
        if !self.is_ready() {
            return false;
        }
        
        if self.is_toggled {
            self.toggle_state = !self.toggle_state;
        } else {
            self.charges -= 1;
            self.cooldown_remaining = cooldown;
        }
        
        true
    }
    
    /// 更新技能狀態
    pub fn update(&mut self, dt: f32, current_time: f32) {
        // 更新冷卻時間
        if self.cooldown_remaining > 0.0 {
            self.cooldown_remaining -= dt;
            if self.cooldown_remaining < 0.0 {
                self.cooldown_remaining = 0.0;
            }
        }
        
        // 更新技能層數
        if self.charges < self.max_charges && self.charge_restore_time > 0.0 {
            if current_time - self.last_charge_time >= self.charge_restore_time {
                self.charges += 1;
                self.last_charge_time = current_time;
            }
        }
    }
    
    /// 升級技能
    pub fn level_up(&mut self) -> bool {
        if self.current_level < self.max_level {
            self.current_level += 1;
            
            // 根據技能類型設置特殊屬性
            match self.ability_id.as_str() {
                "saika_reinforcements" => {
                    // 雜賀眾：每級增加最大層數
                    self.max_charges = 1 + self.current_level;
                    self.charges = self.max_charges;
                    self.charge_restore_time = 30.0; // 30秒恢復一層
                }
                "sniper_mode" => {
                    // 狙擊模式：切換技能
                    self.is_toggled = true;
                    self.max_charges = 1;
                }
                _ => {}
            }
            
            true
        } else {
            false
        }
    }
}

impl SkillEffect {
    /// 創建新的技能效果
    pub fn new(
        skill_id: String,
        caster: Entity,
        effect_type: SkillEffectType,
        duration: f32,
    ) -> Self {
        SkillEffect {
            id: format!("effect_{}_{}", skill_id, caster.id()),
            skill_id,
            caster,
            target: None,
            target_pos: None,
            effect_type,
            duration,
            remaining_time: duration,
            position: None,
            area_center: None,
            radius: 0.0,
            tick_interval: 1.0,
            last_tick_time: 0.0,
            stacks: 1,
            max_stacks: 1,
            data: SkillEffectData::default(),
        }
    }
    
    /// 檢查效果是否過期
    pub fn is_expired(&self) -> bool {
        self.remaining_time <= 0.0
    }
    
    /// 更新效果
    pub fn update(&mut self, dt: f32, current_time: f32) {
        self.remaining_time -= dt;
    }
    
    /// 檢查是否需要 tick
    pub fn should_tick(&self, current_time: f32) -> bool {
        current_time - self.last_tick_time >= self.tick_interval
    }
    
    /// 執行 tick
    pub fn tick(&mut self, current_time: f32) {
        self.last_tick_time = current_time;
    }
}

impl Default for SkillEffectData {
    fn default() -> Self {
        SkillEffectData {
            damage_bonus: 0.0,
            attack_speed_bonus: 0.0,
            move_speed_bonus: 0.0,
            range_bonus: 0.0,
            accuracy_bonus: 0.0,
            damage_per_second: 0.0,
            heal_per_second: 0.0,
            damage_per_tick: 0.0,
            heal_per_tick: 0.0,
            area_radius: 0.0,
            affects_allies: false,
            affects_enemies: false,
            disable_movement: false,
            disable_attack: false,
            invisibility: false,
            magic_immunity: false,
            custom_data: HashMap::new(),
        }
    }
}