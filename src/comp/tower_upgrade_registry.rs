//! Server-side 48 個 tower upgrade 配表，存為 ECS resource。
//! 在 state/core.rs 初始化時 insert。

use std::collections::HashMap;
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
        self.defs.insert((def.tower_kind.clone(), def.path, def.level), def);
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
            effects: vec![UpgradeEffect::StatMod { key: "range_bonus".into(), value: 50.0, op: StatOp::Add }],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 0, level: 2,
            name: "Enhanced Eyesight".into(),
            description: "射程 →450, damage 10→15".into(),
            cost: 100,
            effects: vec![
                UpgradeEffect::StatMod { key: "range_bonus".into(), value: 50.0, op: StatOp::Add },
                UpgradeEffect::StatMod { key: "damage_bonus".into(), value: 0.5, op: StatOp::Add },
            ],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 0, level: 3,
            name: "Razor Sharp Shots".into(),
            description: "穿透 +1, damage →20".into(),
            cost: 200,
            effects: vec![
                UpgradeEffect::BehaviorFlag { flag: "sharp_pierce".into() },
                UpgradeEffect::StatMod { key: "damage_bonus".into(), value: 0.5, op: StatOp::Add },
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
            effects: vec![UpgradeEffect::StatMod { key: "attack_speed_multiplier".into(), value: 0.83, op: StatOp::Mul }],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 1, level: 2,
            name: "Very Quick Shots".into(),
            description: "攻速再 +30%".into(),
            cost: 100,
            effects: vec![UpgradeEffect::StatMod { key: "attack_speed_multiplier".into(), value: 0.70, op: StatOp::Mul }],
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
                UpgradeEffect::StatMod { key: "attack_speed_multiplier".into(), value: 0.70, op: StatOp::Mul },
            ],
        });
        // Path 2 — Crit Master
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 2, level: 1,
            name: "Keen Eyes".into(),
            description: "爆率 25→40%, 爆傷 30→40".into(),
            cost: 50,
            effects: vec![
                UpgradeEffect::StatMod { key: "crit_chance".into(), value: 0.40, op: StatOp::Add },
                UpgradeEffect::StatMod { key: "crit_bonus".into(), value: 40.0, op: StatOp::Add },
            ],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 2, level: 2,
            name: "Crossbow".into(),
            description: "爆率 →50%, 爆傷 →60, 射程 +30".into(),
            cost: 100,
            effects: vec![
                UpgradeEffect::StatMod { key: "crit_chance".into(), value: 0.10, op: StatOp::Add },
                UpgradeEffect::StatMod { key: "crit_bonus".into(), value: 20.0, op: StatOp::Add },
                UpgradeEffect::StatMod { key: "range_bonus".into(), value: 30.0, op: StatOp::Add },
            ],
        });
        self.insert(TowerUpgradeDef {
            tower_kind: kind.into(), path: 2, level: 3,
            name: "Sharp Shooter".into(),
            description: "必爆 (100%), base dmg +30%".into(),
            cost: 200,
            effects: vec![
                UpgradeEffect::BehaviorFlag { flag: "always_crit".into() },
                UpgradeEffect::StatMod { key: "damage_bonus".into(), value: 0.3, op: StatOp::Add },
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

    // 下面 3 個將由後續 task 補滿。此 commit 僅驗證 Dart + 架構。
    fn register_bomb(&mut self) { /* TODO Task 5 */ }
    fn register_tack(&mut self) { /* TODO Task 5 */ }
    fn register_ice(&mut self)  { /* TODO Task 5 */ }
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
}
