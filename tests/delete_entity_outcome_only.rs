//! 階段 1.6b grep Guard：確保 `omb/src/` 呼叫中沒有來源文件
//! `world.entities().delete()` / `entities.delete()` / `world.delete_entity()`
//! 直接地。 omb sim 路徑中的實體刪除必須經過
//! `Outcome::EntityRemoved`，由 `process_outcomes` 統一處理
//! （請參閱“comp/outcome.rs::RemovedEntitiesQueue”）。
//!
//! 白名單：「comp/game_processor.rs」中的一個站點
//! `process_outcomes` 本身呼叫 `entities().delete()` 是規範的
//! 沉沒的結果是允許的。
//!
//! 為什麼這很重要：「omfx/game/src/sim_runner.rs」中的快照擷取器
//! 排出“RemovedEntitiesQueue”以填入“SimWorldSnapshot.removed_entity_ids”。
//! 渲染端場景節點清理鍵關閉該欄位。直接 `.delete(e)`
//! 跳過佇列會默默地洩漏 omfx 渲染狀態。

use std::fs;
use std::path::{Path, PathBuf};

const FORBIDDEN_PATTERNS: &[&str] = &["entities().delete(", ".delete_entity("];

/// 直接呼叫 `entities().delete()` 的檔案路徑是規範的
/// 接收器（process_outcomes'Outcome::EntityRemoved 手臂）。列入許可名單的人
/// 相對於板條箱根的後綴匹配。
const ALLOWLIST_SUFFIXES: &[&str] = &[
    // process_outcomes 本身：結果路由到的單一接收器
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
                    violations.push(format!(
                        "{}:{} — `{}` (use Outcome::EntityRemoved instead)",
                        rel,
                        lineno_zero + 1,
                        pat
                    ));
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
