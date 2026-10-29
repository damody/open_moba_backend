//! M3' ② dev hot reload of script DLLs (docs/plans/2026-10-28-history-nobu-full-port/
//! m3-rust-codegen-design.md §4). Off unless the authority starts with `OMB_SCRIPT_HOT_DIR`;
//! formal matches never set it.
//!
//! `scripts/hero_hot_reload.lua` copies a rebuilt hero DLL into that directory under a new
//! name and writes `request-<generation>.json` ({module, file, sha256, generation}). Every
//! `POLL_TICKS` ticks the authority, before stepping, swaps each new request
//! (`omoba_core::scripting::loader::apply_hot_swap`), announces it in this tick's team frames
//! (`ScriptSwapsThisTick`) so every replica swaps before the same tick, records it in the
//! accepted-input log, and answers `request-<generation>.result.json` ({ok, tick | error}).
use std::collections::BTreeSet;
use std::path::PathBuf;

use omoba_core::game_proto::ScriptModuleSwap;

/// Ticks between two looks at the request directory (0.5 s at 60 Hz).
pub const POLL_TICKS: u64 = 30;

pub struct ScriptHotReload {
    dir: PathBuf,
    answered: BTreeSet<u64>,
}

/// The ECS resource holding the manager (None when hot reload is off).
#[derive(Default)]
pub struct ScriptHotReloadState(pub Option<ScriptHotReload>);

impl ScriptHotReload {
    pub fn from_env() -> Option<Self> {
        let dir = std::env::var_os("OMB_SCRIPT_HOT_DIR").filter(|dir| !dir.is_empty())?;
        log::warn!("[script-hot-reload] enabled (dev only): {:?}", dir);
        Some(Self { dir: PathBuf::from(dir), answered: BTreeSet::new() })
    }

    /// New requests, oldest generation first.
    pub fn pending(&mut self) -> Vec<(PathBuf, Result<ScriptModuleSwap, String>)> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return Vec::new() };
        let mut found = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(generation) = name.strip_prefix("request-").and_then(|rest| rest.strip_suffix(".json")) else { continue };
            let Ok(generation) = generation.parse::<u64>() else { continue };
            if self.answered.contains(&generation) || self.result_path(generation).exists() {
                continue;
            }
            found.push((generation, entry.path()));
        }
        found.sort();
        found
            .into_iter()
            .map(|(generation, path)| {
                self.answered.insert(generation);
                (self.result_path(generation), parse_request(&path, generation))
            })
            .collect()
    }

    fn result_path(&self, generation: u64) -> PathBuf {
        self.dir.join(format!("request-{generation}.result.json"))
    }
}

fn parse_request(path: &std::path::Path, generation: u64) -> Result<ScriptModuleSwap, String> {
    let text = std::fs::read_to_string(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|error| format!("parse {}: {error}", path.display()))?;
    let field = |name: &str| value[name].as_str().map(str::to_owned).ok_or_else(|| format!("{}: no {name}", path.display()));
    Ok(ScriptModuleSwap { module: field("module")?, file: field("file")?, sha256: field("sha256")?, generation })
}

/// Writes the answer hero_hot_reload.lua waits for.
pub fn answer(path: &std::path::Path, result: &Result<u64, String>) {
    let value = match result {
        Ok(tick) => serde_json::json!({"ok": true, "tick": tick}),
        Err(error) => serde_json::json!({"ok": false, "error": error}),
    };
    let partial = path.with_extension("json.partial");
    if std::fs::write(&partial, value.to_string()).and_then(|_| std::fs::rename(&partial, path)).is_err() {
        log::warn!("[script-hot-reload] cannot write {}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_are_taken_once_in_generation_order_and_answered() {
        let dir = std::env::temp_dir().join(format!("omoba-script-hot-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for generation in [2u64, 1] {
            std::fs::write(
                dir.join(format!("request-{generation}.json")),
                serde_json::json!({"module": "hero_a02", "file": format!("hero_a02.{generation}.dll"), "sha256": "ab"}).to_string(),
            )
            .unwrap();
        }
        std::fs::write(dir.join("request-3.json"), "{").unwrap();
        let mut reload = ScriptHotReload { dir: dir.clone(), answered: BTreeSet::new() };
        let pending = reload.pending();
        let generations: Vec<_> = pending.iter().map(|(_, request)| request.as_ref().map(|swap| swap.generation).ok()).collect();
        assert_eq!(generations, vec![Some(1), Some(2), None], "oldest first; a malformed request is an error");
        assert!(reload.pending().is_empty(), "each request is taken once");
        answer(&pending[0].0, &Ok(90));
        let answered: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("request-1.result.json")).unwrap()).unwrap();
        assert_eq!((answered["ok"].as_bool(), answered["tick"].as_u64()), (Some(true), Some(90)));
        let mut restarted = ScriptHotReload { dir: dir.clone(), answered: BTreeSet::new() };
        assert_eq!(restarted.pending().len(), 2, "an answered request is not taken again");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
