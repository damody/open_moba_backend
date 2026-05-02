//! `UnitStats` — 集中「最終屬性」計算 helper。
//!
//! Dota 2 modifier property 系統的 host 端實裝：
//! 所有 tick 系統（creep_tick / hero_tick / tower_tick / damage pipeline）
//! 統一透過這裡取最終數值，避免各自呼 `BuffStore::sum_add` 造成 key 拼寫分歧。
//!
//! 建築物識別：`IsBuilding` component 存在 → 跳過 movespeed / respawn / vision
//! / illusion / bounty 類屬性聚合。
//!
//! 讀 base value：若 entity 有 component 欄位就用那個當 base（TAttack.atk_physic
//! 為 atk base、CProperty.msd 為 move base 等）；其餘未內建欄位的屬性（crit /
//! armor / magic_resist 等）由 spawn 腳本 `on_spawn` 打 duration=∞ 的 base_stats
//! buff 提供基底。

use omb_script_abi::stat_keys::StatKey;
use omb_script_abi::types::DamageKind;
use omoba_sim::Fixed32;
use specs::Entity;

use crate::ability_runtime::BuffStore;

/// Per-tick snapshot of a unit's stat-aggregation context.
/// Build once, query N times — cheap (holds references only).
pub struct UnitStats<'a> {
    pub buffs: &'a BuffStore,
    pub is_building: bool,
}

impl<'a> UnitStats<'a> {
    /// 建一個 `UnitStats`。呼叫端需自行取 `BuffStore` 和 `IsBuilding` 的 borrow，
    /// 再傳進來 — 這樣符合 specs `SystemData` 的 resource lock 流程，
    /// 避免在 System 內部再 `read_resource` 衝突。
    ///
    /// 典型用法（System run 裡）：
    /// ```ignore
    /// let stats = UnitStats::from_refs(&*buffs, is_buildings.get(e).is_some());
    /// let msd = stats.final_move_speed(cp.msd, e);
    /// ```
    pub fn from_refs(buffs: &'a BuffStore, is_building: bool) -> Self {
        Self { buffs, is_building }
    }

    // ================= 位移 =================

    pub fn final_move_speed(&self, base: Fixed32, e: Entity) -> Fixed32 {
        if self.is_building {
            return Fixed32::ZERO;
        }
        let abs = self.buffs.sum_add(e, StatKey::MoveSpeedAbsolute);
        let effective = if abs > Fixed32::ZERO {
            abs
        } else {
            let override_base = self.buffs.sum_add(e, StatKey::MoveSpeedBaseOverride);
            let base_eff = if override_base > Fixed32::ZERO { override_base } else { base };
            // Equipment flat（boots、靴類道具）：跟 base 一起被 percentage 縮放
            let bonus_c = self.buffs.sum_add(e, StatKey::MoveSpeedBonusEquipment);
            // Percentage（含 ice tower 用的 MoveSpeedBonus，當 -50% 寫進去）
            let pct = self.buffs.sum_add(e, StatKey::MoveSpeedBonusPercentage)
                + self.buffs.sum_add(e, StatKey::MoveSpeedBonusPercentageUnique)
                + self.buffs.sum_add(e, StatKey::MoveSpeedBonusPercentageUnique2)
                + self.buffs.sum_add(e, StatKey::MoveSpeedBonus);
            // Buff flat post-percentage：不被 slow 削弱、不疊到 base/equipment 上
            let buff_bonus = self.buffs.sum_add(e, StatKey::MoveSpeedBonusBuff);
            (base_eff + bonus_c) * (Fixed32::ONE + pct) + buff_bonus
        };
        self.apply_move_clamp(effective, e)
    }

    fn apply_move_clamp(&self, v: Fixed32, e: Entity) -> Fixed32 {
        let min_abs = self.buffs.sum_add(e, StatKey::MoveSpeedAbsoluteMin);
        let max_abs = self.buffs.sum_add(e, StatKey::MoveSpeedMax);
        let limit = self.buffs.sum_add(e, StatKey::MoveSpeedLimit);
        let mut r = v;
        if min_abs > Fixed32::ZERO && r < min_abs {
            r = min_abs;
        }
        if max_abs > Fixed32::ZERO && r > max_abs {
            r = max_abs;
        }
        if limit > Fixed32::ZERO && r > limit {
            r = limit;
        }
        if r < Fixed32::ZERO { Fixed32::ZERO } else { r }
    }

