//! 共用 stat key 常數 — 對應 Dota 2 MODIFIER_PROPERTY_*（~118 項）。
//!
//! 所有 DLL 腳本透過 `world.add_stat_buff(..., modifiers_json)` 寫入屬性修改；
//! host 端 `UnitStats` helper 聚合並套用到 tick 系統 / damage pipeline。
//!
//! 命名慣例（與 BuffStore 聚合對應）：
//! * `*_BONUS` / `*_CONSTANT` / `*_STACKING` → 加法聚合（`sum_add`）；空為 0
//! * `*_PERCENTAGE` → 加法聚合後當倍率（乘到 `1 + sum`）
//! * `*_MULTIPLIER` → 乘法聚合（`product_mult`）；空為 1
//! * `*_CHANCE` → 觸發機率（0..=1）
//!
//! 分三大 section：
//! 1. **全單位通用**（Hero/Creep/Unit/Tower 都吃）
//! 2. **僅非建築物**（有 `IsBuilding` component 的實體跳過）
//! 3. **視覺/前端**（host pass-through only）

// ============================================================
// SECTION 1 — 全單位通用（包括建築物）
// ============================================================

// ---- PreAttack 系 damage (0-7) ----
pub const PREATTACK_BONUS_DAMAGE: &str = "preattack_bonus_damage";
pub const PREATTACK_BONUS_DAMAGE_PROC: &str = "preattack_bonus_damage_proc";
pub const PREATTACK_BONUS_DAMAGE_POST_CRIT: &str = "preattack_bonus_damage_post_crit";
pub const BASEATTACK_BONUS_DAMAGE: &str = "baseattack_bonus_damage";
pub const PROCATTACK_BONUS_DAMAGE_PHYSICAL: &str = "procattack_bonus_damage_physical";
pub const PROCATTACK_BONUS_DAMAGE_MAGICAL: &str = "procattack_bonus_damage_magical";
pub const PROCATTACK_BONUS_DAMAGE_PURE: &str = "procattack_bonus_damage_pure";
pub const PROCATTACK_FEEDBACK: &str = "procattack_feedback";

// ---- 隱形 (9-10) ----
pub const INVISIBILITY_LEVEL: &str = "invisibility_level";
pub const PERSISTENT_INVISIBILITY: &str = "persistent_invisibility";

// ---- 攻速 / BAT / 攻擊節奏 (22-27) ----
pub const ATTACKSPEED_BASE_OVERRIDE: &str = "attackspeed_base_override";
pub const FIXED_ATTACK_RATE: &str = "fixed_attack_rate";
pub const ATTACKSPEED_BONUS_CONSTANT: &str = "attackspeed_bonus_constant";
pub const COOLDOWN_REDUCTION_CONSTANT: &str = "cooldown_reduction_constant";
pub const BASE_ATTACK_TIME_CONSTANT: &str = "base_attack_time_constant";
pub const ATTACK_POINT_CONSTANT: &str = "attack_point_constant";

// ---- 傷害輸出 multiplier (28-35) ----
pub const DAMAGEOUTGOING_PERCENTAGE: &str = "damageoutgoing_percentage";
pub const TOTALDAMAGEOUTGOING_PERCENTAGE: &str = "totaldamageoutgoing_percentage";
pub const SPELL_AMPLIFY_PERCENTAGE: &str = "spell_amplify_percentage";
pub const HP_REGEN_AMPLIFY_PERCENTAGE: &str = "hp_regen_amplify_percentage";
pub const MAGICDAMAGEOUTGOING_PERCENTAGE: &str = "magicdamageoutgoing_percentage";
pub const BASEDAMAGEOUTGOING_PERCENTAGE: &str = "basedamageoutgoing_percentage";
pub const BASEDAMAGEOUTGOING_PERCENTAGE_UNIQUE: &str = "basedamageoutgoing_percentage_unique";

// ---- 傷害承受 (36-39) ----
pub const INCOMING_DAMAGE_PERCENTAGE: &str = "incoming_damage_percentage";
pub const INCOMING_PHYSICAL_DAMAGE_PERCENTAGE: &str = "incoming_physical_damage_percentage";
pub const INCOMING_PHYSICAL_DAMAGE_CONSTANT: &str = "incoming_physical_damage_constant";
pub const INCOMING_SPELL_DAMAGE_CONSTANT: &str = "incoming_spell_damage_constant";

