//! Load base_content.dll via abi_stable, extract unit ids + tower_metadata
//! and ability definitions.

use crate::lib::model::{AbilityEntry, TowerStats, UnitKind};
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
        let tower = def.script.tower_metadata().into_option().map(|tm| TowerStats {
            atk: tm.atk,
            asd_interval: tm.asd_interval,
            range: tm.range,
            bullet_speed: tm.bullet_speed,
            splash_radius: tm.splash_radius,
            hit_radius: tm.hit_radius,
            slow_factor: tm.slow_factor,
            slow_duration: tm.slow_duration,
            cost: tm.cost,
            footprint: tm.footprint,
            hp: tm.hp,
            turn_speed_deg: tm.turn_speed_deg,
            label: tm.label.to_string(),
        });
        let kind = if tower.is_some() { UnitKind::Tower } else { UnitKind::Unknown };
        units.push(DllUnit { id, kind, tower });
    }

    let mut abilities = Vec::new();
    for a in abilities_fn() {
        let json_str = a.def_json.to_string();
        let v: serde_json::Value = serde_json::from_str(&json_str).unwrap_or(serde_json::Value::Null);
        let id = v.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
        abilities.push(AbilityEntry { id, def_json: v });
    }

    Ok(DllData { units, abilities })
}
