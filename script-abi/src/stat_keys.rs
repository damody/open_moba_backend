//! 共用 stat key 常數 — 參考 Dota 2 MODIFIER_PROPERTY_*。
//!
//! 所有 DLL 腳本透過 `world.add_stat_buff(..., modifiers_json)` 寫入屬性修改；
//! host 端 tick 系統（hero_tick / creep_tick / tower_tick / damage pipeline）
//! 用 `BuffStore::sum_add(key)` 或 `product_mult(key)` 聚合。
//!
//! 命名慣例：
//! * `_BONUS` 後綴 → 加法聚合（`sum_add`）；空為 0
//! * `_MULTIPLIER` 後綴 → 乘法聚合（`product_mult`）；空為 1
//! * `_CHANCE` 後綴 → 觸發機率（0..=1），腳本自己抽 `rand_f32()` 比對
//!
//! 取 key 的地方不直接打字串，呼叫這裡的常數避免兩邊拼寫分歧。

// ---- 傷害 / 攻擊 ----
pub const BONUS_DAMAGE: &str = "bonus_damage";
pub const BONUS_DAMAGE_PROC: &str = "bonus_damage_proc";
pub const BONUS_DAMAGE_POST_CRIT: &str = "bonus_damage_post_crit";
pub const BASE_DAMAGE_BONUS: &str = "base_damage_bonus";
pub const DAMAGE_OUT_MULTIPLIER: &str = "damage_out_multiplier";
pub const DAMAGE_IN_MULTIPLIER: &str = "damage_in_multiplier";
pub const MAGIC_DAMAGE_OUT_MULTIPLIER: &str = "magic_damage_out_multiplier";
pub const SPELL_AMP_BONUS: &str = "spell_amp_bonus";

// ---- 移速 ----
pub const MOVE_SPEED_BONUS: &str = "move_speed_bonus";
pub const MOVE_SPEED_MULTIPLIER: &str = "move_speed_multiplier";
pub const MOVE_SPEED_ABSOLUTE: &str = "move_speed_absolute";
pub const MOVE_SPEED_MIN: &str = "move_speed_min";
pub const MOVE_SPEED_MAX: &str = "move_speed_max";
pub const MOVE_SPEED_LIMIT: &str = "move_speed_limit";

// ---- 攻速 / BAT / 冷卻 ----
pub const ATTACK_SPEED_BONUS: &str = "attack_speed_bonus";
pub const ATTACK_SPEED_MULTIPLIER: &str = "attack_speed_multiplier";
pub const BASE_ATTACK_TIME_OVERRIDE: &str = "base_attack_time_override";
pub const COOLDOWN_REDUCTION_MULTIPLIER: &str = "cooldown_reduction_multiplier";
pub const COOLDOWN_REDUCTION_STACKING: &str = "cooldown_reduction_stacking";
pub const CAST_TIME_MULTIPLIER: &str = "cast_time_multiplier";
pub const MANA_COST_MULTIPLIER: &str = "mana_cost_multiplier";

// ---- 射程 / 彈速 ----
pub const ATTACK_RANGE_BONUS: &str = "attack_range_bonus";
pub const ATTACK_RANGE_BONUS_UNIQUE: &str = "attack_range_bonus_unique";
pub const CAST_RANGE_BONUS: &str = "cast_range_bonus";
pub const PROJECTILE_SPEED_BONUS: &str = "projectile_speed_bonus";

// ---- 生命 / 魔力 ----
pub const HEALTH_BONUS: &str = "health_bonus";
pub const MANA_BONUS: &str = "mana_bonus";
pub const HP_REGEN_BONUS: &str = "hp_regen_bonus";
pub const HP_REGEN_MULTIPLIER: &str = "hp_regen_multiplier";
pub const MANA_REGEN_BONUS: &str = "mana_regen_bonus";
pub const MANA_REGEN_MULTIPLIER: &str = "mana_regen_multiplier";
pub const MANA_REGEN_TOTAL_PERCENTAGE: &str = "mana_regen_total_percentage";

// ---- 主屬性 ----
pub const STRENGTH_BONUS: &str = "strength_bonus";
pub const AGILITY_BONUS: &str = "agility_bonus";
pub const INTELLECT_BONUS: &str = "intellect_bonus";

// ---- 防禦 ----
pub const ARMOR_PHYSICAL_BONUS: &str = "armor_physical_bonus";
pub const ARMOR_MAGICAL_BONUS: &str = "armor_magical_bonus";
pub const MAGIC_RESIST_BONUS: &str = "magic_resist_bonus";
pub const EVASION_CHANCE: &str = "evasion_chance";
pub const MISS_CHANCE: &str = "miss_chance";

// ---- 暴擊 / 格擋 ----
pub const CRIT_CHANCE: &str = "crit_chance";
pub const CRIT_MULTIPLIER: &str = "crit_multiplier";
pub const CRIT_TARGET_MULTIPLIER: &str = "crit_target_multiplier";
pub const BLOCK_PHYSICAL: &str = "block_physical";
pub const BLOCK_MAGICAL: &str = "block_magical";
pub const BLOCK_TOTAL: &str = "block_total";

// ---- 位移 / 轉向 ----
pub const TURN_RATE_MULTIPLIER: &str = "turn_rate_multiplier";
pub const DISABLE_TURNING: &str = "disable_turning";
pub const IGNORE_CAST_ANGLE: &str = "ignore_cast_angle";

// ---- 其他狀態類 ----
pub const STUN_CHANCE: &str = "stun_chance";
pub const STUN_DURATION: &str = "stun_duration";
pub const HEAL_RECEIVED_MULTIPLIER: &str = "heal_received_multiplier";
pub const HEAL_DISABLED: &str = "heal_disabled";
pub const LIFESTEAL: &str = "lifesteal";
pub const ALWAYS_ALLOW_ATTACK: &str = "always_allow_attack";

// ---- 復活 ----
pub const RESPAWN_TIME_BONUS: &str = "respawn_time_bonus";
pub const RESPAWN_TIME_MULTIPLIER: &str = "respawn_time_multiplier";

// ---- 視野 ----
pub const VISION_DAY_BONUS: &str = "vision_day_bonus";
pub const VISION_NIGHT_BONUS: &str = "vision_night_bonus";
pub const VISION_MULTIPLIER: &str = "vision_multiplier";

// ---- 護盾 / 吸收 ----
pub const ABSORB_SPELL: &str = "absorb_spell";
pub const REFLECT_SPELL: &str = "reflect_spell";
pub const DAMAGE_PREVENTION_PHYSICAL: &str = "damage_prevention_physical";
pub const DAMAGE_PREVENTION_MAGICAL: &str = "damage_prevention_magical";
pub const DAMAGE_PREVENTION_PURE: &str = "damage_prevention_pure";

// ---- 常用控制 buff id（不是 stat key，是 `has_buff` 檢查用）----
pub const BUFF_ID_STUN: &str = "stun";
pub const BUFF_ID_ROOT: &str = "root";
pub const BUFF_ID_SILENCE: &str = "silence";
pub const BUFF_ID_INVISIBLE: &str = "invisible";
pub const BUFF_ID_INVULNERABLE: &str = "invulnerable";
