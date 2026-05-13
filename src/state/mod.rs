/// 遊戲狀態管理模塊
///
/// 負責管理整個遊戲的核心狀態，包括 ECS 世界、資源管理、時間循環等
pub mod core;
#[cfg(feature = "runtime-lua-content")]
pub mod dev_lua_hot_reload;
#[cfg(any(feature = "grpc", feature = "kcp"))]
pub mod query;
pub mod resource_management;
pub mod time_management;

pub use core::State;
pub use omoba_core::runtime::{StateInitializer, SystemDispatcher};
pub use resource_management::ResourceManager;
pub use time_management::TimeManager;
