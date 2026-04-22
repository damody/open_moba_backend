//! Host-side integration for native `UnitScript` DLLs.
//!
//! - [`loader`]: load manifest DLLs from a directory.
//! - [`registry`]: `unit_id → UnitScript_TO` dispatch table.
//! - [`world_adapter`]: implements `omb_script_abi::GameWorld` over `&mut specs::World`.
//! - [`event`]: `ScriptEvent` enum + `ScriptEventQueue` resource.
//! - [`dispatch`]: drain the event queue and fire hooks.
//! - [`tag`]: `ScriptUnitTag` component — marks which entities have a script.

pub mod event;
pub mod registry;
pub mod loader;
pub mod tag;
pub mod world_adapter;
pub mod dispatch;

pub use event::{ScriptEvent, ScriptEventQueue};
pub use registry::ScriptRegistry;
pub use tag::ScriptUnitTag;
pub use dispatch::run_script_dispatch;
