//! Server-side 48 個 tower upgrade 配表，存為 ECS resource。
//! 在 state/core.rs 初始化時 insert。

use std::collections::HashMap;
use omb_script_abi::stat_keys::StatKey;
use omoba_core::tower_meta::{TowerUpgradeDef, UpgradeEffect, StatOp};

pub struct TowerUpgradeRegistry {
    /// key = (tower_kind, path, level)
    defs: HashMap<(String, u8, u8), TowerUpgradeDef>,
}

impl TowerUpgradeRegistry {
    pub fn new() -> Self {
        let mut reg = Self { defs: HashMap::new() };
        reg.register_dart();
        reg.register_bomb();
        reg.register_tack();
        reg.register_ice();
        reg
    }

    pub fn get(&self, kind: &str, path: u8, level: u8) -> Option<&TowerUpgradeDef> {
        self.defs.get(&(kind.to_string(), path, level))
    }

    fn insert(&mut self, def: TowerUpgradeDef) {
        let key = (def.tower_kind.clone(), def.path, def.level);
        let prev = self.defs.insert(key.clone(), def);
        debug_assert!(prev.is_none(), "duplicate upgrade def for {:?}", key);
    }

    // Dart Monkey (base 200): Path 0 Sharp, Path 1 Quick, Path 2 Crit
    fn register_dart(&mut self) {
        let kind = "tower_dart";
        // Path 0 — Sharp Shots
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 0, level: 1,
            name: "Long Range Darts".into(),
            description: "射程 350→400".into(),
            cost: 50,
            effects: vec![UpgradeEffect::StatMod { key: StatKey::AttackRangeBonus.as_str().into(), value: 50.0, op: StatOp::Add }],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 0, level: 2,
            name: "Enhanced Eyesight".into(),
            description: "射程 →450, damage 10→15".into(),
            cost: 100,
            effects: vec![
                UpgradeEffect::StatMod { key: StatKey::AttackRangeBonus.as_str().into(), value: 50.0, op: StatOp::Add },
                UpgradeEffect::StatMod { key: StatKey::BaseDamageOutgoingPercentage.as_str().into(), value: 0.5, op: StatOp::Add },
            ],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 0, level: 3,
            name: "Razor Sharp Shots".into(),
            description: "穿透 +1, damage →20".into(),
            cost: 200,
            effects: vec![
                UpgradeEffect::BehaviorFlag { flag: "sharp_pierce".into() },
                UpgradeEffect::StatMod { key: StatKey::BaseDamageOutgoingPercentage.as_str().into(), value: 0.5, op: StatOp::Add },
            ],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 0, level: 4,
            name: "Spike-o-pult".into(),
            description: "改投巨釘：splash 100, damage 40, 彈速減半".into(),
            cost: 500,
            effects: vec![UpgradeEffect::BehaviorFlag { flag: "spike_o_pult".into() }],
        });
        // Path 1 — Quick Shots
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 1, level: 1,
            name: "Quick Shots".into(),
            description: "攻速 +20%".into(),
            cost: 50,
            effects: vec![UpgradeEffect::StatMod { key: StatKey::AttackSpeedMultiplier.as_str().into(), value: 0.83, op: StatOp::Mul }],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 1, level: 2,
            name: "Very Quick Shots".into(),
            description: "攻速再 +30%".into(),
            cost: 100,
            effects: vec![UpgradeEffect::StatMod { key: StatKey::AttackSpeedMultiplier.as_str().into(), value: 0.70, op: StatOp::Mul }],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 1, level: 3,
            name: "Triple Shot".into(),
            description: "一發變 3 發扇形 ±15°".into(),
            cost: 200,
            effects: vec![UpgradeEffect::BehaviorFlag { flag: "triple_shot".into() }],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 1, level: 4,
            name: "Super Monkey Fan Club".into(),
            description: "5 發扇形 + 彈速×2 + 攻速再 +30%".into(),
            cost: 500,
            effects: vec![
                UpgradeEffect::BehaviorFlag { flag: "fan_club".into() },
                UpgradeEffect::StatMod { key: StatKey::AttackSpeedMultiplier.as_str().into(), value: 0.70, op: StatOp::Mul },
            ],
        });
        // Path 2 — Crit Master
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 2, level: 1,
            name: "Keen Eyes".into(),
            description: "爆率 25→40%, 爆傷 30→40".into(),
            cost: 50,
            effects: vec![
                UpgradeEffect::StatMod { key: StatKey::PreattackCriticalStrike.as_str().into(), value: 0.40, op: StatOp::Add },
                UpgradeEffect::StatMod { key: StatKey::CritBonus.as_str().into(), value: 40.0, op: StatOp::Add },
            ],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 2, level: 2,
            name: "Crossbow".into(),
            description: "爆率 →50%, 爆傷 →60, 射程 +30".into(),
            cost: 100,
            effects: vec![
                UpgradeEffect::StatMod { key: StatKey::PreattackCriticalStrike.as_str().into(), value: 0.10, op: StatOp::Add },
                UpgradeEffect::StatMod { key: StatKey::CritBonus.as_str().into(), value: 20.0, op: StatOp::Add },
                UpgradeEffect::StatMod { key: StatKey::AttackRangeBonus.as_str().into(), value: 30.0, op: StatOp::Add },
            ],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 2, level: 3,
            name: "Sharp Shooter".into(),
            description: "必爆 (100%), base dmg +30%".into(),
            cost: 200,
            effects: vec![
                UpgradeEffect::BehaviorFlag { flag: "always_crit".into() },
                UpgradeEffect::StatMod { key: StatKey::BaseDamageOutgoingPercentage.as_str().into(), value: 0.3, op: StatOp::Add },
            ],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 2, level: 4,
            name: "Ultra-Juggernaut".into(),
            description: "爆擊 100 dmg + splash 60".into(),
            cost: 500,
            effects: vec![UpgradeEffect::BehaviorFlag { flag: "mega_crit".into() }],
        });
    }

    // Bomb Shooter (base 650): Path 0 Bigger Bombs, Path 1 Missile Launcher, Path 2 Cluster Bombs
    fn register_bomb(&mut self) {
        let kind = "tower_bomb";
        // Path 0 — Bigger Bombs
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 0, level: 1,
            name: "Extra Range".into(),
            description: "射程 400→475".into(),
            cost: 162,
            effects: vec![UpgradeEffect::StatMod { key: StatKey::AttackRangeBonus.as_str().into(), value: 75.0, op: StatOp::Add }],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 0, level: 2,
            name: "Bigger Bombs".into(),
            description: "splash 200→250, damage 30→40".into(),
            cost: 325,
            effects: vec![
                UpgradeEffect::StatMod { key: StatKey::SplashBonus.as_str().into(), value: 50.0, op: StatOp::Add },
                UpgradeEffect::StatMod { key: StatKey::BaseDamageOutgoingPercentage.as_str().into(), value: 0.33, op: StatOp::Add },
            ],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 0, level: 3,
            name: "Really Big Bombs".into(),
            description: "splash →300, damage →60".into(),
            cost: 650,
            effects: vec![
                UpgradeEffect::StatMod { key: StatKey::SplashBonus.as_str().into(), value: 50.0, op: StatOp::Add },
                UpgradeEffect::StatMod { key: StatKey::BaseDamageOutgoingPercentage.as_str().into(), value: 0.67, op: StatOp::Add },
            ],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 0, level: 4,
            name: "Bloon Impact".into(),
            description: "splash →400, damage →100, 命中 0.5s 眩暈".into(),
            cost: 1625,
            effects: vec![
                UpgradeEffect::BehaviorFlag { flag: "bomb_stun".into() },
                UpgradeEffect::StatMod { key: StatKey::SplashBonus.as_str().into(), value: 100.0, op: StatOp::Add },
                UpgradeEffect::StatMod { key: StatKey::BaseDamageOutgoingPercentage.as_str().into(), value: 1.33, op: StatOp::Add },
            ],
        });
        // Path 1 — Missile Launcher
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 1, level: 1,
            name: "Faster Reload".into(),
            description: "攻速 +20%".into(),
            cost: 162,
            effects: vec![UpgradeEffect::StatMod { key: StatKey::AttackSpeedMultiplier.as_str().into(), value: 0.83, op: StatOp::Mul }],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 1, level: 2,
            name: "Missile Launcher".into(),
            description: "射程 +150, 彈速 900→1350".into(),
            cost: 325,
            effects: vec![
                UpgradeEffect::StatMod { key: StatKey::AttackRangeBonus.as_str().into(), value: 150.0, op: StatOp::Add },
                UpgradeEffect::BehaviorFlag { flag: "missile".into() },
            ],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 1, level: 3,
            name: "MOAB Mauler".into(),
            description: "damage +30, 彈速再 +50%".into(),
            cost: 650,
            effects: vec![UpgradeEffect::StatMod { key: StatKey::BaseDamageOutgoingPercentage.as_str().into(), value: 1.0, op: StatOp::Add }],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 1, level: 4,
            name: "MOAB Assassin".into(),
            description: "每 15s 超級彈 + 常攻再 +30% 攻速".into(),
            cost: 1625,
            effects: vec![
                UpgradeEffect::BehaviorFlag { flag: "moab_assassin".into() },
                UpgradeEffect::StatMod { key: StatKey::AttackSpeedMultiplier.as_str().into(), value: 0.70, op: StatOp::Mul },
            ],
        });
        // Path 2 — Cluster Bombs
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 2, level: 1,
            name: "Frag Bombs".into(),
            description: "爆炸後 8 方向碎片 15 dmg".into(),
            cost: 162,
            effects: vec![UpgradeEffect::BehaviorFlag { flag: "frag_8".into() }],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 2, level: 2,
            name: "Cluster Bombs".into(),
            description: "碎片 →12, dmg 25".into(),
            cost: 325,
            effects: vec![UpgradeEffect::BehaviorFlag { flag: "frag_12".into() }],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 2, level: 3,
            name: "Recursive Cluster".into(),
            description: "碎片 dmg →45, 再生 4 個小碎片".into(),
            cost: 650,
            effects: vec![UpgradeEffect::BehaviorFlag { flag: "frag_recursive".into() }],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 2, level: 4,
            name: "Bomb Blitz".into(),
            description: "碎片 →16 homing, 主彈 dmg +50".into(),
            cost: 1625,
            effects: vec![
                UpgradeEffect::BehaviorFlag { flag: "frag_homing".into() },
                UpgradeEffect::StatMod { key: StatKey::BaseDamageOutgoingPercentage.as_str().into(), value: 1.67, op: StatOp::Add },
            ],
        });
    }

    // Tack Shooter (base 400): Path 0 Sharp Tacks, Path 1 Ring of Fire, Path 2 More Tacks
    fn register_tack(&mut self) {
        let kind = "tower_tack";
        // Path 0 — Sharp Tacks
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 0, level: 1,
            name: "Faster Shooting".into(),
            description: "攻速 +20%".into(),
            cost: 100,
            effects: vec![UpgradeEffect::StatMod { key: StatKey::AttackSpeedMultiplier.as_str().into(), value: 0.83, op: StatOp::Mul }],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 0, level: 2,
            name: "Long Range Tacks".into(),
            description: "射程 380→460, damage 8→11".into(),
            cost: 200,
            effects: vec![
                UpgradeEffect::StatMod { key: StatKey::AttackRangeBonus.as_str().into(), value: 80.0, op: StatOp::Add },
                UpgradeEffect::StatMod { key: StatKey::BaseDamageOutgoingPercentage.as_str().into(), value: 0.375, op: StatOp::Add },
            ],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 0, level: 3,
            name: "Super Range Tacks".into(),
            description: "射程 →530, damage →14".into(),
            cost: 400,
            effects: vec![
                UpgradeEffect::StatMod { key: StatKey::AttackRangeBonus.as_str().into(), value: 70.0, op: StatOp::Add },
                UpgradeEffect::StatMod { key: StatKey::BaseDamageOutgoingPercentage.as_str().into(), value: 0.375, op: StatOp::Add },
            ],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 0, level: 4,
            name: "Blade Shooter".into(),
            description: "飛刀: dmg 20, hit_radius 110, 穿透 +2".into(),
            cost: 1000,
            effects: vec![
                UpgradeEffect::BehaviorFlag { flag: "blade_shooter".into() },
                UpgradeEffect::StatMod { key: StatKey::BaseDamageOutgoingPercentage.as_str().into(), value: 1.5, op: StatOp::Add },
            ],
        });
        // Path 1 — Ring of Fire
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 1, level: 1,
            name: "Hot Shots".into(),
            description: "命中附 2s 灼燒 5dps".into(),
            cost: 100,
            effects: vec![UpgradeEffect::BehaviorFlag { flag: "burn_tier1".into() }],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 1, level: 2,
            name: "Burny Stuff".into(),
            description: "灼燒 3s × 10dps".into(),
            cost: 200,
            effects: vec![UpgradeEffect::BehaviorFlag { flag: "burn_tier2".into() }],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 1, level: 3,
            name: "Ring of Fire".into(),
            description: "每次開火塔周 200 半徑 20 dmg".into(),
            cost: 400,
            effects: vec![UpgradeEffect::BehaviorFlag { flag: "ring_of_fire".into() }],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 1, level: 4,
            name: "Inferno Ring".into(),
            description: "火圈 dmg →50, 針 dmg +10, 火圈附燃燒".into(),
            cost: 1000,
            effects: vec![
                UpgradeEffect::BehaviorFlag { flag: "inferno_ring".into() },
                UpgradeEffect::StatMod { key: StatKey::BaseDamageOutgoingPercentage.as_str().into(), value: 1.25, op: StatOp::Add },
            ],
        });
        // Path 2 — More Tacks
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 2, level: 1,
            name: "Faster Shooting II".into(),
            description: "攻速 +30%".into(),
            cost: 100,
            effects: vec![UpgradeEffect::StatMod { key: StatKey::AttackSpeedMultiplier.as_str().into(), value: 0.77, op: StatOp::Mul }],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 2, level: 2,
            name: "Even More Tacks".into(),
            description: "針數 8→12".into(),
            cost: 200,
            effects: vec![UpgradeEffect::BehaviorFlag { flag: "needles_12".into() }],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 2, level: 3,
            name: "Tack Sprayer".into(),
            description: "針數 →16, 射程 +50".into(),
            cost: 400,
            effects: vec![
                UpgradeEffect::BehaviorFlag { flag: "needles_16".into() },
                UpgradeEffect::StatMod { key: StatKey::AttackRangeBonus.as_str().into(), value: 50.0, op: StatOp::Add },
            ],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 2, level: 4,
            name: "The Tack Zone".into(),
            description: "針數 →32, 攻速再 +40%".into(),
            cost: 1000,
            effects: vec![
                UpgradeEffect::BehaviorFlag { flag: "needles_32".into() },
                UpgradeEffect::StatMod { key: StatKey::AttackSpeedMultiplier.as_str().into(), value: 0.70, op: StatOp::Mul },
            ],
        });
    }

    // Ice Monkey (base 400): Path 0 Permafrost, Path 1 Arctic Wind, Path 2 Embrittlement
    fn register_ice(&mut self) {
        let kind = "tower_ice";
        // Path 0 — Permafrost
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 0, level: 1,
            name: "Permafrost".into(),
            description: "slow 50%→65%".into(),
            cost: 100,
            effects: vec![UpgradeEffect::StatMod { key: StatKey::SlowFactorOverride.as_str().into(), value: 0.35, op: StatOp::Add }],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 0, level: 2,
            name: "Enhanced Freeze".into(),
            description: "slow 持續 2.0→3.0s".into(),
            cost: 200,
            effects: vec![UpgradeEffect::StatMod { key: StatKey::SlowDurationBonus.as_str().into(), value: 1.0, op: StatOp::Add }],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 0, level: 3,
            name: "Deep Freeze".into(),
            description: "命中附 1.0s 完全凍結".into(),
            cost: 400,
            effects: vec![UpgradeEffect::BehaviorFlag { flag: "deep_freeze".into() }],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 0, level: 4,
            name: "Absolute Zero".into(),
            description: "每 15s 全屏凍結 2s, 常規 slow →80%".into(),
            cost: 1000,
            effects: vec![
                UpgradeEffect::BehaviorFlag { flag: "absolute_zero".into() },
                UpgradeEffect::StatMod { key: StatKey::SlowFactorOverride.as_str().into(), value: -0.15, op: StatOp::Add },
            ],
        });
        // Path 1 — Arctic Wind
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 1, level: 1,
            name: "Larger Range".into(),
            description: "range 180→250, splash 90→120".into(),
            cost: 100,
            effects: vec![
                UpgradeEffect::StatMod { key: StatKey::AttackRangeBonus.as_str().into(), value: 70.0, op: StatOp::Add },
                UpgradeEffect::StatMod { key: StatKey::SplashBonus.as_str().into(), value: 30.0, op: StatOp::Add },
            ],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 1, level: 2,
            name: "Arctic Wind".into(),
            description: "range →300, 塔周光環減速 20%".into(),
            cost: 200,
            effects: vec![
                UpgradeEffect::StatMod { key: StatKey::AttackRangeBonus.as_str().into(), value: 50.0, op: StatOp::Add },
                UpgradeEffect::BehaviorFlag { flag: "arctic_aura_20".into() },
            ],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 1, level: 3,
            name: "Snowstorm".into(),
            description: "光環疊到 35%, 凍敵所有塔攻速 +10%".into(),
            cost: 400,
            effects: vec![UpgradeEffect::BehaviorFlag { flag: "snowstorm".into() }],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 1, level: 4,
            name: "Cryo Cannon".into(),
            description: "range →400, 光環 40%, 每 10s 射巨冰彈".into(),
            cost: 1000,
            effects: vec![
                UpgradeEffect::BehaviorFlag { flag: "cryo_cannon".into() },
                UpgradeEffect::StatMod { key: StatKey::AttackRangeBonus.as_str().into(), value: 100.0, op: StatOp::Add },
            ],
        });
        // Path 2 — Embrittlement
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 2, level: 1,
            name: "Enhanced Freeze".into(),
            description: "本塔減速敵人受物理 +15%".into(),
            cost: 100,
            effects: vec![UpgradeEffect::BehaviorFlag { flag: "embrittle_15".into() }],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 2, level: 2,
            name: "Re-Freeze".into(),
            description: "攻擊刷新 slow 到滿 duration".into(),
            cost: 200,
            effects: vec![UpgradeEffect::BehaviorFlag { flag: "refreeze".into() }],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 2, level: 3,
            name: "Embrittlement".into(),
            description: "減速中敵人受全來源 +25% 傷害".into(),
            cost: 400,
            effects: vec![UpgradeEffect::BehaviorFlag { flag: "embrittle_25".into() }],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 2, level: 4,
            name: "Icicle Impale".into(),
            description: "冰錐穿透 3, base dmg 3→25, splash 150".into(),
            cost: 1000,
            effects: vec![
                UpgradeEffect::BehaviorFlag { flag: "icicle_impale".into() },
                UpgradeEffect::StatMod { key: StatKey::BaseDamageOutgoingPercentage.as_str().into(), value: 7.33, op: StatOp::Add },
            ],
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dart_has_12_upgrades() {
        let reg = TowerUpgradeRegistry::new();
        for path in 0..3 {
            for level in 1..=4 {
                assert!(reg.get("tower_dart", path, level).is_some(),
                    "dart path {} level {}", path, level);
            }
        }
    }

    #[test]
    fn all_four_towers_have_12_upgrades_each() {
        let reg = TowerUpgradeRegistry::new();
        for kind in &["tower_dart", "tower_bomb", "tower_tack", "tower_ice"] {
            for path in 0..3 {
                for level in 1..=4 {
                    assert!(reg.get(kind, path, level).is_some(),
                        "{} path {} level {}", kind, path, level);
                }
            }
        }
    }

    #[test]
    fn costs_match_formula() {
        use omoba_core::tower_meta::upgrade_cost;
        let reg = TowerUpgradeRegistry::new();
        let bases = [("tower_dart", 200), ("tower_bomb", 650), ("tower_tack", 400), ("tower_ice", 400)];
        for (kind, base) in bases {
            for path in 0..3u8 {
                for level in 1..=4 {
                    let def = reg.get(kind, path, level).unwrap();
                    assert_eq!(def.cost, upgrade_cost(base, level),
                        "{} path {} L{}", kind, path, level);
                }
            }
        }
    }

    #[test]
    fn no_duplicate_keys() {
        let reg = TowerUpgradeRegistry::new();
        assert_eq!(reg.defs.len(), 48);
    }
}
