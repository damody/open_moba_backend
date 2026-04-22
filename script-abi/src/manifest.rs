//! Manifest — the root module each script DLL exports.
//!
//! Host calls `Manifest_Ref::load_from_file(dll)` then `(manifest.units())()`
//! to collect every `UnitDef` the DLL provides.

use abi_stable::{
    StableAbi,
    library::RootModule,
    package_version_strings,
    sabi_types::VersionStrings,
    std_types::{RBox, RString, RVec},
};
use crate::script::UnitScript_TO;

#[repr(C)]
#[derive(StableAbi)]
pub struct UnitDef {
    pub unit_id: RString,
    pub script: UnitScript_TO<'static, RBox<()>>,
}

#[repr(C)]
#[derive(StableAbi)]
#[sabi(kind(Prefix(prefix_ref = Manifest_Ref, prefix_fields = Manifest_Prefix)))]
#[sabi(missing_field(panic))]
pub struct Manifest {
    /// Returns every unit this DLL provides.
    #[sabi(last_prefix_field)]
    pub units: extern "C" fn() -> RVec<UnitDef>,
}

impl RootModule for Manifest_Ref {
    abi_stable::declare_root_module_statics! { Manifest_Ref }
    const BASE_NAME: &'static str = "omb_script";
    const NAME: &'static str = "omb_script";
    const VERSION_STRINGS: VersionStrings = package_version_strings!();
}