    pub fn turn_rate_mult(&self, e: Entity) -> Fixed32 {
        if self.is_building {
            return Fixed32::ONE;
        }
        Fixed32::ONE + self.buffs.sum_add(e, StatKey::TurnRatePercentage)
    }

    // ================= 攻擊 =================

    pub fn final_atk(&self, base: Fixed32, e: Entity) -> Fixed32 {
        let bonus = self.buffs.sum_add(e, StatKey::PreattackBonusDamage)
            + self.buffs.sum_add(e, StatKey::BaseAttackBonusDamage);
        let pct_total = self.buffs.sum_add(e, StatKey::TotalDamageOutgoingPercentage);
        let pct_base = self.buffs.sum_add(e, StatKey::BaseDamageOutgoingPercentage)
            + self.buffs.sum_add(e, StatKey::BaseDamageOutgoingPercentageUnique);
        let mult = Fixed32::ONE + pct_total + pct_base;
        let v = (base + bonus) * mult;
        if v < Fixed32::ZERO { Fixed32::ZERO } else { v }
    }

    /// 攻速倍數（乘到 base attack interval 上）。
    /// Dota: effective_attacks_per_sec = base × (1 + as_bonus / 100)
    /// 簡化：以 bonus/100 當 multiplier 加成；fixed_attack_rate 若設則覆蓋。
    /// 另疊 `ATTACK_SPEED_MULTIPLIER`（專案自訂 product_mult，tower upgrade 用）。
    pub fn final_attack_speed_mult(&self, e: Entity) -> Fixed32 {
        let fixed = self.buffs.sum_add(e, StatKey::FixedAttackRate);
        if fixed > Fixed32::ZERO {
            return fixed;
        }
        let as_bonus = self.buffs.sum_add(e, StatKey::AttackSpeedBonusConstant);
        let hundred = Fixed32::from_i32(100);
        let one_tenth = Fixed32::from_raw(102); // 0.1 in Q22.10 (102/1024 ≈ 0.0996)
        let constant_mult_raw = Fixed32::ONE + as_bonus / hundred;
        let constant_mult = if constant_mult_raw < one_tenth { one_tenth } else { constant_mult_raw };
        let extra_mult = self.buffs.product_mult(e, StatKey::AttackSpeedMultiplier);
        let v = constant_mult * extra_mult;
        if v < one_tenth { one_tenth } else { v }
    }

    /// 射程 = base + ATTACK_RANGE_BONUS + ATTACK_RANGE_BONUS_UNIQUE，
    /// 再由 MAX_ATTACK_RANGE 上限（若設）。
    pub fn final_attack_range(&self, base: Fixed32, e: Entity) -> Fixed32 {
        let bonus = self.buffs.sum_add(e, StatKey::AttackRangeBonus)
            + self.buffs.sum_add(e, StatKey::AttackRangeBonusUnique);
        let raw = base + bonus;
        let r = if raw < Fixed32::ZERO { Fixed32::ZERO } else { raw };
        let max = self.buffs.sum_add(e, StatKey::MaxAttackRange);
        if max > Fixed32::ZERO && r > max {
            max
        } else {
            r
        }
    }

    pub fn final_cast_range(&self, base: Fixed32, e: Entity) -> Fixed32 {
        let v = base
            + self.buffs.sum_add(e, StatKey::CastRangeBonus)
            + self.buffs.sum_add(e, StatKey::CastRangeBonusStacking);
        if v < Fixed32::ZERO { Fixed32::ZERO } else { v }
    }

    // ================= 防禦 =================

    pub fn final_armor(&self, base: Fixed32, e: Entity) -> Fixed32 {
        base + self.buffs.sum_add(e, StatKey::PhysicalArmorBonus)
            + self.buffs.sum_add(e, StatKey::PhysicalArmorBonusUnique)
            + self.buffs.sum_add(e, StatKey::PhysicalArmorBonusUniqueActive)
    }

