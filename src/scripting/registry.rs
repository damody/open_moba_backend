//! Maps `unit_id → UnitScript_TO` for dispatch.
//! Also retains the loaded `RootModule` refs so the underlying DLLs stay
//! alive for the entire process lifetime (H1 — no hot reload).

use abi_stable::std_types::RBox;
use hashbrown::HashMap;
use omb_script_abi::{manifest::Manifest_Ref, script::UnitScript_TO};

pub struct ScriptRegistry {
    scripts: HashMap<String, UnitScript_TO<'static, RBox<()>>>,
    /// 保持腳本 DLL `units()` 回傳順序，前端 UI 按鈕依此排序
    order: Vec<String>,
    /// Keep manifest refs alive → DLLs stay mapped.
    _manifests: Vec<Manifest_Ref>,
}

impl ScriptRegistry {
    pub fn new() -> Self {
        Self {
            scripts: HashMap::new(),
            order: Vec::new(),
            _manifests: Vec::new(),
        }
    }

    pub fn insert_manifest(&mut self, manifest: Manifest_Ref) {
        let units = (manifest.units())();
        for def in units {
            let id: String = def.unit_id.into();
            if self.scripts.contains_key(&id) {
                log::warn!("[scripting] duplicate unit_id `{}` — overriding", id);
            } else {
                self.order.push(id.clone());
            }
            self.scripts.insert(id, def.script);
        }
        self._manifests.push(manifest);
    }

    pub fn get(&self, unit_id: &str) -> Option<&UnitScript_TO<'static, RBox<()>>> {
        self.scripts.get(unit_id)
    }

    pub fn len(&self) -> usize {
        self.scripts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.scripts.is_empty()
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.scripts.keys().map(|s| s.as_str())
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &UnitScript_TO<'static, RBox<()>>)> {
        self.scripts.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// 依 DLL `units()` 註冊順序 iterate
    pub fn iter_ordered(&self) -> impl Iterator<Item = (&str, &UnitScript_TO<'static, RBox<()>>)> {
        self.order.iter().filter_map(|id| {
            self.scripts.get(id).map(|s| (id.as_str(), s))
        })
    }
}

impl Default for ScriptRegistry {
    fn default() -> Self {
        Self::new()
    }
}
