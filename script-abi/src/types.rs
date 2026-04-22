//! Stable-ABI value types that cross the host/DLL boundary.

use abi_stable::{StableAbi, std_types::{ROption, RString}};

/// Opaque handle to a game entity. Host converts to/from `specs::Entity`.
#[repr(C)]
#[derive(StableAbi, Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct EntityHandle {
    pub id: u32,
    pub gen: u32,
}

impl EntityHandle {
    pub const INVALID: Self = Self { id: u32::MAX, gen: 0 };
    pub fn is_valid(&self) -> bool { self.id != u32::MAX }
}

#[repr(C)]
#[derive(StableAbi, Copy, Clone, Debug, Default)]
pub struct Vec2f {
    pub x: f32,
    pub y: f32,
}

impl Vec2f {
    pub const fn new(x: f32, y: f32) -> Self { Self { x, y } }
}

#[repr(u8)]
#[derive(StableAbi, Copy, Clone, Debug, PartialEq, Eq)]
pub enum DamageKind {
    Physical,
    Magical,
    Pure,
}

/// Passed to `on_damage_taken` as `&mut` — scripts may modify `amount`
/// (e.g. shield, damage reduction, reflect).
#[repr(C)]
#[derive(StableAbi, Clone, Debug)]
pub struct DamageInfo {
    pub attacker: ROption<EntityHandle>,
    pub amount: f32,
    pub kind: DamageKind,
}

#[repr(C)]
#[derive(StableAbi, Clone, Debug)]
pub enum Target {
    Entity(EntityHandle),
    Point(Vec2f),
    None,
}

/// 子彈的飛行路徑規格：
/// - `Homing` 會鎖定 `target` 實體並 per-tick 跟進位置
/// - `Straight` 從發射位置直線飛到 `end_pos`，Tack 放射針用這個
#[repr(C)]
#[derive(StableAbi, Clone, Debug)]
pub enum PathSpec {
    Homing { target: EntityHandle },
    Straight { end_pos: Vec2f },
}

/// TD 塔的完整 metadata（由腳本回報；host 和前端共用）。
/// 新增第 5 種塔只要寫新腳本 + 填這個 struct，host 和前端都不用動。
#[repr(C)]
#[derive(StableAbi, Clone, Debug, Default)]
pub struct TowerMetadata {
    // ===== 戰鬥數值 =====
    /// 基礎攻擊力（物理）
    pub atk: f32,
    /// 攻擊間隔秒數
    pub asd_interval: f32,
    /// 射程（backend 單位）
    pub range: f32,
    /// 子彈飛行速度（backend 單位/秒）
    pub bullet_speed: f32,
    /// 命中後 AoE 半徑（0 = 單體）
    pub splash_radius: f32,
    /// 沿路命中半徑（Tack 針用；0 = 只在 end_pos 觸發）
    pub hit_radius: f32,
    /// 減速乘數（0 = 不減速）
    pub slow_factor: f32,
    /// 減速持續秒數
    pub slow_duration: f32,

    // ===== Host/UI 欄位（原本在 tower_template.rs 的 TowerTemplate）=====
    /// 建造金幣
    pub cost: i32,
    /// 放置碰撞半徑（placement validation + 與其他塔 overlap 判定）
    pub footprint: f32,
    /// 塔 HP（CProperty.hp / mhp）
    pub hp: f32,
    /// 塔轉向速度（度/秒；host tower_tick 用來平滑 rotate facing）
    pub turn_speed_deg: f32,
    /// UI 顯示名稱（按鈕 label、sell 面板、log）
    pub label: RString,
}

/// 發射子彈的完整規格。`spawn_projectile_ex` 接這個。
/// 欄位一次列清所有可能的特性；不用就填 0 / 空字串。
#[repr(C)]
#[derive(StableAbi, Clone, Debug)]
pub struct ProjectileSpec {
    /// 起始位置（世界座標，backend 單位）
    pub from: Vec2f,
    /// 發射者 entity（用於傷害歸屬與 faction filter）
    pub owner: EntityHandle,
    /// 路徑規格
    pub path: PathSpec,
    /// 子彈飛行速度（backend 單位/秒）
    pub speed: f32,
    /// 基礎傷害（物理）
    pub damage: f32,
    /// 沿路 hit-test 半徑（只對 Straight 有意義；0 = 不沿路碰撞，只在 end_pos 觸發）
    pub hit_radius: f32,
    /// 命中後的 AoE 半徑（0 = 單體）
    pub splash_radius: f32,
    /// 命中目標的減速乘數（0 = 不減速，0.5 = 降到 50%）
    pub slow_factor: f32,
    /// 減速持續秒數
    pub slow_duration: f32,
    /// 前端渲染標籤（"dart"/"bomb"/"tack"/"ice"）—— 決定子彈顏色與視覺
    pub kind_tag: RString,
}
