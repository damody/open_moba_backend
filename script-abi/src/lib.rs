//! omb-script-abi — stable ABI contract between omb host and script DLLs.
//!
//! This crate is the **only** thing both the host and all script cdylibs
//! depend on. It must use ONLY `abi_stable` types; no specs, no omb main
//! crate, nothing that would pull engine internals across the FFI boundary.

pub mod ability;
pub mod manifest;
pub mod script;
pub mod types;
pub mod world;

pub mod prelude {
    pub use crate::ability::{AbilityDefFFI, AbilityScript, AbilityScript_TO};
    pub use crate::script::{UnitScript, UnitScript_TO};
    pub use crate::types::*;
    pub use crate::world::{GameWorld, GameWorldDyn, GameWorld_TO};
    pub use abi_stable::{
        rstr,
        sabi_trait::prelude::*,
        std_types::{RBox, RNone, ROption, RSome, RStr, RString, RVec},
    };

    // 讓腳本可以方便建 ProjectileSpec 不用自己 import
    pub use crate::types::{PathSpec, ProjectileSpec};
}
