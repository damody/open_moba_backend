use specs::storage::VecStorage;
use specs::{Component, Entity};
use serde::{Deserialize, Serialize};
use omoba_sim::Fixed64;

/// 統一的單位組件 - 代表遊戲中所有可攻擊的單位
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Unit {
    pub id: String,
    pub name: String,
    pub unit_type: UnitType,

    // 戰鬥屬性 - 這些會覆蓋或補充 CProperty 和 TAttack
    pub max_hp: i32,
    pub current_hp: i32,
    pub base_armor: Fixed64,
    pub magic_resistance: Fixed64,
    pub base_damage: i32,
    pub attack_range: Fixed64,
    pub move_speed: Fixed64,
    pub attack_speed: Fixed64,

    // AI 和行為
    pub ai_type: AiType,
    pub aggro_range: Fixed64,
    pub abilities: Vec<String>,

    // 狀態追蹤
    pub current_target: Option<Entity>,
    pub last_attack_time: Fixed64,
    // NOTE: spawn_position is f32 by design (initial pos, never mutated); sim-side reads Pos (Fixed64) directly.
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
    Summon,         // 召喚物
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
            base_armor: Fixed64::ZERO,
            magic_resistance: Fixed64::ZERO,
            base_damage: 10,
            attack_range: Fixed64::from_i32(100),
            move_speed: Fixed64::from_i32(300),
            attack_speed: Fixed64::ONE,
            ai_type: AiType::Aggressive,
            aggro_range: Fixed64::from_i32(800),
            abilities: Vec::new(),
            current_target: None,
            last_attack_time: Fixed64::ZERO,
            spawn_position: (0.0, 0.0),
            exp_reward: 25,
            gold_reward: 10,
            bounty_type: BountyType::Normal,
        }
    }
    
    /// 從戰役 creep 資料創建單位
    pub fn from_creep_data(creep_data: &crate::ue4::import_campaign::CreepJD) -> Self {
        // generated story creep 條目已 slim 成只剩 id；數值從 templates.lua 走
        // omoba_template_ids::creep_stats() 取。
        use omoba_template_ids::{creep_by_name, creep_display, creep_stats};
        let cid = creep_by_name(&creep_data.id)
            .unwrap_or_else(|| panic!("creep id '{}' not in generated templates", creep_data.id));
        let s = creep_stats(cid)
            .unwrap_or_else(|| panic!("creep '{}' has no stats in generated templates", creep_data.id));

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

        // template-ids creep_stats already Fixed64; we keep i32 for hp/damage by converting via render boundary.
        // NOTE: Unit.{max_hp, base_damage} are i32 by design (integer game values); convert from Fixed64 at this boundary.
        let attack_range = if s.attack_range.raw() > 0 { s.attack_range } else { Fixed64::from_i32(150) };
        Unit {
            id: creep_data.id.clone(),
            name: creep_display(cid).to_string(),
            unit_type,
            max_hp: s.hp.to_f32_for_render() as i32,
            current_hp: s.hp.to_f32_for_render() as i32,
            base_armor: s.armor,
            magic_resistance: s.magic_resistance,
            base_damage: s.damage.to_f32_for_render() as i32,
            attack_range,
            move_speed: s.move_speed,
            attack_speed: Fixed64::ONE,
            ai_type,
            aggro_range: Fixed64::from_i32(600),
            abilities: Vec::new(),
            current_target: None,
            last_attack_time: Fixed64::ZERO,
            spawn_position: (0.0, 0.0),
            exp_reward: s.exp_reward,
            gold_reward: s.gold_reward,
            bounty_type: BountyType::Normal,
        }
    }

    /// 從戰役 enemy 資料創建單位 — generated story enemy 條目已 slim 成只剩 id +
    /// abilities override，數值從 templates.lua creep_stats() 取。
    pub fn from_enemy_data(enemy_data: &crate::ue4::import_campaign::EnemyJD) -> Self {
        use omoba_template_ids::{creep_by_name, creep_display, creep_stats};
        let cid = creep_by_name(&enemy_data.id)
            .unwrap_or_else(|| panic!("enemy id '{}' not in generated templates", enemy_data.id));
        let s = creep_stats(cid)
            .unwrap_or_else(|| panic!("enemy '{}' has no stats in generated templates", enemy_data.id));

        // u8 (codegen) → enum 變體
        let unit_type = match s.enemy_type {
            3 => UnitType::Boss,    // boss
            _ => UnitType::Enemy,
        };
        let ai_type = match s.ai_type {
            0 => AiType::Defensive,
            1 => AiType::Aggressive,
            2 => AiType::Patrol,
            3 => AiType::Guard,
            4 => AiType::Passive,
            _ => AiType::Aggressive,
        };

        // NOTE: Unit.{current_hp, max_hp, base_damage} are i32 by design (integer game values).
        Unit {
            id: enemy_data.id.clone(),
            name: creep_display(cid).to_string(),
            unit_type: unit_type.clone(),
            max_hp: s.hp.to_f32_for_render() as i32,
            current_hp: s.hp.to_f32_for_render() as i32,
            base_armor: s.armor,
            magic_resistance: s.magic_resistance,
            base_damage: s.damage.to_f32_for_render() as i32,
            attack_range: s.attack_range,
            move_speed: s.move_speed,
            attack_speed: Fixed64::ONE,
            ai_type,
            aggro_range: Fixed64::from_i32(800),
            abilities: enemy_data.abilities.clone(),
            current_target: None,
            last_attack_time: Fixed64::ZERO,
            spawn_position: (0.0, 0.0),
            exp_reward: s.exp_reward,
            gold_reward: s.gold_reward,
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
                // damage_reduction = armor / (armor + 100)
                let armor = self.base_armor;
                let denom = armor + Fixed64::from_i32(100);
                let reduction = armor / denom;
                let mult = Fixed64::ONE - reduction;
                (Fixed64::from_i32(damage) * mult).to_f32_for_render() as i32
            },
            DamageType::Magical => {
                // damage_reduction = magic_resistance / 100, clamped to 0.75
                let res_pct = self.magic_resistance / Fixed64::from_i32(100);
                let cap = Fixed64::from_raw(768); // 0.75
                let reduction = if res_pct > cap { cap } else { res_pct };
                let mult = Fixed64::ONE - reduction;
                (Fixed64::from_i32(damage) * mult).to_f32_for_render() as i32
            },
            DamageType::Pure => damage,
        };

        self.current_hp = (self.current_hp - actual_damage).max(0);
        actual_damage
    }

    /// 檢查是否可以攻擊
    pub fn can_attack(&self, current_time: Fixed64) -> bool {
        match self.ai_type {
            AiType::None => false, // 訓練假人等不能攻擊
            _ => {
                // (current - last) >= 1 / attack_speed  ⇔  (current - last) * attack_speed >= 1
                if self.attack_speed.raw() <= 0 {
                    return false;
                }
                let elapsed = current_time - self.last_attack_time;
                elapsed * self.attack_speed >= Fixed64::ONE
            }
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
    
    /// 創建雜賀鐵炮兵召喚單位
    pub fn create_saika_gunner(position: (f32, f32), owner_team: i32) -> Self {
        let mut unit = Unit {
            id: "saika_gunner".to_string(),
            name: "雜賀鐵炮兵".to_string(),
            unit_type: UnitType::Summon,
            max_hp: 150,        // 中等血量
            current_hp: 150,
            base_armor: Fixed64::from_i32(2),    // 輕甲
            magic_resistance: Fixed64::ZERO,
            base_damage: 35,    // 較高攻擊力
            attack_range: Fixed64::from_i32(450), // 遠程攻擊
            move_speed: Fixed64::from_i32(280),  // 較慢移速
            attack_speed: Fixed64::from_raw(819),  // 0.8
            ai_type: AiType::Aggressive, // 主動攻擊
            aggro_range: Fixed64::from_i32(500), // 攻擊索敵範圍
            abilities: Vec::new(),
            current_target: None,
            last_attack_time: Fixed64::ZERO,
            spawn_position: position,
            exp_reward: 0,      // 召喚物不給經驗
            gold_reward: 0,     // 召喚物不給金錢
            bounty_type: BountyType::None,
        };

        unit.set_spawn_position(position.0, position.1);
        unit
    }

    /// 創建弓箭手召喚單位
    pub fn create_archer(position: (f32, f32), owner_team: i32) -> Self {
        let mut unit = Unit {
            id: "archer".to_string(),
            name: "弓箭手".to_string(),
            unit_type: UnitType::Summon,
            max_hp: 120,        // 較低血量
            current_hp: 120,
            base_armor: Fixed64::ONE,    // 輕甲
            magic_resistance: Fixed64::ZERO,
            base_damage: 25,    // 中等攻擊力
            attack_range: Fixed64::from_i32(550), // 遠程攻擊
            move_speed: Fixed64::from_i32(320),  // 較快移速
            attack_speed: Fixed64::from_raw(1229), // 1.2
            ai_type: AiType::Aggressive,
            aggro_range: Fixed64::from_i32(600),
            abilities: Vec::new(),
            current_target: None,
            last_attack_time: Fixed64::ZERO,
            spawn_position: position,
            exp_reward: 0,
            gold_reward: 0,
            bounty_type: BountyType::None,
        };

        unit.set_spawn_position(position.0, position.1);
        unit
    }

    /// 創建劍士召喚單位
    pub fn create_swordsman(position: (f32, f32), owner_team: i32) -> Self {
        let mut unit = Unit {
            id: "swordsman".to_string(),
            name: "劍士".to_string(),
            unit_type: UnitType::Summon,
            max_hp: 200,        // 高血量
            current_hp: 200,
            base_armor: Fixed64::from_i32(3),    // 重甲
            magic_resistance: Fixed64::from_i32(10),
            base_damage: 40,    // 高攻擊力
            attack_range: Fixed64::from_i32(120), // 近戰攻擊
            move_speed: Fixed64::from_i32(300),  // 中等移速
            attack_speed: Fixed64::from_raw(922),  // 0.9
            ai_type: AiType::Aggressive,
            aggro_range: Fixed64::from_i32(400),
            abilities: Vec::new(),
            current_target: None,
            last_attack_time: Fixed64::ZERO,
            spawn_position: position,
            exp_reward: 0,
            gold_reward: 0,
            bounty_type: BountyType::None,
        };

        unit.set_spawn_position(position.0, position.1);
        unit
    }

    /// 創建法師召喚單位
    pub fn create_mage(position: (f32, f32), owner_team: i32) -> Self {
        let mut unit = Unit {
            id: "mage".to_string(),
            name: "法師".to_string(),
            unit_type: UnitType::Summon,
            max_hp: 80,         // 低血量
            current_hp: 80,
            base_armor: Fixed64::ZERO,    // 無護甲
            magic_resistance: Fixed64::from_i32(25), // 高魔抗
            base_damage: 45,    // 高魔法攻擊力
            attack_range: Fixed64::from_i32(600), // 超遠程攻擊
            move_speed: Fixed64::from_i32(280),  // 慢移速
            attack_speed: Fixed64::from_raw(717),  // 0.7
            ai_type: AiType::Defensive, // 防守型
            aggro_range: Fixed64::from_i32(650),
            abilities: vec!["magic_missile".to_string()], // 有技能
            current_target: None,
            last_attack_time: Fixed64::ZERO,
            spawn_position: position,
            exp_reward: 0,
            gold_reward: 0,
            bounty_type: BountyType::None,
        };

        unit.set_spawn_position(position.0, position.1);
        unit
    }
    
    /// 創建通用召喚單位
    pub fn create_summon_unit(unit_id: &str, position: (f32, f32), owner_team: i32) -> Option<Self> {
        match unit_id {
            "saika_gunner" => Some(Self::create_saika_gunner(position, owner_team)),
            "archer" => Some(Self::create_archer(position, owner_team)),
            "swordsman" => Some(Self::create_swordsman(position, owner_team)),
            "mage" => Some(Self::create_mage(position, owner_team)),
            _ => {
                log::warn!("未知的召喚單位類型: {}", unit_id);
                None
            }
        }
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

/// 召喚物組件 - 追蹤召喚單位的生命週期
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SummonedUnit {
    pub summoner: Entity,        // 召喚者
    pub duration: Option<f32>,   // 持續時間（秒），None 表示永久
    pub time_remaining: Option<f32>, // 剩餘時間
    pub summon_time: f32,        // 召喚時間戳
}

impl Component for SummonedUnit {
    type Storage = VecStorage<Self>;
}

impl SummonedUnit {
    /// 創建新的召喚物組件
    pub fn new(summoner: Entity, duration: Option<f32>, current_time: f32) -> Self {
        SummonedUnit {
            summoner,
            duration,
            time_remaining: duration,
            summon_time: current_time,
        }
    }
    
    /// 更新時間並檢查是否過期
    pub fn update(&mut self, dt: f32) -> bool {
        if let Some(ref mut remaining) = self.time_remaining {
            *remaining -= dt;
            *remaining <= 0.0
        } else {
            false // 永久召喚物不會過期
        }
    }
    
    /// 檢查是否過期
    pub fn is_expired(&self) -> bool {
        if let Some(remaining) = self.time_remaining {
            remaining <= 0.0
        } else {
            false
        }
    }
}
