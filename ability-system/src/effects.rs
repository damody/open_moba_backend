use serde::{Deserialize, Serialize};
use specs::Entity;
use std::collections::HashMap;
use crate::types::*;

/// 活躍效果實例
#[derive(Debug, Clone)]
pub struct ActiveEffect {
    pub id: String,
    pub source_ability: String,
    pub caster: Entity,
    pub target: Entity,
    pub effect_type: ActiveEffectType,
    pub duration: f32,
    pub remaining_time: f32,
    pub tick_interval: f32,
    pub last_tick_time: f32,
    pub stacks: i32,
    pub max_stacks: i32,
    pub data: EffectData,
}

/// 活躍效果類型
#[derive(Debug, Clone)]
pub enum ActiveEffectType {
    Buff,
    Debuff,
    Aura,
    Transform,
    Summon,
    AreaEffect {
        center: vek::Vec2<f32>,
        radius: f32,
    },
    Projectile {
        start: vek::Vec2<f32>,
        target: vek::Vec2<f32>,
        speed: f32,
        current_pos: vek::Vec2<f32>,
    },
}

/// 效果數據
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectData {
    /// 屬性修改
    pub attribute_modifiers: HashMap<String, AttributeModifier>,
    
    /// 持續效果
    pub damage_per_tick: f32,
    pub heal_per_tick: f32,
    
    /// 狀態標誌
    pub flags: EffectFlags,
    
    /// 自定義數據
    pub custom_data: HashMap<String, serde_json::Value>,
}

/// 屬性修改器
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttributeModifier {
    pub modifier_type: ModifierType,
    pub value: f32,
    pub is_percentage: bool,
}

/// 修改器類型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ModifierType {
    Add,      // 加法
    Multiply, // 乘法
    Override, // 覆蓋
}

/// 效果標誌
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectFlags {
    pub disable_movement: bool,
    pub disable_attack: bool,
    pub disable_abilities: bool,
    pub invisibility: bool,
    pub invulnerability: bool,
    pub magic_immunity: bool,
    pub silence: bool,
    pub stun: bool,
    pub slow: bool,
    pub root: bool,
}

/// 效果管理器
pub struct EffectManager {
    active_effects: HashMap<String, ActiveEffect>,
    next_effect_id: u64,
}

impl EffectManager {
    pub fn new() -> Self {
        Self {
            active_effects: HashMap::new(),
            next_effect_id: 0,
        }
    }
    
    /// 應用效果
    pub fn apply_effect(&mut self, mut effect: ActiveEffect) -> String {
        let effect_id = format!("effect_{}", self.next_effect_id);
        self.next_effect_id += 1;
        
        effect.id = effect_id.clone();
        
        // 檢查是否與現有效果疊加
        if let Some(existing) = self.find_similar_effect(&effect) {
            self.stack_effect(existing, &effect);
        } else {
            self.active_effects.insert(effect_id.clone(), effect);
        }
        
        effect_id
    }
    
    /// 移除效果
    pub fn remove_effect(&mut self, effect_id: &str) -> Option<ActiveEffect> {
        self.active_effects.remove(effect_id)
    }
    
    /// 更新所有效果
    pub fn update(&mut self, dt: f32, current_time: f32) -> Vec<EffectEvent> {
        let mut events = Vec::new();
        let mut expired_effects = Vec::new();
        
        for (effect_id, effect) in &mut self.active_effects {
            effect.remaining_time -= dt;
            
            // 處理 tick 效果
            if effect.tick_interval > 0.0 && current_time - effect.last_tick_time >= effect.tick_interval {
                events.push(EffectEvent::EffectTick {
                    effect_id: effect_id.clone(),
                    target: effect.target,
                    damage: effect.data.damage_per_tick,
                    heal: effect.data.heal_per_tick,
                });
                effect.last_tick_time = current_time;
            }
            
            // 檢查是否過期
            if effect.remaining_time <= 0.0 {
                expired_effects.push(effect_id.clone());
            }
        }
        
        // 移除過期效果
        for effect_id in expired_effects {
            if let Some(effect) = self.active_effects.remove(&effect_id) {
                events.push(EffectEvent::EffectExpired {
                    effect_id,
                    target: effect.target,
                });
            }
        }
        
        events
    }
    
    /// 獲取目標身上的所有效果
    pub fn get_effects_on_target(&self, target: Entity) -> Vec<&ActiveEffect> {
        self.active_effects.values()
            .filter(|effect| effect.target == target)
            .collect()
    }
    
    /// 獲取目標身上特定類型的效果
    pub fn get_effects_by_type(&self, target: Entity, effect_type: &str) -> Vec<&ActiveEffect> {
        self.active_effects.values()
            .filter(|effect| {
                effect.target == target && 
                effect.source_ability.contains(effect_type)
            })
            .collect()
    }
    
