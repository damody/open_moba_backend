//! `AbilityRegistry` — 存所有透過 DLL 腳本註冊的 `AbilityDef` metadata。
//!
//! 這個 registry 會在 `scripts/base_content` 等 DLL 載入時被 populated。
//! Phase 1 只定義空的 registry 作為 ECS Resource；Phase 2 會在
//! `scripting/registry.rs` 載入 manifest 時呼叫 `register_from_ffi` 填入。

use std::collections::BTreeMap;

use omoba_core::ability_meta::AbilityDef;

/// 技能 metadata 索引（ECS Resource）。
///
/// Client 透過 `list_abilities` / `get_ability_detail` query 取得這裡的資料，
/// 用於 tooltip、技能樹 UI。
#[derive(Default, Debug)]
pub struct AbilityRegistry {
    defs: BTreeMap<String, AbilityDef>,
}

impl AbilityRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 註冊一筆 metadata（Phase 2 會由 scripting loader 從 DLL 的
    /// `AbilityDefFFI::def_json` 反序列化後呼叫）。
    pub fn register(&mut self, def: AbilityDef) {
        self.defs.insert(def.id.clone(), def);
    }

    pub fn get(&self, id: &str) -> Option<&AbilityDef> {
        self.defs.get(id)
    }

    pub fn all(&self) -> impl Iterator<Item = &AbilityDef> {
        self.defs.values()
    }

    pub fn len(&self) -> usize {
        self.defs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }
}
