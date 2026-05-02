use specs::storage::VecStorage;
use specs::{Component, FlaggedStorage, NullStorage, saveload};
use specs::Entity;
use serde::{Deserialize, Serialize};
use omoba_sim::Fixed64;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum CreepStatus {
    Walk,
    Stop,
    PreWalk,
    /// TD 模式：已走到 path 終點，等 GameProcessor 扣 PlayerLives 後 despawn。
    /// 設定後便不再嘗試移動或重複 push Outcome::CreepLeaked。
    Leaked,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Creep {
    /// Internal identifier (matches CreepEmiter key, e.g. "practice_dummy").
    pub name: String,
    /// Optional display label shown on client (e.g. "練習假人"); falls back to `name`.
    #[serde(default)]
    pub label: Option<String>,
    pub path: String,
    pub pidx: usize,
    pub block_tower: Option<Entity>,
    pub status: CreepStatus,
}

impl Component for Creep {
    type Storage = VecStorage<Self>;
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CProperty {
    pub hp: Fixed64,  // 目前血量
    pub mhp: Fixed64,  // 最大血量
    pub msd: Fixed64, // 移動速度
    pub def_physic: Fixed64, // 物理防禦
    pub def_magic: Fixed64, // 魔法防禦
}

impl Component for CProperty {
    type Storage = VecStorage<Self>;
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CreepEmiter {
    pub root: Creep,
    pub property: CProperty,
    #[serde(default)]
    pub faction_name: String, // "Player" or "Enemy"，空字串視為 "Enemy"
    /// 轉速（度/秒）；未指定用 90
    #[serde(default = "default_turn_speed_deg")]
    pub turn_speed_deg: f32,
    /// 碰撞半徑；未指定用 20
    #[serde(default = "default_creep_collision_radius")]
    pub collision_radius: f32,
}

fn default_turn_speed_deg() -> f32 { 90.0 }
fn default_creep_collision_radius() -> f32 { 20.0 }
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct CurrentCreepWave {
    pub wave: usize,
    pub path: Vec<usize>,
    /// TD 模式下是否正在跑本波；false 代表 idle，等待 StartRound 指令。
    /// 非 TD 模式預設 true，沿用時間觸發邏輯。
    #[serde(default)]
    pub is_running: bool,
    /// TD 模式下本波的開始時刻（按 StartRound 時記錄 totaltime）。
    /// 非 TD 模式忽略此欄位，沿用 `CreepWave.time` 作為開始時間。
    #[serde(default)]
    pub wave_start_time: f32,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CreepWave {
    pub time: f32,
    pub path_creeps: Vec<PathCreeps>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PathCreeps {
    pub creeps: Vec<CreepEmit>,
    pub path_name: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CreepEmit {
    pub time: f32,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TakenDamage {
    pub phys: Fixed64,
    pub magi: Fixed64,
    pub real: Fixed64,
    pub ent: Entity,
    pub source: Entity,  // 攻擊者
}
