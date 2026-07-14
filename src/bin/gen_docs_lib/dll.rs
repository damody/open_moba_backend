//! 透過abi_stable載入base_content.dll，提取單元id + tower_metadata
//! 和能力定義。

use crate::lib::model::{
    AbilityEntry, TowerActiveAbilityInfo, TowerStats, TowerUpgradeInfo, UnitKind,
};
use abi_stable::library::RootModule;
use anyhow::{Context, Result};
use omb_script_abi::manifest::Manifest_Ref;
use std::path::Path;

pub struct DllData {
    pub units: Vec<DllUnit>,
    pub abilities: Vec<AbilityEntry>,
}

pub struct DllUnit {
    pub id: String,
    pub kind: UnitKind,
    pub tower: Option<TowerStats>,
}

/// 為什麼不像 `omb/src/scripting/registry.rs` 那樣保留 `Manifest_Ref`：
/// 我們把所有需要的資料（`TowerStats`、`serde_json::Value`、`String`）
/// eagerly 拷貝成 owned 型別，`DllData` 內沒有任何 vtable 或 DLL 記憶體
/// 指標。加上 `declare_root_module_statics!` 用 static OnceCell 讓 DLL
/// 在 process lifetime 內保持 mapped，所以丟掉 `Manifest_Ref` 無害。
/// 若未來加入 `UnitScript_TO` / `AbilityScript_TO` 欄位到 `DllUnit`，
/// 必須改成保留 `Manifest_Ref` 才安全。
pub fn load(dll_path: &Path) -> Result<DllData> {
    let m = Manifest_Ref::load_from_file(dll_path)
        .with_context(|| format!("loading manifest from {}", dll_path.display()))?;
    let units_fn = m.units();
    let abilities_fn = m.abilities();

    let mut units = Vec::new();
    for def in units_fn() {
        let id = def.unit_id.to_string();
        let upgrades = tower_upgrades(&id);
        // gen_docs_lib::model::TowerStats 是一個報告結構（f32 用於 HTML
        // 展示）;在此邊界處從 ABI Fix64 轉換。
        // 注意：僅渲染 HTML 報告結構； gen-docs 接收器有意設定 f32 邊界。
        let tower = def
            .script
            .tower_metadata()
            .into_option()
            .map(|tm| TowerStats {
                atk: tm.atk.to_f32_for_render(),
                asd_interval: tm.asd_interval.to_f32_for_render(),
                range: tm.range.to_f32_for_render(),
                bullet_speed: tm.bullet_speed.to_f32_for_render(),
                splash_radius: tm.splash_radius.to_f32_for_render(),
                hit_radius: tm.hit_radius.to_f32_for_render(),
                slow_factor: tm.slow_factor.to_f32_for_render(),
                slow_duration: tm.slow_duration.to_f32_for_render(),
                cost: tm.cost,
                footprint: tm.footprint.to_f32_for_render(),
                placement_radius: tm.placement_radius.to_f32_for_render(),
                hp: tm.hp.to_f32_for_render(),
                turn_speed_deg: tm.turn_speed_deg.to_f32_for_render(),
                label: tm.label.to_string(),
                render_mode: tm.render.render_mode.to_string(),
                base_image: tm.render.base.to_string(),
                barrel_image: tm.render.barrel.to_string(),
                render_visual_size: tm.render.visual_size.to_f32_for_render(),
                barrel_frames: tm
                    .render
                    .barrel_frames
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
                body_frames: tm
                    .render
                    .body_frames
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
                rotation_mode: tm.render.rotation_mode.to_string(),
                barrel_layout: tm.render.barrel_layout.to_string(),
                barrel_variants: tm
                    .render
                    .barrel_variants
                    .iter()
                    .map(|v| format!("{}:{}", v.count, v.image))
                    .collect(),
                recoil_mode: tm.render.recoil.mode.to_string(),
                recoil_distance: tm.render.recoil.distance.to_f32_for_render(),
                recoil_scale: tm.render.recoil.scale.to_f32_for_render(),
                attack_windup: tm.attack_timing.windup,
                attack_backswing: tm.attack_timing.backswing,
                upgrades,
            });
        let kind = if tower.is_some() {
            UnitKind::Tower
        } else {
            UnitKind::Unknown
        };
        units.push(DllUnit { id, kind, tower });
    }

    let mut abilities = Vec::new();
    for a in abilities_fn() {
        let json_str = a.def_json.to_string();
        let v: serde_json::Value =
            serde_json::from_str(&json_str).unwrap_or(serde_json::Value::Null);
        let id = v
            .get("id")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        abilities.push(AbilityEntry { id, def_json: v });
    }

    Ok(DllData { units, abilities })
}

fn tower_upgrades(unit_id: &str) -> Vec<TowerUpgradeInfo> {
    let Some(tower_id) = omoba_template_ids::tower_by_name(unit_id) else {
        return Vec::new();
    };
    let Some(paths) = omoba_template_ids::active_tower_upgrades(tower_id) else {
        return Vec::new();
    };

    paths
        .iter()
        .enumerate()
        .flat_map(|(path, levels)| {
            levels.iter().enumerate().map(move |(level, upgrade)| {
                let active_ability = upgrade
                    .active_ability
                    .map(|ability| TowerActiveAbilityInfo {
                        ability_id: ability.ability_id.to_string(),
                        display_name: ability.display_name.to_string(),
                        cooldown: ability.cooldown.to_f32_for_render(),
                    });
                TowerUpgradeInfo {
                    path,
                    level: level + 1,
                    name: upgrade.name.to_string(),
                    active_ability,
                }
            })
        })
        .collect()
}
