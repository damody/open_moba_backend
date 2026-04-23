//! Ability 模組 — 主 crate 內的 runtime framework。
//!
//! - `registry`：`AbilityRegistry` — 從 DLL 收集的技能 metadata（client query 用）
//! - `buff_store`：`BuffStore` — 統一的 buff 儲存與倒數（取代 SlowBuff）

pub mod buff_store;
pub mod registry;
pub mod unit_stats;

pub use buff_store::{BuffEntry, BuffStore};
pub use registry::AbilityRegistry;
pub use unit_stats::{armor_to_mult, UnitStats};
