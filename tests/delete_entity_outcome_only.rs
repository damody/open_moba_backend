//! Phase 1.6b grep guard: ensure no source file in `omb/src/` calls
//! `world.entities().delete()` / `entities.delete()` / `world.delete_entity()`
//! directly. Entity deletion in the omb sim path MUST go through
//! `Outcome::EntityRemoved`, which `process_outcomes` handles uniformly
//! (see `comp/outcome.rs::RemovedEntitiesQueue`).
//!
//! Allowlist: the one site in `comp/game_processor.rs` where
//! `process_outcomes` itself calls `entities().delete()` is the canonical
//! sink for the outcome and is allowed.
//!
//! Why this matters: the snapshot extractor in `omfx/game/src/sim_runner.rs`
//! drains `RemovedEntitiesQueue` to populate `SimWorldSnapshot.removed_entity_ids`.
//! Render-side scene-node cleanup keys off that field. A direct `.delete(e)`
//! that skips the queue would silently leak omfx render state.

use std::fs;
use std::path::{Path, PathBuf};

const FORBIDDEN_PATTERNS: &[&str] = &[
    "entities().delete(",
    ".delete_entity(",
];

/// File paths whose direct `entities().delete()` call is the canonical
/// sink (process_outcomes' Outcome::EntityRemoved arm). Allowlisted by
/// suffix-match relative to crate root.
const ALLOWLIST_SUFFIXES: &[&str] = &[
    // process_outcomes itself: the single sink that the outcome routes to
    "src/comp/game_processor.rs",
];

#[test]
fn no_raw_entity_delete_outside_outcome_sink() {
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let src_dir = crate_root.join("src");
    let mut violations: Vec<String> = Vec::new();

    visit_rs_files(&src_dir, &mut |path| {
        let rel: String = path
            .strip_prefix(&crate_root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");

        if ALLOWLIST_SUFFIXES.iter().any(|suf| rel.ends_with(*suf)) {
            return;
        }

        let content = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => return,
        };

        for (lineno_zero, line) in content.lines().enumerate() {
            let stripped = line.trim_start();
            if stripped.starts_with("//") || stripped.starts_with("///") {
                continue;
            }
            if stripped.starts_with("/*") || stripped.starts_with("*") {
                continue;
            }
            for pat in FORBIDDEN_PATTERNS {
                if line.contains(pat) {
                    violations.push(format!("{}:{} — `{}` (use Outcome::EntityRemoved instead)", rel, lineno_zero + 1, pat));
                }
            }
        }
    });

    assert!(
        violations.is_empty(),
        "raw entity-delete sites found outside process_outcomes sink:\n{}",
        violations.join("\n"),
    );
}

fn visit_rs_files(dir: &Path, cb: &mut dyn FnMut(&Path)) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            visit_rs_files(&path, cb);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            cb(&path);
        }
    }
}