    /// 計算目標的屬性修改
    pub fn calculate_attribute_modifiers(&self, target: Entity, attribute: &str) -> f32 {
        let mut total_modifier = 0.0;
        let mut multiplier = 1.0;
        
        for effect in self.get_effects_on_target(target) {
            if let Some(modifier) = effect.data.attribute_modifiers.get(attribute) {
                match modifier.modifier_type {
                    ModifierType::Add => {
                        if modifier.is_percentage {
                            multiplier *= 1.0 + modifier.value;
                        } else {
                            total_modifier += modifier.value;
                        }
                    }
                    ModifierType::Multiply => {
                        multiplier *= modifier.value;
                    }
                    ModifierType::Override => {
                        // 覆蓋型修改器會取最後一個
                        total_modifier = modifier.value;
                        multiplier = 1.0;
                    }
                }
            }
        }
        
        total_modifier * multiplier
    }
    
    /// 檢查目標是否有特定狀態
    pub fn has_flag(&self, target: Entity, flag: EffectFlag) -> bool {
        self.get_effects_on_target(target).iter().any(|effect| {
            match flag {
                EffectFlag::DisableMovement => effect.data.flags.disable_movement,
                EffectFlag::DisableAttack => effect.data.flags.disable_attack,
                EffectFlag::DisableAbilities => effect.data.flags.disable_abilities,
                EffectFlag::Invisibility => effect.data.flags.invisibility,
                EffectFlag::Invulnerability => effect.data.flags.invulnerability,
                EffectFlag::MagicImmunity => effect.data.flags.magic_immunity,
                EffectFlag::Silence => effect.data.flags.silence,
                EffectFlag::Stun => effect.data.flags.stun,
                EffectFlag::Slow => effect.data.flags.slow,
                EffectFlag::Root => effect.data.flags.root,
            }
        })
    }
    
    /// 尋找相似效果（用於疊加）
    fn find_similar_effect(&self, new_effect: &ActiveEffect) -> Option<&str> {
        for (effect_id, existing) in &self.active_effects {
            if existing.source_ability == new_effect.source_ability &&
               existing.target == new_effect.target &&
               std::mem::discriminant(&existing.effect_type) == std::mem::discriminant(&new_effect.effect_type) {
                return Some(effect_id);
            }
        }
        None
    }
    
    /// 疊加效果
    fn stack_effect(&mut self, existing_id: &str, new_effect: &ActiveEffect) {
        if let Some(existing) = self.active_effects.get_mut(existing_id) {
            // 重置持續時間
            existing.remaining_time = new_effect.duration;
            
            // 增加疊加層數
            if existing.stacks < existing.max_stacks {
                existing.stacks += 1;
            }
        }
    }
}

/// 效果事件
#[derive(Debug, Clone)]
pub enum EffectEvent {
    EffectApplied {
        effect_id: String,
        target: Entity,
    },
    EffectExpired {
        effect_id: String,
        target: Entity,
    },
    EffectTick {
        effect_id: String,
        target: Entity,
        damage: f32,
        heal: f32,
    },
    EffectStacked {
        effect_id: String,
        target: Entity,
        stacks: i32,
    },
}

/// 效果標誌枚舉
#[derive(Debug, Clone, Copy)]
pub enum EffectFlag {
    DisableMovement,
    DisableAttack,
    DisableAbilities,
    Invisibility,
    Invulnerability,
    MagicImmunity,
    Silence,
    Stun,
    Slow,
    Root,
}

impl Default for EffectManager {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for EffectData {
    fn default() -> Self {
        Self {
            attribute_modifiers: HashMap::new(),
            damage_per_tick: 0.0,
            heal_per_tick: 0.0,
            flags: EffectFlags::default(),
            custom_data: HashMap::new(),
        }
    }
}

impl Default for EffectFlags {
    fn default() -> Self {
        Self {
            disable_movement: false,
            disable_attack: false,
            disable_abilities: false,
            invisibility: false,
            invulnerability: false,
            magic_immunity: false,
            silence: false,
            stun: false,
            slow: false,
            root: false,
        }
    }
}

impl ActiveEffect {
    pub fn new(
        source_ability: String,
        caster: Entity,
        target: Entity,
        effect_type: ActiveEffectType,
        duration: f32,
    ) -> Self {
        Self {
            id: String::new(), // 將由 EffectManager 設置
            source_ability,
            caster,
            target,
            effect_type,
            duration,
            remaining_time: duration,
            tick_interval: 0.0,
            last_tick_time: 0.0,
            stacks: 1,
            max_stacks: 1,
            data: EffectData::default(),
        }
    }
    
    /// 是否已過期
    pub fn is_expired(&self) -> bool {
        self.remaining_time <= 0.0
    }
    
    /// 設置 tick 間隔
    pub fn with_tick_interval(mut self, interval: f32) -> Self {
        self.tick_interval = interval;
        self
    }
    
    /// 設置最大疊加層數
    pub fn with_max_stacks(mut self, max_stacks: i32) -> Self {
        self.max_stacks = max_stacks;
        self
    }
}