    /// 魔抗：0..1 = 百分比。direct_modification 若存在 → 覆蓋 base + bonus。
    pub fn final_magic_resist(&self, base: Fixed32, e: Entity) -> Fixed32 {
        let direct = self.buffs.sum_add(e, StatKey::MagicalResistanceDirectModification);
        if direct > Fixed32::ZERO {
            return clamp_fx(direct, Fixed32::ZERO, Fixed32::ONE);
        }
        let bonus = self.buffs.sum_add(e, StatKey::MagicalResistanceBonus);
        let decrepify = self.buffs.sum_add(e, StatKey::MagicalResistanceDecrepifyUnique);
        let hundred = Fixed32::from_i32(100);
        // Dota 疊加公式：1 - (1-r1)(1-r2)...
        let combined = Fixed32::ONE
            - (Fixed32::ONE - base)
                * (Fixed32::ONE - bonus / hundred)
                * (Fixed32::ONE - decrepify / hundred);
        clamp_fx(combined, Fixed32::ZERO - Fixed32::ONE, Fixed32::ONE)
    }

    // ================= 命中率 =================

    pub fn evasion_chance(&self, e: Entity) -> Fixed32 {
        let v = self.buffs.sum_add(e, StatKey::EvasionConstant)
            - self.buffs.sum_add(e, StatKey::NegativeEvasionConstant);
        clamp_fx(v, Fixed32::ZERO, Fixed32::ONE)
    }

    pub fn miss_chance(&self, e: Entity) -> Fixed32 {
        clamp_fx(
            self.buffs.sum_add(e, StatKey::MissPercentage),
            Fixed32::ZERO,
            Fixed32::ONE,
        )
    }

    /// 回 (chance, multiplier)；chance 為 0..1；multiplier 預設 1.0（無暴擊）
    pub fn crit(&self, e: Entity) -> (Fixed32, Fixed32) {
        let chance = clamp_fx(
            self.buffs.sum_add(e, StatKey::PreattackCriticalStrike),
            Fixed32::ZERO,
            Fixed32::ONE,
        );
        let mult_raw = self.buffs.sum_add(e, StatKey::CritMultiplier);
        let mult = if mult_raw > Fixed32::ZERO { mult_raw } else { Fixed32::ONE };
        (chance, mult)
    }

    // ================= CD / 施法 =================

    /// Cooldown percentage multiplier: final_cd = base_cd × (1 + pct + stacking)
    pub fn cooldown_mult(&self, e: Entity) -> Fixed32 {
        let pct = self.buffs.sum_add(e, StatKey::CooldownPercentage);
        let stacking = self.buffs.sum_add(e, StatKey::CooldownPercentageStacking);
        let one_tenth = Fixed32::from_raw(102);
        let v = Fixed32::ONE + pct + stacking;
        if v < one_tenth { one_tenth } else { v }
    }

    pub fn cast_time_mult(&self, e: Entity) -> Fixed32 {
        let one_tenth = Fixed32::from_raw(102);
        let v = Fixed32::ONE + self.buffs.sum_add(e, StatKey::CastTimePercentage);
        if v < one_tenth { one_tenth } else { v }
    }

    pub fn mana_cost_mult(&self, e: Entity) -> Fixed32 {
        let v = Fixed32::ONE + self.buffs.sum_add(e, StatKey::ManaCostPercentage);
        if v < Fixed32::ZERO { Fixed32::ZERO } else { v }
    }

    // ================= 回復 =================

    pub fn hp_regen(&self, base: Fixed32, e: Entity) -> Fixed32 {
        let half = Fixed32::from_raw(512); // 0.5 in Q22.10
        if self.buffs.has(e, StatKey::DisableHealing.as_str())
            || self.buffs.sum_add(e, StatKey::DisableHealing) > half
        {
            return Fixed32::ZERO;
        }
        let bonus = self.buffs.sum_add(e, StatKey::HealthRegenConstant);
        let pct = self.buffs.sum_add(e, StatKey::HealthRegenPercentage);
        let amp = Fixed32::ONE + self.buffs.sum_add(e, StatKey::HpRegenAmplifyPercentage);
        let v = (base + bonus) * (Fixed32::ONE + pct) * amp;
        if v < Fixed32::ZERO { Fixed32::ZERO } else { v }
    }

    pub fn mana_regen(&self, base: Fixed32, e: Entity) -> Fixed32 {
        let base_override = self.buffs.sum_add(e, StatKey::BaseManaRegen);
        let base_eff = if base_override > Fixed32::ZERO { base_override } else { base };
        let bonus = self.buffs.sum_add(e, StatKey::ManaRegenConstant)
            + self.buffs.sum_add(e, StatKey::ManaRegenConstantUnique);
        let pct = self.buffs.sum_add(e, StatKey::ManaRegenPercentage);
        let total_pct = self.buffs.sum_add(e, StatKey::ManaRegenTotalPercentage);
        let v = (((base_eff + bonus) * (Fixed32::ONE + pct)) * (Fixed32::ONE + total_pct));
        if v < Fixed32::ZERO { Fixed32::ZERO } else { v }
    }

