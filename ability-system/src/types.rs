use serde::{Deserialize, Serialize};
use specs::Entity;
use std::collections::HashMap;
use vek::Vec2;

/// 技能執行結果
#[derive(Debug, Clone)]
pub enum AbilityResult {
    Success(Vec<AbilityEffect>),
    Failed(String),
    Cooldown(f32),
    InsufficientResources,
}

/// 技能效果類型
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AbilityEffect {
    /// 立即傷害
    InstantDamage {
        target: String,      // 目標選擇器
        damage: f32,
        damage_type: DamageType,
    },
    /// 持續效果
    Buff {
        target: String,
        duration: f32,
        effects: HashMap<String, f32>,
    },
    /// 召喚
    Summon {
        position: Vec2<f32>,
        unit_type: String,
        count: i32,
        duration: Option<f32>,
    },
    /// 範圍效果
    AreaEffect {
        center: Vec2<f32>,
        radius: f32,
        duration: f32,
        tick_interval: f32,
        effects: Vec<AbilityEffect>,
    },
    /// 投射物
    Projectile {
        start: Vec2<f32>,
        target: Vec2<f32>,
        speed: f32,
        on_hit: Vec<AbilityEffect>,
    },
    /// 變身/切換
    Transform {
        target: String,
        transform_id: String,
        duration: Option<f32>,
    },
}

/// 傷害類型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DamageType {
    Physical,
    Magical,
    Pure,
    True,
}

/// 技能執行上下文
#[derive(Debug)]
pub struct AbilityContext {
    pub caster: Entity,
    pub target: Option<Entity>,
    pub target_position: Option<Vec2<f32>>,
    pub level: i32,
    pub time: f32,
    pub additional_data: HashMap<String, serde_json::Value>,
    
    // ECS 相關引用 (通過trait object提供)
    pub world_access: Box<dyn WorldAccess>,
}

/// 世界存取介面
pub trait WorldAccess {
    fn get_position(&self, entity: Entity) -> Option<Vec2<f32>>;
    fn set_position(&mut self, entity: Entity, pos: Vec2<f32>);
    fn get_entities_in_range(&self, center: Vec2<f32>, radius: f32) -> Vec<Entity>;
    fn create_entity(&mut self) -> Entity;
    fn apply_damage(&mut self, target: Entity, damage: f32, damage_type: DamageType);
    fn apply_buff(&mut self, target: Entity, effects: &HashMap<String, f32>, duration: f32);
}

/// 目標選擇器
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TargetSelector {
    Self_,
    Target,
    AllEnemies,
    AllAllies,
    Nearest { enemy: bool, range: f32 },
    InRadius { center: Vec2<f32>, radius: f32, enemy: bool },
    Custom(String), // 自定義選擇邏輯
}

/// 條件檢查
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Condition {
    pub condition_type: ConditionType,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConditionType {
    HasBuff(String),
    HealthBelow(f32),
    HealthAbove(f32),
    InRange(f32),
    HasMana(f32),
    Custom(String),
}

impl AbilityContext {
    pub fn new(caster: Entity, world_access: Box<dyn WorldAccess>) -> Self {
        Self {
            caster,
            target: None,
            target_position: None,
            level: 1,
            time: 0.0,
            additional_data: HashMap::new(),
            world_access,
        }
    }
    
    pub fn with_target(mut self, target: Entity) -> Self {
        self.target = Some(target);
        self
    }
    
    pub fn with_position(mut self, position: Vec2<f32>) -> Self {
        self.target_position = Some(position);
        self
    }
    
    pub fn with_level(mut self, level: i32) -> Self {
        self.level = level;
        self
    }
}