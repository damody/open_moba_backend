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
}
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct PathJD {
    pub Name: String,
    pub Points: Vec<String>,
}
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct CreepJD {
    pub Name: String,
    /// Optional display label (e.g. "練習假人"); falls back to `Name` if absent.
    #[serde(default)]
    pub Label: Option<String>,
    pub HP: f32,
    pub DefendPhysic: f32,
    pub DefendMagic: f32,
    pub MoveSpeed: f32,
    /// 陣營 "Player" / "Enemy"；預設 Enemy。用於 LoL 式雙方出兵
    #[serde(default)]
    pub Faction: Option<String>,
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

