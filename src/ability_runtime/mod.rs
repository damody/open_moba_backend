//! Ability 模組 — 主 crate 內的 runtime framework
//!
//! Phase 1：只有 `AbilityRegistry`（metadata 索引）。
//! Phase 2+：processor、effects、types、config 會陸續搬進來。

pub mod registry;

pub use registry::AbilityRegistry;
