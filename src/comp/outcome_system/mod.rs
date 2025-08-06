/// 遊戲事件結果系統
/// 
/// 管理遊戲中各種事件結果的處理和分派

pub mod combat_events;
pub mod movement_events;
pub mod creation_events;
pub mod system_events;
pub mod event_dispatcher;

pub use combat_events::CombatEventHandler;
pub use movement_events::MovementEventHandler;
pub use creation_events::CreationEventHandler;
pub use system_events::SystemEventHandler;
pub use event_dispatcher::EventDispatcher;

// Re-export common types
pub use crate::comp::outcome::*;