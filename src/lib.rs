#![allow(
    ambiguous_glob_reexports,
    dead_code,
    non_snake_case,
    redundant_semicolons,
    unexpected_cfgs,
    unused_assignments,
    unused_attributes,
    unused_imports,
    unused_macros,
    unused_mut,
    unused_parens,
    unused_variables
)]

/// 開啟MOBA後端庫
///
/// Open MOBA 遊戲後端的主庫箱
pub mod ability_runtime;
pub mod aoi;
pub mod comp;
pub mod config;
pub mod item;
pub mod json_preprocessor;
#[cfg(feature = "kcp")]
pub mod lockstep;
#[cfg(feature = "mqtt")]
pub mod mqtt;
pub mod msg;
pub mod runtime_events;
pub mod scripting;
pub mod state;
pub mod tick;
pub mod transport;
pub mod ue4;
pub mod util;
pub mod vision;

// 重新匯出常用類型
pub use crate::comp::*;
pub use crate::msg::MqttMsg;
pub use crate::transport::{InboundMsg, OutboundMsg};
pub use crate::vision::*;
