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
    /// 計算 `e` 朝 `target` 位移 `step` 後的合法位置（避開其他 CollisionRadius
    /// 實體與 BlockedRegion blocker）。策略：直接走 → 只走 X 軸 → 只走 Y 軸 → 停。
    /// 回傳 post-collision 位置；DLL 拿到後可自行 `set_pos` 與 `set_facing`。
    /// 適用於腳本化召喚物/主動位移單位（如 saika_gunner）。
    fn advance_with_collision(
        &mut self,
        e: EntityHandle,
        target: Vec2f,
        step: f32,
    ) -> Vec2f;
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
    /// 查詢 target 身上是否有指定 buff（toggle 技能判斷是否要 remove 用）。
    fn has_buff(&self, target: EntityHandle, buff_id: RStr<'_>) -> bool;
    /// 加帶 payload 的 buff — `modifiers_json` 是一份 JSON 物件字串，例：
    /// `{"range_bonus":300.0,"damage_bonus":0.3,"attack_speed_multiplier":0.7}`。
    /// host 端的 tower_tick / hero_tick 等系統會從 BuffStore 聚合這些值套用
    /// 到單位屬性計算。命名慣例：`_bonus` 為加法加成，`_multiplier` 為乘法倍率。
    fn add_stat_buff(
        &mut self,
        target: EntityHandle,
        buff_id: RStr<'_>,
        duration: f32,
        modifiers_json: RStr<'_>,
    );
    fn spawn_projectile(
        &mut self,
        from: Vec2f,
        to: EntityHandle,
        speed: f32,
        dmg: f32,
        owner: EntityHandle,
    ) -> EntityHandle;
    /// 召喚一個單位：在 `pos` 生成 `unit_type` 指定的預設模板（saika_gunner、
    /// archer、swordsman、mage 等），陣營繼承 `owner`，`duration > 0` 秒後
    /// 由 `summon_tick` 自動 despawn；傳 0 代表永久。回傳新 entity handle。
    fn spawn_summoned_unit(
        &mut self,
        pos: Vec2f,
        unit_type: RStr<'_>,
        owner: EntityHandle,
        duration: f32,
    ) -> EntityHandle;
    /// TD-mode 通用發射 API：依 `ProjectileSpec` 建立 projectile entity 並
    /// 廣播 projectile/C 給前端。支援 Homing / Straight / AoE / Slow。
    fn spawn_projectile_ex(&mut self, spec: ProjectileSpec) -> EntityHandle;
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

    // ============================================================
    // Dota 2 modifier 風格聚合查詢 API
    // ============================================================

    /// 加法聚合：回傳 `e` 身上所有 buff payload 中 `stat_key` 欄位的和。
    /// 慣例：`_bonus` 後綴 stat 用這個（例 `range_bonus`、`bonus_damage`）。
    fn sum_stat(&self, e: EntityHandle, stat_key: RStr<'_>) -> f32;

    /// 乘法聚合：回傳 `e` 身上所有 buff payload 中 `stat_key` 欄位的積。
    /// 空集合回 1.0。慣例：`_multiplier` 後綴 stat 用這個。
    fn product_stat(&self, e: EntityHandle, stat_key: RStr<'_>) -> f32;

    /// 回傳單位的「實際」移速：`base_msd * (1 + move_speed_bonus_sum) *
    /// move_speed_multiplier_product`，並 clamp 到 `move_speed_min/max`（若有 buff）。
    fn get_final_move_speed(&self, e: EntityHandle) -> f32;

    /// 回傳單位的「實際」攻擊力：`base_atk + bonus_damage_sum + base_damage_bonus_sum`，
    /// 再乘以 `damage_out_multiplier_product`。
    fn get_final_atk(&self, e: EntityHandle) -> f32;

    /// 取得該塔第 `path` 路線已升級的等級（0..=4）。
    fn get_tower_upgrade(&self, e: EntityHandle, path: u8) -> u8;

    /// 該塔是否掛了指定的 behavior flag（e.g. "triple_shot"）。
    fn has_tower_flag(&self, e: EntityHandle, flag: RStr<'_>) -> bool;

    /// 對 tower entity 套一個永久 stat buff（供 upgrade 使用）。
    /// `modifiers_json` 應為 `{"key": value}` 形式（與 add_stat_buff 同）。
    fn apply_tower_permanent_buff(&mut self, e: EntityHandle, buff_id: RStr<'_>, modifiers_json: RStr<'_>);

    /// 回傳單位的「實際」攻擊射程：`base_range + attack_range_bonus_sum`。
    fn get_final_attack_range(&self, e: EntityHandle) -> f32;

    /// 回傳指定 buff 還剩多少秒；不存在回 0。
    fn get_buff_remaining(&self, e: EntityHandle, buff_id: RStr<'_>) -> f32;

    /// 讀取當前 mana（英雄/可施法單位）。無法取得時回 0。
    fn current_mana(&self, e: EntityHandle) -> f32;

    /// 扣 mana；足夠時扣除並回傳 true，不足回 false。
    /// 會自動 push `SpentMana` 事件供腳本 hook。
    fn spend_mana(&mut self, e: EntityHandle, amount: f32, ability_id: RStr<'_>) -> bool;

    /// 補 mana，自動 push `ManaGained` 事件。
    fn restore_mana(&mut self, e: EntityHandle, amount: f32);

    /// 從腳本端主動 push 一個 `ModifierAdded` 事件（進階用）。
    /// 一般呼叫 `add_buff` / `add_stat_buff` 時會自動 push；
    /// 這個 API 用於不想改 BuffStore 但要發 hook 的情況。
    fn trigger_modifier_added(&mut self, e: EntityHandle, modifier_id: RStr<'_>);

    /// 從腳本端主動 push `StateChanged` 事件。
    fn trigger_state_changed(&mut self, e: EntityHandle, state_id: RStr<'_>, active: bool);

    // ============================================================
    // Dota 2 modifier property 完整查詢（Phase E）
    // ============================================================

    /// 實際護甲 = base + PHYSICAL_ARMOR_BONUS (+ UNIQUE + UNIQUE_ACTIVE)。
    /// base 來自 `CProperty.def_physic`。
    fn get_final_armor(&self, e: EntityHandle) -> f32;

    /// 實際魔抗（0..1）。支援 MAGICAL_RESISTANCE_DIRECT_MODIFICATION 覆蓋、
    /// MAGICAL_RESISTANCE_BONUS / DECREPIFY_UNIQUE 疊加。
    /// base 來自 `CProperty.def_magic`。
    fn get_final_magic_resist(&self, e: EntityHandle) -> f32;

    /// 閃避機率（0..1）= EVASION_CONSTANT - NEGATIVE_EVASION_CONSTANT clamp。
    fn get_evasion_chance(&self, e: EntityHandle) -> f32;

    /// Miss 機率（0..1）= MISS_PERCENTAGE clamp。
    fn get_miss_chance(&self, e: EntityHandle) -> f32;

    /// 暴擊機率 = PREATTACK_CRITICALSTRIKE clamp 0..1。
    fn get_crit_chance(&self, e: EntityHandle) -> f32;

    /// 暴擊倍率 = CRIT_MULTIPLIER（預設 1.0，若 buff payload 未設）。
    fn get_crit_multiplier(&self, e: EntityHandle) -> f32;

    /// 冷卻倍率 = 1 + COOLDOWN_PERCENTAGE + COOLDOWN_PERCENTAGE_STACKING。
    fn get_cooldown_mult(&self, e: EntityHandle) -> f32;

    /// 是否為建築物（有 `IsBuilding` marker component）。
    fn is_building(&self, e: EntityHandle) -> bool;

    /// HP 上限加值 = HEALTH_BONUS + EXTRA_HEALTH_BONUS。
    fn get_max_hp_bonus(&self, e: EntityHandle) -> f32;

    /// HP regen / 秒（已套 DISABLE_HEALING 與 HP_REGEN_AMPLIFY_PERCENTAGE）。
    fn get_hp_regen(&self, e: EntityHandle) -> f32;

    /// 直接讀 BuffStore 加法聚合（`sum_add(e, key)`）。
    /// 供塔腳本讀 upgrade buff 寫入的任意 key（例如 `crit_chance`, `slow_factor_override`）。
    fn get_stat_bonus(&self, e: EntityHandle, key: RStr<'_>) -> f32;

    /// 對 `at` 點做圓形範圍傷害；射程內所有敵方單位吃 `damage`。
    /// 用於 ring_of_fire / mega_crit 這類 upgrade 派生的 AoE 傷害。
    fn deal_damage_splash(
        &mut self,
        at: Vec2f,
        radius: f32,
        damage: f32,
        kind: DamageKind,
        source: ROption<EntityHandle>,
    );
}
