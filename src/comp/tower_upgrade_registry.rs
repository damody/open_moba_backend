//! Server-side 48 個 tower upgrade 配表，存為 ECS resource。
//! 在 state/core.rs 初始化時 insert。
//!
//! 數值來源：`omb/Story/templates.json` 的 `towers[].upgrades`，由
//! `omoba-template-ids/build.rs` 編譯期讀取生 `TOWER_<NAME>_UPGRADES` const +
//! `tower_upgrades(id)` lookup。本檔案只負責把 const POD 轉成 runtime
//! `TowerUpgradeDef`（含 String / Vec）並塞入 HashMap 供查詢。

use std::collections::HashMap;
use omoba_core::tower_meta::{TowerUpgradeDef, UpgradeEffect, StatOp};
use omoba_template_ids::{
    tower_upgrades, StatOpC, UpgradeDefConst, UpgradeEffectConst, UpgradeEffectKindC,
    TOWER_BOMB, TOWER_DART, TOWER_ICE, TOWER_TACK,
};

pub struct TowerUpgradeRegistry {
    /// key = (tower_kind, path, level)
    defs: HashMap<(String, u8, u8), TowerUpgradeDef>,
}

impl TowerUpgradeRegistry {
    pub fn new() -> Self {
        let mut defs = HashMap::new();
        for &tid in &[TOWER_DART, TOWER_TACK, TOWER_BOMB, TOWER_ICE] {
            let kind = tid.as_str();
            let Some(paths) = tower_upgrades(tid) else { continue };
            for (path_idx, path) in paths.iter().enumerate() {
                for (lvl_idx, c) in path.iter().enumerate() {
                    let lvl = (lvl_idx + 1) as u8;
                    let def = TowerUpgradeDef {
                        tower_kind: kind.into(),
                        path: path_idx as u8,
                        level: lvl,
                        name: c.name.into(),
                        description: c.description.into(),
                        cost: c.cost,
                        effects: c.effects.iter().map(upgrade_effect_from_const).collect(),
                    };
                    let prev = defs.insert((kind.into(), path_idx as u8, lvl), def);
                    debug_assert!(
                        prev.is_none(),
                        "duplicate upgrade def for {} path {} level {}",
                        kind, path_idx, lvl
                    );
                }
            }
        }
        Self { defs }
    }

    pub fn get(&self, kind: &str, path: u8, level: u8) -> Option<&TowerUpgradeDef> {
        self.defs.get(&(kind.to_string(), path, level))
    }
}

fn upgrade_effect_from_const(c: &UpgradeEffectConst) -> UpgradeEffect {
    match c.kind {
        UpgradeEffectKindC::StatMod => UpgradeEffect::StatMod {
            key: c.key.into(),
            // TODO Phase 1[cd]: drop conversion when UpgradeEffect::StatMod.value migrates to Fixed32
            value: c.value.to_f32_for_render(),
            op: match c.op {
                StatOpC::Add => StatOp::Add,
                StatOpC::Mul => StatOp::Mul,
            },
        },
        UpgradeEffectKindC::BehaviorFlag => UpgradeEffect::BehaviorFlag {
            flag: c.key.into(),
        },
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
                assert!(
                    reg.get(TOWER_DART.as_str(), path, level).is_some(),
                    "dart path {} level {}",
                    path,
                    level
                );
            }
        }
    }

    #[test]
    fn all_four_towers_have_12_upgrades_each() {
        let reg = TowerUpgradeRegistry::new();
        for kind in &[
            TOWER_DART.as_str(),
            TOWER_BOMB.as_str(),
            TOWER_TACK.as_str(),
            TOWER_ICE.as_str(),
        ] {
            for path in 0..3 {
                for level in 1..=4 {
                    assert!(
                        reg.get(kind, path, level).is_some(),
                        "{} path {} level {}",
                        kind,
                        path,
                        level
                    );
                }
            }
        }
    }

    #[test]
    fn costs_match_formula() {
        use omoba_core::tower_meta::upgrade_cost;
        use omoba_template_ids::{
            TOWER_BOMB_STATS, TOWER_DART_STATS, TOWER_ICE_STATS, TOWER_TACK_STATS,
        };
        let reg = TowerUpgradeRegistry::new();
        let bases = [
            (TOWER_DART.as_str(), TOWER_DART_STATS.cost),
            (TOWER_BOMB.as_str(), TOWER_BOMB_STATS.cost),
            (TOWER_TACK.as_str(), TOWER_TACK_STATS.cost),
            (TOWER_ICE.as_str(), TOWER_ICE_STATS.cost),
        ];
        for (kind, base) in bases {
            for path in 0..3u8 {
                for level in 1..=4 {
                    let def = reg.get(kind, path, level).unwrap();
                    assert_eq!(
                        def.cost,
                        upgrade_cost(base, level),
                        "{} path {} L{}",
                        kind,
                        path,
                        level
                    );
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
