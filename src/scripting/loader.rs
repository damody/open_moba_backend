//! Scan a directory for script DLLs and load each as a `Manifest_Ref`.

use abi_stable::library::{LibraryError, RootModule};
use omb_script_abi::manifest::Manifest_Ref;
use std::path::{Path, PathBuf};

use super::registry::ScriptRegistry;

#[cfg(target_os = "windows")]
const DLL_EXT: &str = "dll";
#[cfg(target_os = "linux")]
const DLL_EXT: &str = "so";
#[cfg(target_os = "macos")]
const DLL_EXT: &str = "dylib";

/// Load all script DLLs under `dir` (non-recursive).
/// Errors on individual DLLs are logged and skipped; the registry always
/// returns (potentially empty).
pub fn load_scripts_dir(dir: &Path) -> ScriptRegistry {
    let mut reg = ScriptRegistry::new();

    if !dir.is_dir() {
        log::warn!(
            "[scripting] scripts directory {:?} does not exist — running with zero scripts",
            dir
        );
        return reg;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            log::error!("[scripting] read_dir({:?}) failed: {}", dir, e);
            return reg;
        }
    };

    let mut candidates: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let p = entry.path();
        if p.extension().and_then(|s| s.to_str()) == Some(DLL_EXT) {
            candidates.push(p);
        }
    }
    candidates.sort();

    for path in candidates {
        match load_one(&path) {
            Ok(manifest) => {
                log::info!("[scripting] loaded {:?}", path.file_name().unwrap_or_default());
                reg.insert_manifest(manifest);
            }
            Err(e) => {
                log::error!("[scripting] load {:?} failed: {}", path, e);
            }
        }
    }

    log::info!(
        "[scripting] registry ready — {} unit scripts: {:?}",
        reg.len(),
        reg.keys().collect::<Vec<_>>()
    );
    reg
}

fn load_one(path: &Path) -> Result<Manifest_Ref, LibraryError> {
    Manifest_Ref::load_from_file(path)
}
