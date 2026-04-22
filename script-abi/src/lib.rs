//! omb-script-abi — stable ABI contract between omb host and script DLLs.
//!
//! This crate is the **only** thing both the host and all script cdylibs
//! depend on. It must use ONLY `abi_stable` types; no specs, no omb main
//! crate, nothing that would pull engine internals across the FFI boundary.

pub mod types;
pub mod world;
pub mod script;
pub mod manifest;

pub mod prelude {
    pub use crate::types::*;
    pub use crate::world::{GameWorld, GameWorldDyn, GameWorld_TO};
    pub use crate::script::{UnitScript, UnitScript_TO};
    pub use abi_stable::{
        std_types::{RStr, RString, RVec, ROption, RSome, RNone, RBox},
        sabi_trait::prelude::*,
        rstr,
    };
}
