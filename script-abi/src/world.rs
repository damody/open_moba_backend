//! `GameWorld` — the sabi_trait that gives scripts read/write access to the
//! host's ECS. Host implements this on a `WorldAdapter<'a>` that wraps
//! `&'a mut specs::World`.
//!
//! Methods are non-generic (FFI constraint). Adding a component exposure
//! means adding methods here.

use abi_stable::{
    RMut, sabi_trait,
    std_types::{ROption, RStr, RVec},
};
use crate::types::*;
pub use crate::types::{PathSpec, ProjectileSpec};

/// Type alias for the borrowed-mutable dyn form of `GameWorld` — this is
/// what hooks receive. Using this uniformly avoids sprinkling the pointer
/// generic across every hook signature.
pub type GameWorldDyn<'a> = GameWorld_TO<'a, RMut<'a, ()>>;

#[sabi_trait]
pub trait GameWorld: Send {
    // ---- Query ----
    fn get_pos(&self, e: EntityHandle) -> ROption<Vec2f>;
    fn get_hp(&self, e: EntityHandle) -> ROption<f32>;
    fn get_max_hp(&self, e: EntityHandle) -> ROption<f32>;
    fn is_alive(&self, e: EntityHandle) -> bool;
    fn faction_of(&self, e: EntityHandle) -> ROption<RStr<'_>>;
    fn unit_id_of(&self, e: EntityHandle) -> ROption<RStr<'_>>;
    fn query_enemies_in_range(
        &self,
        center: Vec2f,
        radius: f32,
        of: EntityHandle,
    ) -> RVec<EntityHandle>;

    // ---- Mutate ----
    fn set_pos(&mut self, e: EntityHandle, p: Vec2f);
    fn deal_damage(
        &mut self,
        target: EntityHandle,
        amount: f32,
        kind: DamageKind,
        source: ROption<EntityHandle>,
    );
    fn heal(&mut self, target: EntityHandle, amount: f32);
    fn add_buff(&mut self, target: EntityHandle, buff_id: RStr<'_>, duration: f32);
    fn remove_buff(&mut self, target: EntityHandle, buff_id: RStr<'_>);
    fn spawn_projectile(
        &mut self,
        from: Vec2f,
        to: EntityHandle,
        speed: f32,
        dmg: f32,
        owner: EntityHandle,
    ) -> EntityHandle;
    /// TD-mode 通用發射 API：依 `ProjectileSpec` 建立 projectile entity 並
    /// 廣播 projectile/C 給前端。支援 Homing / Straight / AoE / Slow。
    fn spawn_projectile_ex(&mut self, spec: ProjectileSpec) -> EntityHandle;
    /// 套用減速 buff：目標 creep 在 `duration` 秒內移動速度乘 `factor`。
    /// 新 buff 取更強（factor 較小）、更長（duration 取 max）。
    fn add_slow_buff(&mut self, target: EntityHandle, factor: f32, duration: f32);
    /// 發爆炸特效事件給前端（由小到大紅圈）。不造成傷害；傷害由 projectile 本身的 splash。
    fn emit_explosion(&mut self, pos: Vec2f, radius: f32, duration: f32);
    fn despawn(&mut self, e: EntityHandle);

    // ---- Tower / 單位屬性讀寫（供 on_tick 使用）----
    /// 讀塔的攻擊射程（TAttack.range）
    fn get_tower_range(&self, e: EntityHandle) -> f32;
    /// 讀塔的攻擊力（TAttack.atk_physic）
    fn get_tower_atk(&self, e: EntityHandle) -> f32;
    /// 讀攻速間隔秒數（TAttack.asd）
    fn get_asd_interval(&self, e: EntityHandle) -> f32;
    /// 讀目前攻速計數器（TAttack.asd_count）
    fn get_asd_count(&self, e: EntityHandle) -> f32;
    /// 設目前攻速計數器（腳本決定何時消耗）
    fn set_asd_count(&mut self, e: EntityHandle, v: f32);
    /// 設攻擊力（覆寫 TAttack.atk_physic；供 on_spawn 初始化數值用）
    fn set_tower_atk(&mut self, e: EntityHandle, v: f32);
    /// 設射程（覆寫 TAttack.range）
    fn set_tower_range(&mut self, e: EntityHandle, v: f32);
    /// 設攻擊間隔秒數（覆寫 TAttack.asd）
    fn set_asd_interval(&mut self, e: EntityHandle, v: f32);
    /// 設塔/單位 facing 角度（radians，+X = 0，CCW）
    fn set_facing(&mut self, e: EntityHandle, angle_rad: f32);
    /// 查射程內最近的敵人（過濾 faction）；無則 RNone
    fn query_nearest_enemy(
        &self,
        center: Vec2f,
        radius: f32,
        of: EntityHandle,
    ) -> ROption<EntityHandle>;

    // ---- Non-state side effects ----
    fn play_vfx(&mut self, id: RStr<'_>, at: Vec2f);
    fn play_sfx(&mut self, id: RStr<'_>, at: Vec2f);

    // ---- Deterministic RNG (host-seeded) ----
    /// Returns uniform float in [0, 1). Deterministic across replays.
    fn rand_f32(&mut self) -> f32;

    // ---- Log (forwarded to host's log4rs) ----
    fn log_info(&self, msg: RStr<'_>);
    fn log_warn(&self, msg: RStr<'_>);
    fn log_error(&self, msg: RStr<'_>);
}
