use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct CreepWaveData {
    pub Path: Vec<PathJD>,
    pub Creep: Vec<CreepJD>,
    pub CheckPoint: Vec<CheckPointJD>,
    pub Tower: Vec<TowerJD>,
    pub CreepWave: Vec<CreepWaveJD>,
    /// 初始建物放置（引用 `Tower` 模板，指定位置/陣營/是否為基地）
    #[serde(default)]
    pub Structures: Vec<StructureJD>,
    /// 不可通行多邊形區域（英雄與單位會被擋住；不影響視野/投射物）
    #[serde(default)]
    pub BlockedRegions: Vec<BlockedRegionJD>,
    /// 遊戲模式。未指定視為 "Moba"（MVP_1 沿用行為）。
    /// 可選值："Moba" 或 "TowerDefense"。由 `state/initialization.rs::init_creep_wave`
    /// 轉為 `GameMode` resource 供各 system 查詢。
    #[serde(default)]
    pub GameMode: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct StructureJD {
    /// 對應 `Tower[*].Name`（模板名稱，用於查 Hp/Range/AttackSpeed/Physic）
    pub Tower: String,
    /// "Player" 或 "Enemy"
    pub Faction: String,
    pub X: f32,
    pub Y: f32,
    /// 是否為基地（擊毀敵方基地＝玩家勝）
    #[serde(default)]
    pub IsBase: bool,
    /// 覆寫該實例的碰撞半徑（未填用預設）
    #[serde(default)]
    pub CollisionRadius: Option<f32>,
}
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct PathJD {
    pub Name: String,
    pub Points: Vec<String>,
}
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct CreepJD {
    /// Generated creep template id. Map-local creep stats are rejected during
    /// Lua codegen; runtime resolves stats through `omoba-template-ids`.
    pub Name: String,
    /// Legacy JSON-only field. Generated map data omits this.
    #[serde(default)]
    pub Label: Option<String>,
    #[serde(default)]
    pub HP: f32,
    #[serde(default)]
    pub DefendPhysic: f32,
    #[serde(default)]
    pub DefendMagic: f32,
    #[serde(default)]
    pub MoveSpeed: f32,
    /// 陣營 "Player" / "Enemy"；預設 Enemy。用於 LoL 式雙方出兵
    #[serde(default)]
    pub Faction: Option<String>,
    /// 轉速（度/秒），未填用 90
    #[serde(default)]
    pub TurnSpeed: Option<f32>,
    /// 碰撞半徑（未填用預設 20）
    #[serde(default)]
    pub CollisionRadius: Option<f32>,
}
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct CheckPointJD {
    pub Name: String,
    pub Class: String,
    pub X: f32,
    pub Y: f32,
}
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct TowerJD {
    pub Name: String,
    pub Property: PropertyJD,
    pub Attack: AttackJD,
    /// 轉速（度/秒），未填用 90
    #[serde(default)]
    pub TurnSpeed: Option<f32>,
    /// 碰撞半徑（未填用預設 50）
    #[serde(default)]
    pub CollisionRadius: Option<f32>,
}
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct AttackJD {
    pub Range: f32,
    pub AttackSpeed: f32,
    pub Physic: f32,
    pub Magic: f32,
}
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct PropertyJD {
    pub Hp: i32,
    pub Block: i32,
}
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct CreepWaveJD {
    pub Name: String,
    pub StartTime: f32,
    pub Detail: Vec<DetailJD>,
}
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct DetailJD {
    pub Path: String,
    pub Creeps: Vec<CreepsJD>,
}
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct CreepsJD {
    pub Time: f32,
    pub Creep: String,
}

/// 不可通行多邊形區域（凹/凸皆可）。至少 3 點。
/// 由 generated map data 的 `BlockedRegions` 欄位載入，並於 `state/initialization.rs`
/// 轉為 `comp::BlockedRegions` resource 供移動 tick 查詢。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct BlockedRegionJD {
    pub Name: String,
    pub Points: Vec<PointJD>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default)]
pub struct PointJD {
    pub X: f32,
    pub Y: f32,
}
