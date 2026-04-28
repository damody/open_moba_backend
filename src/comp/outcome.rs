// 引入必要的模組和套件
use crate::{comp, Creep, CProperty, TProperty};
use super::Projectile;
use hashbrown::HashSet;
use serde::{Deserialize, Serialize};
use vek::*;  // 向量數學庫
use specs::Entity;  // ECS 實體系統
use std::collections::VecDeque;
use std::sync::Mutex;
use std::ops::DerefMut;
use std::cmp::Ordering;
use voracious_radix_sort::{Radixable, RadixSort};  // 基數排序演算法
use crate::Tower;
use crate::TAttack;

/// 遊戲結果事件枚舉
/// 用於處理遊戲中各種事件的結果，例如傷害、死亡、治療等
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum Outcome {
    /// 傷害事件
    Damage {
        pos: Vec2<f32>,      // 傷害發生位置
        phys: f32,           // 物理傷害數值
        magi: f32,           // 魔法傷害數值
        real: f32,           // 真實傷害數值（無視防禦）
        source: Entity,      // 傷害來源實體
        target: Entity,      // 傷害目標實體
        /// P7: true 表示此 damage 由非 AOE projectile 命中產生，且彈丸在
        /// 發射時已將最終 damage 透過 ProjectileCreate.damage 傳給 client，
        /// client 已排程於 impact 時刻 local 扣血。server 在 `handle_damage`
        /// 中看到此旗標 + aggregation 全部為 predeclared 時，會跳過 creep.H
        /// 廣播省 bytes；若同 tick 還有非 predeclared 來源（melee/ability），
        /// 聚合後 flag 變 false，照常發 creep.H 保持權威。
        #[serde(default)]
        predeclared: bool,
    },
    /// 投射物軌跡事件
    ProjectileLine2 {
        pos: Vec2<f32>,                // 投射物位置
        source: Option<Entity>,        // 投射物來源實體（可選）
        target: Option<Entity>,        // 投射物目標實體（可選）
    },
    /// 死亡事件
    Death {
        pos: Vec2<f32>,      // 死亡位置
        ent: Entity,         // 死亡的實體
    },
    /// 小兵生成事件
    Creep {
        cd: CreepData,       // 小兵資料
    },
    /// 小兵停止移動事件
    CreepStop {
        source: Entity,      // 發起停止的實體
        target: Entity,      // 目標實體
    },
    /// 小兵移動事件
    CreepWalk {
        target: Entity,      // 移動的目標實體
    },
    /// 塔防建築事件
    Tower {
        pos: Vec2<f32>,      // 塔的位置
        td: TowerData,       // 塔的資料
    },
    /// 治療事件
    Heal {
        pos: Vec2<f32>,      // 治療發生位置
        target: Entity,      // 治療目標實體
        amount: f32,         // 治療量
    },
    /// 更新攻擊狀態事件
    UpdateAttack {
        target: Entity,                  // 目標實體
        asd_count: Option<f32>,          // 攻擊速度計數器（可選）
        cooldown_reset: bool,            // 是否重置冷卻時間
    },
    /// 獲得經驗值事件
    GainExperience {
        target: Entity,      // 獲得經驗的實體
        amount: i32,         // 經驗值數量
    },
    /// 獲得金錢事件（擊殺獎勵、任務獎勵等）
    GainGold {
        target: Entity,      // 獲得金錢的實體（通常為 hero）
        amount: i32,         // 金錢數量
    },
    /// 生成單位事件
    SpawnUnit {
        pos: Vec2<f32>,                        // 生成位置
        unit: crate::comp::Unit,               // 單位類型
        faction: crate::comp::Faction,         // 陣營
        duration: Option<f32>,                 // 持續時間（可選，用於臨時單位）
    },
    /// TD 模式：小兵走到 path 終點（未被擊殺）。
    /// GameProcessor 會扣 PlayerLives 1、delete entity、並廣播 hero.stats（lives 更新）。
    CreepLeaked {
        ent: Entity,
    },
    /// 通用 buff 施加 outcome：GameProcessor 收到後寫入 `BuffStore`。
    /// 例：attack_stun_chance 命中擲骰成功 → AddBuff{"stun", ...}。
    AddBuff {
        target: Entity,
        buff_id: String,
        duration: f32,
        #[serde(default)]
        payload: serde_json::Value,
    },
    /// Bomb 塔 AoE 命中 → 前端渲染「由小到大紅圈」爆炸特效。
    /// GameProcessor 收到後廣播 `game/explosion` 給前端。
    Explosion {
        pos: Vec2<f32>,
        radius: f32,
        duration: f32,
    },
    /// Tack 塔放射針：無 target，從 `pos` 飛向 `end_pos`。
    /// projectile_tick 會每 tick 掃描沿路是否命中敵人（第一個打到就消失）。
    ProjectileDirectional {
        pos: Vec2<f32>,
        source: Option<Entity>,
        end_pos: Vec2<f32>,
    }
}

