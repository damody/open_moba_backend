//! 本機「UnitScript」DLL 的主機端整合。
//!
//! - [`loader`]：從目錄載入清單 DLL。
//! - [`registry`]：`unit_id → UnitScript_TO` 調度表。
//! - [`world_adapter`]：在`&mut specs::World`上實作`omb_script_abi::GameWorld`。
//! - [`event`]：`ScriptEvent` 枚舉 + `ScriptEventQueue` 資源。
//! - [`dispatch`]：排空事件佇列並觸發鉤子。
//! - [`tag`]：`ScriptUnitTag` 元件 — 標記哪些實體有腳本。

pub mod loader;

pub use omoba_core::runtime::scripting::{
    dispatch, event, registry, run_script_dispatch, tag, world_adapter, ScriptEvent,
    ScriptEventQueue, ScriptRegistry, ScriptUnitTag, SkillTarget, WorldAdapter,
};