// ---- 閃避 / miss (40-44) ----
pub const EVASION_CONSTANT: &str = "evasion_constant";
pub const NEGATIVE_EVASION_CONSTANT: &str = "negative_evasion_constant";
pub const AVOID_DAMAGE: &str = "avoid_damage";
pub const AVOID_SPELL: &str = "avoid_spell";
pub const MISS_PERCENTAGE: &str = "miss_percentage";

// ---- 護甲 / 魔抗 (45-51) ----
pub const PHYSICAL_ARMOR_BONUS: &str = "physical_armor_bonus";
pub const PHYSICAL_ARMOR_BONUS_UNIQUE: &str = "physical_armor_bonus_unique";
pub const PHYSICAL_ARMOR_BONUS_UNIQUE_ACTIVE: &str = "physical_armor_bonus_unique_active";
pub const IGNORE_PHYSICAL_ARMOR: &str = "ignore_physical_armor";
pub const MAGICAL_RESISTANCE_DIRECT_MODIFICATION: &str = "magical_resistance_direct_modification";
pub const MAGICAL_RESISTANCE_BONUS: &str = "magical_resistance_bonus";
pub const MAGICAL_RESISTANCE_DECREPIFY_UNIQUE: &str = "magical_resistance_decrepify_unique";

// ---- Mana regen (52-56) ----
pub const BASE_MANA_REGEN: &str = "base_mana_regen";
pub const MANA_REGEN_CONSTANT: &str = "mana_regen_constant";
pub const MANA_REGEN_CONSTANT_UNIQUE: &str = "mana_regen_constant_unique";
pub const MANA_REGEN_PERCENTAGE: &str = "mana_regen_percentage";
pub const MANA_REGEN_TOTAL_PERCENTAGE: &str = "mana_regen_total_percentage";

// ---- HP regen (57-58) ----
pub const HEALTH_REGEN_CONSTANT: &str = "health_regen_constant";
pub const HEALTH_REGEN_PERCENTAGE: &str = "health_regen_percentage";

// ---- HP/Mana 上限 (59-64) ----
pub const HEALTH_BONUS: &str = "health_bonus";
pub const MANA_BONUS: &str = "mana_bonus";
pub const EXTRA_STRENGTH_BONUS: &str = "extra_strength_bonus";
pub const EXTRA_HEALTH_BONUS: &str = "extra_health_bonus";
pub const EXTRA_MANA_BONUS: &str = "extra_mana_bonus";
pub const EXTRA_HEALTH_PERCENTAGE: &str = "extra_health_percentage";

// ---- 主屬性 (65-67) ----
pub const STATS_STRENGTH_BONUS: &str = "stats_strength_bonus";
pub const STATS_AGILITY_BONUS: &str = "stats_agility_bonus";
pub const STATS_INTELLECT_BONUS: &str = "stats_intellect_bonus";

// ---- 射程 / 彈速 (68-73) ----
pub const CAST_RANGE_BONUS: &str = "cast_range_bonus";
pub const CAST_RANGE_BONUS_STACKING: &str = "cast_range_bonus_stacking";
pub const ATTACK_RANGE_BONUS: &str = "attack_range_bonus";
pub const ATTACK_RANGE_BONUS_UNIQUE: &str = "attack_range_bonus_unique";
pub const MAX_ATTACK_RANGE: &str = "max_attack_range";
pub const PROJECTILE_SPEED_BONUS: &str = "projectile_speed_bonus";

// ---- 冷卻 / 施法時間 / 魔耗 (78-81) ----
pub const COOLDOWN_PERCENTAGE: &str = "cooldown_percentage";
pub const COOLDOWN_PERCENTAGE_STACKING: &str = "cooldown_percentage_stacking";
pub const CASTTIME_PERCENTAGE: &str = "casttime_percentage";
pub const MANACOST_PERCENTAGE: &str = "manacost_percentage";

// ---- 暴擊 (84-85) ----
pub const PREATTACK_CRITICALSTRIKE: &str = "preattack_criticalstrike";
pub const PREATTACK_TARGET_CRITICALSTRIKE: &str = "preattack_target_criticalstrike";
/// 非 Dota 原生 key 但常用 — 腳本寫暴擊倍率用
pub const CRIT_MULTIPLIER: &str = "crit_multiplier";

// ---- Block (86-90) ----
pub const MAGICAL_CONSTANT_BLOCK: &str = "magical_constant_block";
pub const PHYSICAL_CONSTANT_BLOCK: &str = "physical_constant_block";
pub const PHYSICAL_CONSTANT_BLOCK_SPECIAL: &str = "physical_constant_block_special";
pub const TOTAL_CONSTANT_BLOCK_UNAVOIDABLE_PRE_ARMOR: &str = "total_constant_block_unavoidable_pre_armor";
pub const TOTAL_CONSTANT_BLOCK: &str = "total_constant_block";