    // ================= HP / Mana 上限 =================

    pub fn max_hp_bonus(&self, e: Entity) -> Fixed32 {
        self.buffs.sum_add(e, StatKey::HealthBonus)
            + self.buffs.sum_add(e, StatKey::ExtraHealthBonus)
    }

    pub fn max_mp_bonus(&self, e: Entity) -> Fixed32 {
        self.buffs.sum_add(e, StatKey::ManaBonus)
            + self.buffs.sum_add(e, StatKey::ExtraManaBonus)
    }

    // ================= Damage pipeline 入口 =================

    /// 計算 `e`（victim）承受 `raw` damage 後的 final 值（含 block / armor / resist / prevention）。
    /// NOTE: evasion / miss 由呼叫端先 roll，此函式假設攻擊已命中。
    /// Phase 1c.3: damage / armor / resist 仍為 f32（damage pipeline 完整 Fixed32 化是 1c.4）。
    /// 內部 sum_add 回 Fixed32 → 在此 boundary 暫時 to_f32_for_render。
    /// TODO Phase 1[d]: rewrite this in Fixed32 once outcome handlers / DamageInstance migrate.
    pub fn apply_incoming_damage(
        &self,
        raw: f32,
        kind: DamageKind,
        e: Entity,
        base_armor: f32,
        base_resist: f32,
    ) -> f32 {
        let half_fx = Fixed32::from_raw(512); // 0.5 in Q22.10
        // 1. 絕對免疫
        match kind {
            DamageKind::Physical
                if self.buffs.sum_add(e, StatKey::AbsoluteNoDamagePhysical) > half_fx =>
            {
                return 0.0
            }
            DamageKind::Magical
                if self.buffs.sum_add(e, StatKey::AbsoluteNoDamageMagical) > half_fx =>
            {
                return 0.0
            }
            DamageKind::Pure
                if self.buffs.sum_add(e, StatKey::AbsoluteNoDamagePure) > half_fx =>
            {
                return 0.0
            }
            _ => {}
        }

        // 2. Block（無法避免、先套）
        let unavoid_block = self.buffs.sum_add(e, StatKey::TotalConstantBlockUnavoidablePreArmor)
            .to_f32_for_render();
        let after_unavoid = (raw - unavoid_block).max(0.0);

        // 3. Armor / Resist
        let after_defense = match kind {
            DamageKind::Physical => {
                let armor = self.final_armor(Fixed32::from_raw((base_armor * 1024.0) as i32), e)
                    .to_f32_for_render();
                after_unavoid * armor_to_mult(armor)
            }
            DamageKind::Magical => {
                let resist = self.final_magic_resist(
                    Fixed32::from_raw((base_resist * 1024.0) as i32), e)
                    .to_f32_for_render();
                after_unavoid * (1.0 - resist)
            }
            DamageKind::Pure => after_unavoid,
            _ => after_unavoid,
        };

        // 4. 類型 block（post-armor）
        let kind_block = self.buffs.sum_add(
            e,
            match kind {
                DamageKind::Physical => StatKey::PhysicalConstantBlock,
                DamageKind::Magical => StatKey::MagicalConstantBlock,
                _ => StatKey::TotalConstantBlock,
            },
        ).to_f32_for_render();
        let after_kind_block = (after_defense - kind_block).max(0.0);

        // 5. Incoming percentage
        let pct_all = 1.0 + self.buffs.sum_add(e, StatKey::IncomingDamagePercentage)
            .to_f32_for_render();
        let pct_kind = 1.0
            + match kind {
                DamageKind::Physical => self.buffs.sum_add(e, StatKey::IncomingPhysicalDamagePercentage)
                    .to_f32_for_render(),
                _ => 0.0,
            };
        let after_pct = after_kind_block * pct_all * pct_kind;

        // 6. Incoming constant
        let k_const = match kind {
            DamageKind::Physical => self.buffs.sum_add(e, StatKey::IncomingPhysicalDamageConstant)
                .to_f32_for_render(),
            DamageKind::Magical => self.buffs.sum_add(e, StatKey::IncomingSpellDamageConstant)
                .to_f32_for_render(),
            _ => 0.0,
        };
        (after_pct + k_const).max(0.0)
    }
}

