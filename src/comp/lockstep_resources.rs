//! 階段 3.4：鎖步輸入管道擁有的 ECS 資源。
//!
//! 位於“comp::”（不是“lockstep::”），因為：
//! 1. 主機 omb 調度程式在每個時鐘週期都會消耗它們，無論
//! kcp 鎖步傳輸是否處於作用中狀態。
//! 2. omfx sim_runner 工作執行緒也寫入它們（之後
//! 將 `TickBatch` 原始輸入轉換為主機 `PlayerInput` 類型），
//! 所以它們必須存在於以 *all* 功能編譯的模組中
//! 配置。 `lockstep::` 位於 `#[cfg(feature = "kcp")]` 後面。
//!
//! `PlayerInput`（prost 產生的原型類型從
//! `lockstep::PlayerInput`) 僅在 `feature = "kcp"` 下可用。到
//! 保持此模組始終編譯，我們將輸入儲存為不透明的
//! `serde_json::Value` 這裡是不可能的（有損+序列化一個空的
//! oneof 很尷尬）。相反，該資源擁有一個功能門控類型
//! 有效負載和消費者係統也具有門功能。非 kcp 建置獲取
//! 沒有任何內容寫入或讀取的空資源。

#[cfg(feature = "kcp")]
use std::collections::HashMap;

#[cfg(feature = "kcp")]
use crate::lockstep::PlayerInput;

use omoba_sim::Vec2 as SimVec2;

/// 從最新解碼的玩家輸入的每個刻度集合
/// `TickBatch`。每個刻度由消費者係統清除。 ‘勾選’紀錄
/// 這些輸入目標的鎖步刻度數 - 僅目前使用
/// 用於診斷日誌記錄/非同步追蹤。
#[cfg(feature = "kcp")]
#[derive(Default)]
pub struct PendingPlayerInputs {
    /// `player_id → PlayerInput` 用於目前的鎖步刻度。每個
    /// `TickBatch` 寫入批次替換了這張地圖（每個玩家一個輸入
    /// 每個價格變動是鎖步合約）。
    pub by_player: HashMap<u32, PlayerInput>,
    /// 鎖步勾選輸入目標。消費者係統使用這個
    /// 僅適用於日誌上下文 - 實際副作用針對任何刻度
    /// 調度程序目前正在運行。
    pub tick: u32,
}

/// 非 kcp 建置：空標記，以便調度程式/系統程式碼可以讀/寫
/// 到處都沒有編譯時功能門的資源。
#[cfg(not(feature = "kcp"))]
#[derive(Default)]
pub struct PendingPlayerInputs;

/// 階段 2.1：延遲的塔生成請求源自
/// `PlayerInputEnum::TowerPlace`。僅鎖定“player_input_tick::Sys”
/// 具有系統資料存取權限（無“&mut World”），但需要“spawn_td_tower”
/// `&mut World` 查詢 `TowerTemplateRegistry` + 建立實體 + 推送
/// `ScriptEvent::Spawn`。所以我們在這裡排隊並排空`tick()`（主機）/
/// 調度程式運行後立即執行“sim_runner”（副本）。
///
/// 不變：必須在主機和副本上的每個蜱蟲上耗盡，因此兩者
/// sims 保持確定性等價。 `comp::遊戲處理器::
/// rain_pending_tower_spawns`是單一共用drain入口點。
#[derive(Default)]
pub struct PendingTowerSpawnQueue {
    pub requests: Vec<PendingTowerSpawn>,
}

#[derive(Clone, Debug)]
pub struct PendingTowerSpawn {
    pub kind_id: u32,
    pub pos: SimVec2,
    pub owner_pid: u32,
}

/// 階段 2.2：延遲的塔樓銷售請求源自
/// `PlayerInputEnum::TowerSell`。與“PendingTowerSpawnQueue”的基本原則相同：
/// 鎖定步驟 `player_input_tick::Sys` 僅有 SystemData 存取權限，但是
/// 出售塔需要“&mut World”（閱讀範本註冊表以獲得退款，
/// 在英雄上寫入“Gold”存儲，刪除實體，清除“BuffStore”）。所以我們
/// 在這裡排隊並在“tick()”（主機）/“sim_runner”（副本）中排出
/// 調度程序運行後。
///
/// 不變：必須透過主機和副本上的每個刻度進行排空
/// `comp::GameProcessor::drain_pending_tower_sells`。
#[derive(Default)]
pub struct PendingTowerSellQueue {
    pub requests: Vec<PendingTowerSell>,
}

#[derive(Clone, Debug)]
pub struct PendingTowerSell {
    pub tower_entity_id: u32,
    pub owner_pid: u32,
}

/// 階段 2.3：延遲的塔升級請求源自
/// `PlayerInputEnum::TowerUpgrade`。與「PendingTowerSellQueue」的基本原則相同：
/// 鎖定步驟 `player_input_tick::Sys` 僅有 SystemData 存取權限，但是
/// 升級塔需要`&mut World`（讀TowerUpgradeRegistry，寫
/// `Gold` / `Tower` / `BuffStore`，透過 tower_upgrade_rules 進行驗證）。所以我們
/// 在這裡排隊並在“tick()”（主機）/“sim_runner”（副本）中排出
/// 調度程序運行後。
///
/// 不變：必須透過主機和副本上的每個刻度進行排空
/// `comp::GameProcessor::drain_pending_tower_upgrades`。
#[derive(Default)]
pub struct PendingTowerUpgradeQueue {
    pub requests: Vec<PendingTowerUpgrade>,
}

