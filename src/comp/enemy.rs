use specs::storage::VecStorage;
use specs::{Component, Entity};
use serde::{Deserialize, Serialize};

/// 敵人組件 - 包含敵人的基礎屬性和AI行為
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Enemy {
    pub id: String,
    pub name: String,
    pub enemy_type: EnemyType,
    
    // 戰鬥屬性
    pub max_hp: i32,
    pub current_hp: i32,
    pub armor: f32,
    pub magic_resistance: f32,
    pub base_damage: i32,
    pub attack_range: f32,
    pub move_speed: f32,
    pub attack_speed: f32,
    
    // AI 行為
    pub ai_type: AiType,
    pub aggro_range: f32,      // 仇恨範圍
    pub chase_range: f32,      // 追擊範圍
    pub return_range: f32,     // 回歸範圍
    pub abilities: Vec<String>, // 技能ID列表
    
    // 狀態
    pub current_target: Option<Entity>,
    pub last_attack_time: f32,
    pub is_returning: bool,    // 是否正在回到出生點
    pub spawn_position: (f32, f32), // 出生位置
    
    // 獎勵
    pub exp_reward: i32,
    pub gold_reward: i32,
    pub item_drops: Vec<String>, // 掉落物品ID
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum EnemyType {
    Melee,      // 近戰
    Ranged,     // 遠程
    Caster,     // 法師
    Boss,       // Boss
    Elite,      // 精英
    Minion,     // 小兵
    Neutral,    // 中立生物
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum AiType {
    Aggressive,  // 主動攻擊，積極追擊
    Defensive,   // 被動防守，範圍內才攻擊
    Patrol,      // 巡邏模式，固定路線
    Guard,       // 守衛模式，保護特定目標
    Passive,     // 完全被動，不主動攻擊
    Berserker,   // 狂戰士，血量越低攻擊越強
}

impl Component for Enemy {
    type Storage = VecStorage<Self>;
}

impl Enemy {
    /// 創建新的敵人實例
    pub fn new(id: String, name: String, enemy_type: EnemyType) -> Self {
        Enemy {
            id,
            name,
            enemy_type,
            max_hp: 100,
            current_hp: 100,
            armor: 0.0,
            magic_resistance: 0.0,
            base_damage: 10,
            attack_range: 100.0,
            move_speed: 300.0,
            attack_speed: 1.0,
            ai_type: AiType::Aggressive,
            aggro_range: 800.0,
            chase_range: 1200.0,
            return_range: 1600.0,
            abilities: Vec::new(),
            current_target: None,
            last_attack_time: 0.0,
            is_returning: false,
            spawn_position: (0.0, 0.0),
            exp_reward: 50,
            gold_reward: 25,
            item_drops: Vec::new(),
        }
    }
    
    /// 從戰役資料創建敵人
    pub fn from_campaign_data(enemy_data: &crate::ue4::import_campaign::EnemyJD) -> Self {
        let enemy_type = match enemy_data.enemy_type.as_str() {
            "melee" => EnemyType::Melee,
            "ranged" => EnemyType::Ranged,
            "caster" => EnemyType::Caster,
            "boss" => EnemyType::Boss,
            "elite" => EnemyType::Elite,
            "minion" => EnemyType::Minion,
            "neutral" => EnemyType::Neutral,
            _ => EnemyType::Melee,
        };
        
        let ai_type = match enemy_data.ai_type.as_str() {
            "aggressive" => AiType::Aggressive,
            "defensive" => AiType::Defensive,
            "patrol" => AiType::Patrol,
            "guard" => AiType::Guard,
            "passive" => AiType::Passive,
            "berserker" => AiType::Berserker,
            _ => AiType::Aggressive,
        };
        
        Enemy {
            id: enemy_data.id.clone(),
            name: enemy_data.name.clone(),
            enemy_type,
            max_hp: enemy_data.hp,
            current_hp: enemy_data.hp,
            armor: enemy_data.armor,
            magic_resistance: enemy_data.magic_resistance,
            base_damage: enemy_data.damage,
            attack_range: enemy_data.attack_range,
            move_speed: enemy_data.move_speed,
            attack_speed: 1.0,
            ai_type,
            aggro_range: 800.0,
            chase_range: 1200.0,
            return_range: 1600.0,
            abilities: enemy_data.abilities.clone(),
            current_target: None,
            last_attack_time: 0.0,
            is_returning: false,
            spawn_position: (0.0, 0.0),
            exp_reward: enemy_data.exp_reward,
            gold_reward: enemy_data.gold_reward,
            item_drops: Vec::new(),
        }
    }
    
    /// 受到傷害
    pub fn take_damage(&mut self, damage: i32, damage_type: DamageType) -> i32 {
        let actual_damage = match damage_type {
            DamageType::Physical => {
                let damage_reduction = self.armor / (self.armor + 100.0);
                ((damage as f32) * (1.0 - damage_reduction)) as i32
            },
            DamageType::Magical => {
                let damage_reduction = self.magic_resistance / 100.0;
                ((damage as f32) * (1.0 - damage_reduction.min(0.75))) as i32 // 魔抗上限75%
            },
            DamageType::Pure => damage, // 純粹傷害無視防禦
        };
        
        self.current_hp = (self.current_hp - actual_damage).max(0);
        actual_damage
    }
    
    /// 治療
    pub fn heal(&mut self, amount: i32) -> i32 {
        let old_hp = self.current_hp;
        self.current_hp = (self.current_hp + amount).min(self.max_hp);
        self.current_hp - old_hp // 返回實際治療量
    }
    
    /// 檢查是否死亡
    pub fn is_dead(&self) -> bool {
        self.current_hp <= 0
    }
    
    /// 檢查是否滿血
    pub fn is_full_health(&self) -> bool {
        self.current_hp >= self.max_hp
    }
    
    /// 獲取血量百分比
    pub fn get_health_percentage(&self) -> f32 {
        if self.max_hp > 0 {
            (self.current_hp as f32 / self.max_hp as f32).max(0.0).min(1.0)
        } else {
            0.0
        }
    }
    
    /// 設置目標
    pub fn set_target(&mut self, target: Option<Entity>) {
        self.current_target = target;
        if target.is_some() {
            self.is_returning = false;
        }
    }
    
    /// 清除目標
    pub fn clear_target(&mut self) {
        self.current_target = None;
    }
    
    /// 開始回歸
    pub fn start_returning(&mut self) {
        self.current_target = None;
        self.is_returning = true;
    }
    
    /// 停止回歸
    pub fn stop_returning(&mut self) {
        self.is_returning = false;
    }
    
    /// 設置出生位置
    pub fn set_spawn_position(&mut self, x: f32, y: f32) {
        self.spawn_position = (x, y);
    }
    
    /// 重置到出生狀態
    pub fn respawn(&mut self) {
        self.current_hp = self.max_hp;
        self.current_target = None;
        self.is_returning = false;
        self.last_attack_time = 0.0;
    }
    
    /// 檢查是否可以攻擊
    pub fn can_attack(&self, current_time: f32) -> bool {
        current_time - self.last_attack_time >= (1.0 / self.attack_speed)
    }
    
    /// 記錄攻擊時間
    pub fn record_attack(&mut self, current_time: f32) {
        self.last_attack_time = current_time;
    }
    
    /// 獲取實際攻擊力
    pub fn get_effective_damage(&self) -> i32 {
        match self.ai_type {
            AiType::Berserker => {
                // 狂戰士：血量越低攻擊力越高
                let health_ratio = self.get_health_percentage();
                let damage_multiplier = 1.0 + (1.0 - health_ratio) * 0.5; // 最高+50%攻擊
                ((self.base_damage as f32) * damage_multiplier) as i32
            },
            _ => self.base_damage,
        }
    }
    
    /// 獲取適合的AI行為範圍
    pub fn get_behavior_ranges(&self) -> (f32, f32, f32) {
        match self.enemy_type {
            EnemyType::Boss => (1200.0, 1800.0, 2400.0), // Boss有更大的行為範圍
            EnemyType::Elite => (1000.0, 1500.0, 2000.0),
            EnemyType::Ranged | EnemyType::Caster => (900.0, 1300.0, 1700.0),
            _ => (self.aggro_range, self.chase_range, self.return_range),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
pub enum DamageType {
    Physical, // 物理傷害
    Magical,  // 魔法傷害
    Pure,     // 純粹傷害
}

impl Default for Enemy {
    fn default() -> Self {
        Enemy::new("unknown".to_string(), "Unknown Enemy".to_string(), EnemyType::Melee)
    }
}