/// Helper: clamp a Fixed32 to [min, max] (no f32 detour).
#[inline]
fn clamp_fx(v: Fixed32, lo: Fixed32, hi: Fixed32) -> Fixed32 {
    if v < lo { lo } else if v > hi { hi } else { v }
}

/// Dota armor → damage multiplier。
/// armor > 0 → 減傷；armor < 0 → 增傷；armor = 0 → 1.0。
/// 公式：`1 - (0.06 * armor) / (1 + 0.06 * |armor|)`
pub fn armor_to_mult(armor: f32) -> f32 {
    let abs = armor.abs();
    let k = 0.06 * abs;
    if armor >= 0.0 {
        1.0 - (0.06 * armor) / (1.0 + k)
    } else {
        1.0 + (0.06 * abs) / (1.0 + k)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use specs::{Builder, World, WorldExt};

    fn fx_secs(seconds: f32) -> Fixed32 {
        Fixed32::from_raw((seconds * 1024.0) as i32)
    }

    fn fx_huge() -> Fixed32 {
        // Stand-in for "infinity" — large enough that tick decay won't reach 0 in the test window.
        Fixed32::from_i32(1_000_000)
    }

    // Regression: ice tower 寫入 payload key `move_speed_bonus`（StatKey::MoveSpeedBonus），
    // 必須被 `final_move_speed` 當 percentage slow 聚合，否則 creep_tick 算出的有效移速會
    // 是 base 全速 → 前端視覺有減速但後端權威位置瞬移。
    #[test]
    fn move_speed_bonus_applies_as_percentage_slow() {
        let mut world = World::new();
        let e = world.create_entity().build();
        let mut store = BuffStore::new();
        store.add(
            e,
            "slow_test",
            fx_secs(2.0),
            json!({ StatKey::MoveSpeedBonus.as_str(): -0.5 }),
        );
        let stats = UnitStats::from_refs(&store, false);
        let effective = stats.final_move_speed(Fixed32::from_i32(100), e).to_f32_for_render();
        assert!(
            (effective - 50.0).abs() < 1.0,
            "expected ~50.0, got {}",
            effective
        );
    }

    // Dota 順序：equipment bonus（boots 等）跟 base 一起被 percentage 縮放。
    // base=300、boots +90、slow -50% → (300+90)*0.5 = 195。
    #[test]
    fn move_speed_equipment_bonus_scales_with_percentage() {
        let mut world = World::new();
        let e = world.create_entity().build();
        let mut store = BuffStore::new();
        store.add(
            e,
            "boots",
            fx_huge(),
            json!({ StatKey::MoveSpeedBonusEquipment.as_str(): 90.0 }),
        );
        store.add(
            e,
            "slow_ice",
            fx_secs(2.0),
            json!({ StatKey::MoveSpeedBonus.as_str(): -0.5 }),
        );
        let stats = UnitStats::from_refs(&store, false);
        let effective = stats.final_move_speed(Fixed32::from_i32(300), e).to_f32_for_render();
        assert!(
            (effective - 195.0).abs() < 1.0,
            "expected ~195.0, got {}",
            effective
        );
    }

    // Buff flat post-pct：不被 percentage 縮放，純加在最末端。
    // base=300、boots +90、slow -50%、buff +60 → (300+90)*0.5 + 60 = 255。
    #[test]
    fn move_speed_buff_bonus_is_flat_post_percentage() {
        let mut world = World::new();
        let e = world.create_entity().build();
        let mut store = BuffStore::new();
        store.add(
            e,
            "boots",
            fx_huge(),
            json!({ StatKey::MoveSpeedBonusEquipment.as_str(): 90.0 }),
        );
        store.add(
            e,
            "slow_ice",
            fx_secs(2.0),
            json!({ StatKey::MoveSpeedBonus.as_str(): -0.5 }),
        );
        store.add(
            e,
            "haste_buff",
            fx_secs(5.0),
            json!({ StatKey::MoveSpeedBonusBuff.as_str(): 60.0 }),
        );
        let stats = UnitStats::from_refs(&store, false);
        let effective = stats.final_move_speed(Fixed32::from_i32(300), e).to_f32_for_render();
        assert!(
            (effective - 255.0).abs() < 1.0,
            "expected ~255.0, got {}",
            effective
        );
    }
}