/// 小兵資料結構
/// 儲存小兵的相關資訊
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CreepData {
    pub pos: Vec2<f32>,      // 小兵位置
    pub creep: Creep,         // 小兵基本資料
    pub cdata: CProperty,     // 小兵屬性資料
    #[serde(default)]
    pub faction_name: String, // "Player" 或 "Enemy"；空視為 "Enemy"
    /// 轉速（度/秒）；預設 90
    #[serde(default = "default_creep_turn_speed_deg")]
    pub turn_speed_deg: f32,
    /// 碰撞半徑；預設 20
    #[serde(default = "default_creep_cr")]
    pub collision_radius: f32,
}

fn default_creep_cr() -> f32 { 20.0 }

fn default_creep_turn_speed_deg() -> f32 { 90.0 }

/// 塔防建築資料結構
/// 儲存塔的相關資訊
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TowerData {
    pub tpty: TProperty,      // 塔的屬性資料
    pub tatk: TAttack,        // 塔的攻擊資料
}

/// 距離索引結構
/// 用於根據距離進行排序，主要用於尋找最近的實體
#[derive(Copy, Clone, Debug, Deserialize, Serialize)]
pub struct DisIndex {
    pub e: Entity,           // 實體參考
    pub dis: f32,            // 距離值（通常是平方距離以避免開根號運算）
}

// 實作 Eq trait，允許完全相等比較
impl Eq for DisIndex {}

// 實作完整排序功能
// 根據距離進行排序
impl Ord for DisIndex {
    fn cmp(&self, other: &Self) -> Ordering{
        self.dis.partial_cmp(&other.dis).unwrap()
    }
}

// 實作部分排序功能
impl PartialOrd for DisIndex {
    fn partial_cmp(&self, other: &DisIndex) -> Option<Ordering> {
        self.dis.partial_cmp(&other.dis)
    }
}

// 實作相等比較
// 只比較距離是否相等
impl PartialEq for DisIndex {
    fn eq(&self, other: &Self) -> bool {
        self.dis == other.dis
    }
}

// 實作基數排序介面
// 使用距離作為排序鍵值
impl Radixable<f32> for DisIndex {
    type Key = f32;
    #[inline]
    fn key(&self) -> Self::Key {
        self.dis
    }
}

/// 搜尋器結構：4 個類別各自包一個 `CollisionIndex`（任意 SpatialIndex impl）。
/// 從 `[collision]` 設定每類獨立選 spatial 演算法（quadtree / hash_grid / bvh / sap）。
pub struct Searcher {
    pub tower: crate::comp::CollisionIndex,
    pub creep: crate::comp::CollisionIndex,
    pub hero: crate::comp::CollisionIndex,
    pub region: crate::comp::CollisionIndex,
}

impl Searcher {
    /// 從 `COLLISION_CONFIG`（game.toml `[collision]` 區段）讀取每類別的演算法名 + params 並構造。
    pub fn from_config() -> Self {
        use crate::config::vision_config::COLLISION_CONFIG;
        use crate::vision::SpatialIndexParams;

        let cfg = &*COLLISION_CONFIG;
        let params = SpatialIndexParams {
            quadtree_max_depth: cfg.QUADTREE_MAX_DEPTH,
            quadtree_max_per_node: cfg.QUADTREE_MAX_PER_NODE,
            hash_grid_cell_size: cfg.SHG_CELL_SIZE,
            bvh_max_leaf: cfg.BVH_MAX_LEAF,
        };
        let s = Self {
            tower: crate::comp::CollisionIndex::new(&cfg.SPATIAL_INDEX_TOWER, params.clone()),
            creep: crate::comp::CollisionIndex::new(&cfg.SPATIAL_INDEX_CREEP, params.clone()),
            hero: crate::comp::CollisionIndex::new(&cfg.SPATIAL_INDEX_HERO, params.clone()),
            region: crate::comp::CollisionIndex::new(&cfg.SPATIAL_INDEX_REGION, params),
        };
        log::info!(
            "Searcher initialized: tower={}, creep={}, hero={}, region={}",
            s.tower.kind(), s.creep.kind(), s.hero.kind(), s.region.kind()
        );
        s
    }

    /// 單位-單位 + 單位-region 碰撞查詢：4 類別並查，回傳合併結果。
    /// `n` 為每個索引各自取幾個最近者（16 在非極端場合即覆蓋所有真實碰撞）。
    pub fn search_collidable(&self, pos: Vec2<f32>, radius: f32, n: usize) -> Vec<DisIndex> {
        let mut out = Vec::with_capacity(n * 4);
        out.extend(self.hero.search_nn(pos, radius, n));
        out.extend(self.creep.search_nn(pos, radius, n));
        out.extend(self.tower.search_nn(pos, radius, n));
        out.extend(self.region.search_nn(pos, radius, n));
        out
    }
}

impl Default for Searcher {
    fn default() -> Self {
        Self::from_config()
    }
}

