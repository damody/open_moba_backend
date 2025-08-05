use specs::storage::VecStorage;
use specs::{Component, Entity};
use serde::{Deserialize, Serialize};

/// 統一的單位組件 - 代表遊戲中所有可攻擊的單位
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Unit {
    pub id: String,
    pub name: String,
    pub unit_type: UnitType,
    
    // 戰鬥屬性 - 這些會覆蓋或補充 CProperty 和 TAttack
    pub max_hp: i32,
    pub current_hp: i32,
    pub base_armor: f32,
    pub magic_resistance: f32,
    pub base_damage: i32,
    pub attack_range: f32,
    pub move_speed: f32,
    pub attack_speed: f32,
    
    // AI 和行為
    pub ai_type: AiType,
    pub aggro_range: f32,
    pub abilities: Vec<String>,
    
    // 狀態追蹤
    pub current_target: Option<Entity>,
    pub last_attack_time: f32,
    pub spawn_position: (f32, f32),
    
    // 獎勵和掉落
    pub exp_reward: i32,
    pub gold_reward: i32,
    pub bounty_type: BountyType,
}

/// 單位類型
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum UnitType {
    Hero,           // 英雄
    Creep,          // 小兵
    Enemy,          // 敵人
    Neutral,        // 中立生物
    Boss,           // Boss
    Elite,          // 精英
    Minion,         // 小怪
    TrainingDummy,  // 訓練假人
}

/// AI 行為類型
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum AiType {
    Aggressive,     // 主動攻擊
    Defensive,      // 被動防守
    Passive,        // 完全被動
    Patrol,         // 巡邏
    Guard,          // 守衛
    None,           // 無AI（如訓練假人）
}

/// 陣營組件 - 決定單位之間的敵友關係
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Faction {
    pub faction_id: FactionType,
    pub team_id: i32,           // 隊伍ID，相同隊伍不會互相攻擊
}

/// 陣營類型
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub enum FactionType {
    Player,         // 玩家陣營
    Enemy,          // 敵對陣營
    Neutral,        // 中立陣營
    Ally,           // 友軍陣營
}

/// 賞金類型
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum BountyType {
    Normal,         // 普通賞金
    Siege,          // 攻城賞金（更高價值）
    Boss,           // Boss賞金
    None,           // 無賞金
}

impl Component for Unit {
    type Storage = VecStorage<Self>;
}

impl Component for Faction {
    type Storage = VecStorage<Self>;
}

impl Unit {
    /// 創建新的單位
    pub fn new(id: String, name: String, unit_type: UnitType) -> Self {
        Unit {
            id,
            name,
            unit_type,
            max_hp: 100,
            current_hp: 100,
            base_armor: 0.0,
            magic_resistance: 0.0,
            base_damage: 10,
            attack_range: 100.0,
            move_speed: 300.0,
            attack_speed: 1.0,
            ai_type: AiType::Aggressive,
            aggro_range: 800.0,
            abilities: Vec::new(),
            current_target: None,
            last_attack_time: 0.0,
            spawn_position: (0.0, 0.0),
            exp_reward: 25,
            gold_reward: 10,
            bounty_type: BountyType::Normal,
        }
    }
    
    /// 從戰役 creep 資料創建單位
    pub fn from_creep_data(creep_data: &crate::ue4::import_campaign::CreepJD) -> Self {
        let unit_type = match creep_data.id.as_str() {
            id if id.contains("dummy") => UnitType::TrainingDummy,
            id if id.contains("boss") => UnitType::Boss,
            _ => UnitType::Creep,
        };
        
        let ai_type = match unit_type {
            UnitType::TrainingDummy => AiType::None,
            UnitType::Boss => AiType::Aggressive,
            _ => AiType::Defensive,
        };
        
        let gold_reward = match creep_data.bounty_type.as_str() {
            "siege" => creep_data.gold_reward * 2,
            _ => creep_data.gold_reward,
        };
        
        let bounty_type = match creep_data.bounty_type.as_str() {
            "siege" => BountyType::Siege,
            "boss" => BountyType::Boss,
            "none" => BountyType::None,
            _ => BountyType::Normal,
        };
        
        Unit {
            id: creep_data.id.clone(),
            name: creep_data.name.clone(),
            unit_type,
            max_hp: creep_data.hp,
            current_hp: creep_data.hp,
            base_armor: creep_data.armor,
            magic_resistance: 0.0,
            base_damage: creep_data.damage,
            attack_range: 150.0, // 默認近戰攻擊距離
            move_speed: creep_data.move_speed,
            attack_speed: 1.0,
            ai_type,
            aggro_range: 600.0,
            abilities: Vec::new(),
            current_target: None,
            last_attack_time: 0.0,
            spawn_position: (0.0, 0.0),
            exp_reward: 0, // creep 資料中沒有經驗值
            gold_reward,
            bounty_type,
        }
    }
    