#[derive(Clone, Debug)]
pub struct PendingTowerUpgrade {
    pub tower_entity_id: u32,
    pub path: u8,
    /// 目標等級（升級後）。階段 2.3：客戶端可以發送“0”，如果
    /// 尚未透過快照觀察到 `Tower.upgrade_levels` — omb
    /// 處理程序根據實體的當前計算實際目標
    /// 在這種情況下為「upgrade_levels[path] + 1」。
    pub level: u8,
    pub owner_pid: u32,
}

/// 來自 `PlayerInputEnum::UpgradeAbility` 的延遲英雄技能升級請求。
/// 更新 `Hero.ability_levels`、消耗 `skill_points`、讀取 `AbilityRegistry`，
/// 以及推送 `ScriptEvent::SkillLearn` 都需要 `&mut World`，所以 input system
/// 先在這裡排隊，再由 `GameProcessor` 於 dispatch 後 drain。
///
/// 不變式：host 與 replica 每個 tick 都必須透過
/// `comp::GameProcessor::drain_pending_ability_upgrades` drain。
#[derive(Default)]
pub struct PendingAbilityUpgradeQueue {
    pub requests: Vec<PendingAbilityUpgrade>,
}

#[derive(Clone, Debug)]
pub struct PendingAbilityUpgrade {
    pub ability_index: u32,
    pub owner_pid: u32,
}

/// 來自 `PlayerInputEnum::CastAbility` 的延遲英雄施法請求。
/// 解析 caster、驗證已學習/冷卻狀態，以及推送 `ScriptEvent::SkillCast`
/// 都需要 `&mut World`，所以施法沿用和升級相同的 queue + drain pattern。
#[derive(Default)]
pub struct PendingAbilityCastQueue {
    pub requests: Vec<PendingAbilityCast>,
}

#[derive(Clone, Debug)]
pub struct PendingAbilityCast {
    pub ability_index: u32,
    pub target_pos: Option<SimVec2>,
    pub target_entity: Option<u32>,
    pub owner_pid: u32,
}

/// 階段 2.4：延遲的物品使用請求源自
/// `PlayerInputEnum::ItemUse`。與其他待處理隊列的基本原理相同：
/// `use_item` 讀取 `ItemRegistry` 資源 + 寫入 `Inventory` + 寫入
/// `CProperty` + 查詢 `Hero`/`Faction` 存儲，其中沒有一個
/// 可從規格“System”SystemData 存取。
///
/// 不變：必須透過主機和副本上的每個刻度進行排空
/// `comp::GameProcessor::drain_pending_item_uses`。
#[derive(Default)]
pub struct PendingItemUseQueue {
    pub requests: Vec<PendingItemUse>,
}

#[derive(Clone, Debug)]
pub struct PendingItemUse {
    pub item_slot: u32,
    pub target_pos: Option<SimVec2>,
    pub target_entity: Option<u32>,
    pub owner_pid: u32,
}

/// MoveTo：英雄右鍵移動。將 `MoveTarget` 元件寫入
/// 玩家的英雄實體。與其他 Pending 相同的「&mut World」基本原理
/// 隊列——系統不能藉用世界，所以我們排隊並在之後耗盡
/// 調度員。
///
/// 不變：必須透過主機和副本上的每個刻度進行排空
/// `comp::GameProcessor::drain_pending_moves`。
#[derive(Default)]
pub struct PendingMoveQueue {
    pub requests: Vec<PendingMoveTo>,
}

#[derive(Clone, Debug)]
pub struct PendingMoveTo {
    pub pos: SimVec2,
    pub owner_pid: u32,
}

/// 階段 5.3：觀察者重新加入的最新序列化世界快照。
///
/// 更新每個“SNAPSHOT_INTERVAL_TICKS”調度程序刻度（= 30 s @ 120 Hz）。
/// 由 KCP 傳輸的 0x16 SnapshotResp 處理程序用於引導
/// 觀察者客戶端在遊戲中期連接；觀察者將位元組應用到
/// 然後它的 sim_runner 透過後續的 TickBatches 向前播放。
///
/// `bytes` 透過 `omoba_sim::snapshot::serialize` 進行二進位碼序列化
/// 元件的穩定子集（`id` + `pos` + `vel` + `faceing` + `hp`/`mhp`
/// +「種類」）。架構透過「WorldSnapshot::schema_version」固定在內部
/// `lockstep::snapshot_ Producer` — 改變線上格式需要
/// 協調兩端。
///
/// 空位元組（`tick = 0`）表示尚未儲存快照； KCP 處理程序
/// 按原樣返回它，觀察者會從“current_tick”開始播放。
#[derive(Default)]
pub struct SnapshotStore {
    /// 勾選快照的拍攝地點。 `0` = 還沒有快照。
    pub tick: u32,
    /// bincode 序列化的「WorldSnapshot」（實體 + Pos / Vel / Facing /
    /// CProperty子集+master_seed+tick+schema_version）。
    pub bytes: Vec<u8>,
}
