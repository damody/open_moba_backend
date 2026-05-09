#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    const TRACE_METADATA_TOKENS: &[&str] = &[
        "server_receive_tick",
        "server_drain_tick",
        "server_queue_us",
        "submit_start_us",
        "submit_done_us",
        "client_receive_us",
        "origin_kind",
        "origin_us",
        "send_lockstep_input_us",
        "client_receive_tickbatch_us",
        "game_forward_to_sim_us",
        "sim_publish_snapshot_us",
        "sim_publish_us",
        "InputOriginKind",
        "LockstepInputMsg",
        "LockstepTickInput",
        "TickBatchInput",
        "AppliedInputMeta",
        "applied_input_meta",
    ];

    const TRACE_METADATA_ALLOWED_FILES: &[&str] = &[
        "omb/src/lockstep/input_buffer.rs",
        "omb/src/lockstep/tick_broadcaster.rs",
        "omb/src/lockstep/metadata_guard.rs",
        "omb/src/transport/kcp_transport.rs",
        "omfx/game/src/lockstep_client.rs",
        "omfx/game/src/lib.rs",
        "omfx/game/src/sim_runner.rs",
        "proto/game.proto",
    ];

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("omb lives under the repo root")
            .to_path_buf()
    }

    fn normalized_relative_path(root: &Path, path: &Path) -> String {
        path.strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    }

    fn collect_files(dir: &Path, extensions: &[&str], out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_files(&path, extensions, out);
                continue;
            }

            let ext = path
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or_default();
            if extensions.iter().any(|wanted| *wanted == ext) {
                out.push(path);
            }
        }
    }

    #[test]
    fn phase_trace_metadata_stays_on_wire_edge_or_client_trace_path() {
        let root = repo_root();
        let mut files = Vec::new();
        for (relative, extensions) in [
            ("omb/src", &["rs"][..]),
            ("omfx/game/src", &["rs"][..]),
            ("omoba-core/src", &["rs"][..]),
            ("omoba-sim/src", &["rs"][..]),
            ("scripts/base_content/src", &["rs"][..]),
            ("proto", &["proto"][..]),
        ] {
            collect_files(&root.join(relative), extensions, &mut files);
        }

        let mut violations = Vec::new();
        for path in files {
            let relative = normalized_relative_path(&root, &path);
            if TRACE_METADATA_ALLOWED_FILES.contains(&relative.as_str()) {
                continue;
            }

            let contents = fs::read_to_string(&path).unwrap_or_default();
            for token in TRACE_METADATA_TOKENS {
                if contents.contains(token) {
                    violations.push(format!("{relative}: contains {token}"));
                }
            }
        }

        assert!(
            violations.is_empty(),
            "lockstep phase trace metadata must not enter gameplay ECS/state paths:\n{}",
            violations.join("\n")
        );
    }

    #[test]
    fn input_id_does_not_enter_gameplay_state_or_scripts() {
        let root = repo_root();
        let mut files = Vec::new();
        for relative in [
            "omb/src/ability_runtime",
            "omb/src/comp",
            "omb/src/item",
            "omb/src/scripting",
            "omb/src/state",
            "omb/src/tick",
            "omb/src/vision",
            "omoba-sim/src",
            "scripts/base_content/src",
        ] {
            collect_files(&root.join(relative), &["rs"], &mut files);
        }

        let mut violations = Vec::new();
        for path in files {
            let contents = fs::read_to_string(&path).unwrap_or_default();
            if contents.contains("input_id") {
                violations.push(normalized_relative_path(&root, &path));
            }
        }

        assert!(
            violations.is_empty(),
            "input_id is wire/trace metadata and must not enter gameplay state or scripts:\n{}",
            violations.join("\n")
        );
    }
}