// ---- 吸收 / 反射 spell (94-96) ----
pub const ABSORB_SPELL: &str = "absorb_spell";
pub const REFLECT_SPELL: &str = "reflect_spell";
pub const DISABLE_AUTOATTACK: &str = "disable_autoattack";

// ---- 絕對下限 / 不受特定類型傷害 (103-106) ----
pub const MIN_HEALTH: &str = "min_health";
pub const ABSOLUTE_NO_DAMAGE_PHYSICAL: &str = "absolute_no_damage_physical";
pub const ABSOLUTE_NO_DAMAGE_MAGICAL: &str = "absolute_no_damage_magical";
pub const ABSOLUTE_NO_DAMAGE_PURE: &str = "absolute_no_damage_pure";

// ---- 治療 / 攻擊開關 (112-114) ----
pub const DISABLE_HEALING: &str = "disable_healing";
pub const ALWAYS_ALLOW_ATTACK: &str = "always_allow_attack";
pub const OVERRIDE_ATTACK_MAGICAL: &str = "override_attack_magical";

// ---- 其他常用 ----
pub const LIFESTEAL: &str = "lifesteal";
pub const HEAL_RECEIVED_MULTIPLIER: &str = "heal_received_multiplier";
pub const UNIT_STATS_NEEDS_REFRESH: &str = "unit_stats_needs_refresh";

// ---- 專案自訂（非 Dota 原生，但 towers / heroes 普遍使用）----
// 這些 key 在 tower 升級、hero ability 都會進 BuffStore payload；
// 統一收在這邊是為了避免 script 與 registry 用 magic string 造成拼寫分歧。
pub const ACCURACY_BONUS: &str = "accuracy_bonus";
pub const SPLASH_BONUS: &str = "splash_bonus";
pub const CRIT_BONUS: &str = "crit_bonus";
pub const SLOW_FACTOR_OVERRIDE: &str = "slow_factor_override";
pub const SLOW_DURATION_BONUS: &str = "slow_duration_bonus";
/// 攻速 multiplier（乘法聚合：product_mult，空為 1）。
/// 語意：攻擊間隔直接乘此值；0.83 = 攻擊間隔 ×0.83（攻速 +20%）。
/// 與 `ATTACKSPEED_BONUS_CONSTANT`（Dota 加法 bonus points）互為不同語意。
pub const ATTACK_SPEED_MULTIPLIER: &str = "attack_speed_multiplier";

// ============================================================
// SECTION 2 — 僅非建築物（IsBuilding 的實體跳過）
// ============================================================

// ---- 移速 (11-21) ----
pub const MOVESPEED_BONUS_CONSTANT: &str = "movespeed_bonus_constant";
pub const MOVESPEED_BASE_OVERRIDE: &str = "movespeed_base_override";
pub const MOVESPEED_BONUS_PERCENTAGE: &str = "movespeed_bonus_percentage";
pub const MOVESPEED_BONUS_PERCENTAGE_UNIQUE: &str = "movespeed_bonus_percentage_unique";
pub const MOVESPEED_BONUS_PERCENTAGE_UNIQUE_2: &str = "movespeed_bonus_percentage_unique_2";
pub const MOVESPEED_BONUS_UNIQUE: &str = "movespeed_bonus_unique";
pub const MOVESPEED_BONUS_UNIQUE_2: &str = "movespeed_bonus_unique_2";
pub const MOVESPEED_ABSOLUTE: &str = "movespeed_absolute";
pub const MOVESPEED_ABSOLUTE_MIN: &str = "movespeed_absolute_min";
pub const MOVESPEED_LIMIT: &str = "movespeed_limit";
pub const MOVESPEED_MAX: &str = "movespeed_max";

// ---- 轉向 (111) ----
pub const TURN_RATE_PERCENTAGE: &str = "turn_rate_percentage";

// ---- 復活 (74-77) ----
pub const REINCARNATION: &str = "reincarnation";
pub const RESPAWNTIME: &str = "respawntime";
pub const RESPAWNTIME_PERCENTAGE: &str = "respawntime_percentage";
pub const RESPAWNTIME_STACKING: &str = "respawntime_stacking";

