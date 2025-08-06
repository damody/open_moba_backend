/// 遊戲狀態管理模塊
/// 
/// 負責管理整個遊戲的核心狀態，包括 ECS 世界、資源管理、時間循環等

pub mod core;
pub mod initialization;
pub mod time_management;
pub mod resource_management;
pub mod system_dispatcher;

pub use core::State;
pub use initialization::StateInitializer;
pub use time_management::TimeManager;
pub use resource_management::ResourceManager;
pub use system_dispatcher::SystemDispatcher;