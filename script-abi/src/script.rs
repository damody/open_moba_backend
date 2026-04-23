//! `UnitScript` — the sabi_trait each unit type (hero/tower/creep) implements.
//! All hooks have default no-op impls; scripts override only what they need.
//!
//! Hooks 命名對應 Dota 2 MODIFIER_EVENT_*：`on_attack_start` / `on_attack_landed`
//! / `on_attacked` / `on_health_gained` / `on_mana_gained` / `on_spent_mana`
//! / `on_heal_received` / `on_state_changed` / `on_modifier_added` /
//! `on_modifier_removed`。所有 hook 皆為 no-op default，腳本只覆寫需要的。

use abi_stable::{
    sabi_trait,
    std_types::{RNone, ROption, RStr},
};
use crate::types::{DamageInfo, EntityHandle, Target, TowerMetadata};
use crate::world::GameWorldDyn;

#[sabi_trait]
pub trait UnitScript: Send + Sync {
    /// Unit identifier used by host to dispatch (must match `script` field
    /// in the unit's config entry).
    fn unit_id(&self) -> RStr<'_>;

    /// Called once when the entity is spawned.
    #[sabi(last_prefix_field)]
    fn on_spawn(&self, _e: EntityHandle, _w: &mut GameWorldDyn<'_>) {}

    /// Called every tick for entities with `ScriptUnitTag`. Scripts use this
    /// to drive active behaviour (e.g. towers: find target → spawn projectile).
    /// `dt` is the tick delta in seconds.
    fn on_tick(&self, _e: EntityHandle, _dt: f32, _w: &mut GameWorldDyn<'_>) {}

    /// 塔的靜態 metadata（atk/asd/range/bullet_speed/...）。
    /// host 在 startup 時 iter registry 收集，連同 host 端的 cost/footprint/label
    /// 組成完整 template 廣播給前端（下拉選單成本顯示 + placement 預覽 range）。
    /// 回 `RNone` 表示「這不是 TD 塔」（英雄/敵人 creep 等）。
    fn tower_metadata(&self) -> ROption<TowerMetadata> { RNone }

    /// Called when the entity dies. `killer` = the killing entity if known.
    fn on_death(
        &self,
        _e: EntityHandle,
        _killer: ROption<EntityHandle>,
        _w: &mut GameWorldDyn<'_>,
    ) {
    }

    /// Called on the victim before damage is applied. Script may mutate
    /// `info.amount` to implement shields / damage reduction / reflect.
    fn on_damage_taken(
        &self,
        _e: EntityHandle,
        _info: &mut DamageInfo,
        _w: &mut GameWorldDyn<'_>,
    ) {
    }

    /// Called on the attacker after `on_damage_taken` has resolved the
    /// final amount. Useful for lifesteal, on-hit effects.
    fn on_damage_dealt(
        &self,
        _attacker: EntityHandle,
        _victim: EntityHandle,
        _final_amount: f32,
        _w: &mut GameWorldDyn<'_>,
    ) {
    }

    /// Called on the caster when a skill is activated.
    fn on_skill_cast(
        &self,
        _caster: EntityHandle,
        _skill_id: RStr<'_>,
        _target: Target,
        _w: &mut GameWorldDyn<'_>,
    ) {
    }

    /// Called on the attacker at the moment an attack connects.
    /// Tower-style scripts usually live here (splash, pierce, crit).
    fn on_attack_hit(
        &self,
        _attacker: EntityHandle,
        _victim: EntityHandle,
        _w: &mut GameWorldDyn<'_>,
    ) {
    }

    // ============================================================
    // Dota 2 MODIFIER_EVENT_* 對應 hooks
    // ============================================================

    /// 對應 `MODIFIER_EVENT_ON_ATTACK_START`：攻擊動作準備發射（pre-cast）。
    /// 腳本可在此選擇 target（orb 技能），修改即將出擊的屬性。
    fn on_attack_start(
        &self,
        _attacker: EntityHandle,
        _target: ROption<EntityHandle>,
        _w: &mut GameWorldDyn<'_>,
    ) {
    }

    /// 對應 `MODIFIER_EVENT_ON_ATTACK_LANDED`：攻擊實際命中（在 `on_attack_hit`
    /// 之後由 host 派發，做為一個更通用的 hook 點，含未命中/格擋資訊）。
    fn on_attack_landed(
        &self,
        _attacker: EntityHandle,
        _victim: EntityHandle,
        _damage: f32,
        _w: &mut GameWorldDyn<'_>,
    ) {
    }

    /// 對應 `MODIFIER_EVENT_ON_ATTACK_FAIL`：攻擊失誤（evasion / miss）。
    fn on_attack_fail(
        &self,
        _attacker: EntityHandle,
        _victim: EntityHandle,
        _w: &mut GameWorldDyn<'_>,
    ) {
    }

    /// 對應 `MODIFIER_EVENT_ON_ATTACKED`：本 unit 被攻擊（命中或未命中皆派發）。
    /// 與 `on_damage_taken` 區別：這裡在解析 pre-damage 前就觸發，
    /// 適合做計數器類行為（被攻擊 N 次 → 觸發護盾）。
    fn on_attacked(
        &self,
        _victim: EntityHandle,
        _attacker: EntityHandle,
        _w: &mut GameWorldDyn<'_>,
    ) {
    }

    /// 對應 `MODIFIER_EVENT_ON_HEALTH_GAINED`：HP 淨增加（heal 或 regen）。
    fn on_health_gained(
        &self,
        _e: EntityHandle,
        _amount: f32,
        _w: &mut GameWorldDyn<'_>,
    ) {
    }

    /// 對應 `MODIFIER_EVENT_ON_MANA_GAINED`：MP 淨增加。
    fn on_mana_gained(
        &self,
        _e: EntityHandle,
        _amount: f32,
        _w: &mut GameWorldDyn<'_>,
    ) {
    }

    /// 對應 `MODIFIER_EVENT_ON_SPENT_MANA`：腳本釋放技能花費 mana 後。
    fn on_spent_mana(
        &self,
        _caster: EntityHandle,
        _cost: f32,
        _ability_id: RStr<'_>,
        _w: &mut GameWorldDyn<'_>,
    ) {
    }

    /// 對應 `MODIFIER_EVENT_ON_HEAL_RECEIVED`：回復量被計算完（含 heal_received_multiplier）。
    fn on_heal_received(
        &self,
        _target: EntityHandle,
        _amount: f32,
        _source: ROption<EntityHandle>,
        _w: &mut GameWorldDyn<'_>,
    ) {
    }

    /// 對應 `MODIFIER_EVENT_ON_STATE_CHANGED`：單位狀態改變（stun / silence /
    /// root / invisible / invulnerable 等）。`state_id` 為狀態 id 字串；
    /// `active=true` 代表剛進入，`false` 代表剛離開。
    fn on_state_changed(
        &self,
        _e: EntityHandle,
        _state_id: RStr<'_>,
        _active: bool,
        _w: &mut GameWorldDyn<'_>,
    ) {
    }

    /// 對應 `MODIFIER_EVENT_ON_MODIFIER_ADDED`：身上新增 buff/modifier。
    fn on_modifier_added(
        &self,
        _e: EntityHandle,
        _modifier_id: RStr<'_>,
        _w: &mut GameWorldDyn<'_>,
    ) {
    }

    /// 對應 `MODIFIER_EVENT_ON_MODIFIER_REMOVED`：身上 buff/modifier 過期或被移除。
    fn on_modifier_removed(
        &self,
        _e: EntityHandle,
        _modifier_id: RStr<'_>,
        _w: &mut GameWorldDyn<'_>,
    ) {
    }

    /// 對應 `MODIFIER_EVENT_ON_ORDER`：玩家下達命令（move / attack / cast 等）。
    /// `order_kind` 為命令類型字串（"move" / "attack" / "cast" / "stop" / "hold"）；
    /// `target` 為命令對象。
    fn on_order(
        &self,
        _e: EntityHandle,
        _order_kind: RStr<'_>,
        _target: Target,
        _w: &mut GameWorldDyn<'_>,
    ) {
    }

    /// 對應 `MODIFIER_EVENT_ON_RESPAWN`：英雄復活完成。
    fn on_respawn(
        &self,
        _e: EntityHandle,
        _w: &mut GameWorldDyn<'_>,
    ) {
    }
}