// ---- 死亡代價 / 經驗 (82-83) ----
pub const DEATHGOLDCOST: &str = "deathgoldcost";
pub const EXP_RATE_BOOST: &str = "exp_rate_boost";

// ---- 視野 (97-102) ----
pub const BONUS_DAY_VISION: &str = "bonus_day_vision";
pub const BONUS_NIGHT_VISION: &str = "bonus_night_vision";
pub const BONUS_NIGHT_VISION_UNIQUE: &str = "bonus_night_vision_unique";
pub const BONUS_VISION_PERCENTAGE: &str = "bonus_vision_percentage";
pub const FIXED_DAY_VISION: &str = "fixed_day_vision";
pub const FIXED_NIGHT_VISION: &str = "fixed_night_vision";

// ---- 幻象 (107-110, 29) ----
pub const IS_ILLUSION: &str = "is_illusion";
pub const ILLUSION_LABEL: &str = "illusion_label";
pub const SUPER_ILLUSION: &str = "super_illusion";
pub const SUPER_ILLUSION_WITH_ULTIMATE: &str = "super_illusion_with_ultimate";
pub const DAMAGEOUTGOING_PERCENTAGE_ILLUSION: &str = "damageoutgoing_percentage_illusion";

// ---- 賞金 (116-117) ----
pub const BOUNTY_CREEP_MULTIPLIER: &str = "bounty_creep_multiplier";
pub const BOUNTY_OTHER_MULTIPLIER: &str = "bounty_other_multiplier";

// ============================================================
// SECTION 3 — 視覺 / 前端（host pass-through only）
// ============================================================

// ---- Animation (8, 91-93) ----
pub const PRE_ATTACK: &str = "pre_attack";
pub const OVERRIDE_ANIMATION: &str = "override_animation";
pub const OVERRIDE_ANIMATION_WEIGHT: &str = "override_animation_weight";
pub const OVERRIDE_ANIMATION_RATE: &str = "override_animation_rate";

// ============================================================
// 控制效果額外 key（非 bonus 聚合，是 runtime flag / 觸發）
// ============================================================
pub const DISABLE_TURNING: &str = "disable_turning";
pub const IGNORE_CAST_ANGLE: &str = "ignore_cast_angle";
pub const STUN_CHANCE: &str = "stun_chance";
pub const STUN_DURATION: &str = "stun_duration";

// 控制 buff id 常數
pub const BUFF_ID_STUN: &str = "stun";
pub const BUFF_ID_ROOT: &str = "root";
pub const BUFF_ID_SILENCE: &str = "silence";
pub const BUFF_ID_INVISIBLE: &str = "invisible";
pub const BUFF_ID_INVULNERABLE: &str = "invulnerable";

// ============================================================
// BUILDING_EXCLUDED_KEYS — 建築物 tick 系統讀取時會 skip 這組
// ============================================================

pub const BUILDING_EXCLUDED_KEYS: &[&str] = &[
    // 移速系
    MOVESPEED_BONUS_CONSTANT,
    MOVESPEED_BASE_OVERRIDE,
    MOVESPEED_BONUS_PERCENTAGE,
    MOVESPEED_BONUS_PERCENTAGE_UNIQUE,
    MOVESPEED_BONUS_PERCENTAGE_UNIQUE_2,
    MOVESPEED_BONUS_UNIQUE,
    MOVESPEED_BONUS_UNIQUE_2,
    MOVESPEED_ABSOLUTE,
    MOVESPEED_ABSOLUTE_MIN,
    MOVESPEED_LIMIT,
    MOVESPEED_MAX,
    // 轉向
    TURN_RATE_PERCENTAGE,
    // 復活
    REINCARNATION,
    RESPAWNTIME,
    RESPAWNTIME_PERCENTAGE,
    RESPAWNTIME_STACKING,
    // 死亡代價 / 經驗
    DEATHGOLDCOST,
    EXP_RATE_BOOST,
    // 視野
    BONUS_DAY_VISION,
    BONUS_NIGHT_VISION,
    BONUS_NIGHT_VISION_UNIQUE,
    BONUS_VISION_PERCENTAGE,
    FIXED_DAY_VISION,
    FIXED_NIGHT_VISION,
    // Illusion
    IS_ILLUSION,
    ILLUSION_LABEL,
    SUPER_ILLUSION,
    SUPER_ILLUSION_WITH_ULTIMATE,
    DAMAGEOUTGOING_PERCENTAGE_ILLUSION,
    // Bounty
    BOUNTY_CREEP_MULTIPLIER,
    BOUNTY_OTHER_MULTIPLIER,
];