    /// 從戰役 enemy 資料創建單位
    pub fn from_enemy_data(enemy_data: &crate::ue4::import_campaign::EnemyJD) -> Self {
        let unit_type = match enemy_data.enemy_type.as_str() {
            "boss" => UnitType::Boss,
            "elite" => UnitType::Elite,
            "minion" => UnitType::Minion,
            "neutral" => UnitType::Neutral,
            _ => UnitType::Enemy,
        };
        
        let ai_type = match enemy_data.ai_type.as_str() {
            "aggressive" => AiType::Aggressive,
            "defensive" => AiType::Defensive,
            "patrol" => AiType::Patrol,
            "guard" => AiType::Guard,
            "passive" => AiType::Passive,
            _ => AiType::Aggressive,
        };
        
        Unit {
            id: enemy_data.id.clone(),
            name: enemy_data.name.clone(),
            unit_type: unit_type.clone(),
            max_hp: enemy_data.hp,
            current_hp: enemy_data.hp,
            base_armor: enemy_data.armor,
            magic_resistance: enemy_data.magic_resistance,
            base_damage: enemy_data.damage,
            attack_range: enemy_data.attack_range,
            move_speed: enemy_data.move_speed,
            attack_speed: 1.0,
            ai_type,
            aggro_range: 800.0,
            abilities: enemy_data.abilities.clone(),
            current_target: None,
            last_attack_time: 0.0,
            spawn_position: (0.0, 0.0),
            exp_reward: enemy_data.exp_reward,
            gold_reward: enemy_data.gold_reward,
            bounty_type: match unit_type {
                UnitType::Boss => BountyType::Boss,
                UnitType::Elite => BountyType::Siege,
                _ => BountyType::Normal,
            },
        }
    }
    
    /// 檢查是否死亡
    pub fn is_dead(&self) -> bool {
        self.current_hp <= 0
    }
    
    /// 受到傷害
    pub fn take_damage(&mut self, damage: i32, damage_type: DamageType) -> i32 {
        let actual_damage = match damage_type {
            DamageType::Physical => {
                let damage_reduction = self.base_armor / (self.base_armor + 100.0);
                ((damage as f32) * (1.0 - damage_reduction)) as i32
            },
            DamageType::Magical => {
                let damage_reduction = self.magic_resistance / 100.0;
                ((damage as f32) * (1.0 - damage_reduction.min(0.75))) as i32
            },
            DamageType::Pure => damage,
        };
        
        self.current_hp = (self.current_hp - actual_damage).max(0);
        actual_damage
    }
    
    /// 檢查是否可以攻擊
    pub fn can_attack(&self, current_time: f32) -> bool {
        match self.ai_type {
            AiType::None => false, // 訓練假人等不能攻擊
            _ => current_time - self.last_attack_time >= (1.0 / self.attack_speed),
        }
    }
    
    /// 設置目標
    pub fn set_target(&mut self, target: Option<Entity>) {
        self.current_target = target;
    }
    
    /// 設置出生位置
    pub fn set_spawn_position(&mut self, x: f32, y: f32) {
        self.spawn_position = (x, y);
    }
}

impl Faction {
    /// 創建新的陣營
    pub fn new(faction_id: FactionType, team_id: i32) -> Self {
        Faction { faction_id, team_id }
    }
    
    /// 檢查是否為敵對陣營
    pub fn is_hostile_to(&self, other: &Faction) -> bool {
        if self.team_id == other.team_id {
            return false; // 同隊不敵對
        }
        
        match (&self.faction_id, &other.faction_id) {
            (FactionType::Player, FactionType::Enemy) => true,
            (FactionType::Enemy, FactionType::Player) => true,
            (FactionType::Player, FactionType::Ally) => false,
            (FactionType::Ally, FactionType::Player) => false,
            (FactionType::Neutral, _) => false, // 中立對所有都不敵對
            (_, FactionType::Neutral) => false,
            _ => false,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum DamageType {
    Physical,
    Magical,
    Pure,
}

impl Default for Unit {
    fn default() -> Self {
        Unit::new("unknown".to_string(), "Unknown Unit".to_string(), UnitType::Creep)
    }
}

impl Default for Faction {
    fn default() -> Self {
        Faction::new(FactionType::Neutral, 0)
    }
}