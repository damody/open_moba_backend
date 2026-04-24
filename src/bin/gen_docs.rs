//! gen-docs — produce a self-contained HTML catalog of units, abilities,
//! and script API coverage.
//!
//! Design: docs/plans/2026-04-23-build-time-catalog-design.md

#[path = "gen_docs_lib/mod.rs"]
mod lib;

use anyhow::{Context, Result};
use clap::Parser;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Parser, Debug)]
#[command(name = "gen-docs", about = "Generate omoba unit & script API catalog HTML")]
struct Args {
    /// Output HTML path (relative to cwd)
    #[arg(long, default_value = "target/docs/index.html")]
    out: PathBuf,
    /// Story folder name under <story-root>/ (overrides game.toml)
    #[arg(long)]
    story: Option<String>,
    /// Path to base_content.dll (auto-detected if omitted)
    #[arg(long)]
    dll: Option<PathBuf>,
    /// script-abi source directory
    #[arg(long, default_value = "../scripts/script-abi/src")]
    abi_src: PathBuf,
    /// base_content source directory (for coverage scan)
    #[arg(long, default_value = "../scripts/base_content/src")]
    content_src: PathBuf,
    /// Story root directory
    #[arg(long, default_value = "Story")]
    story_root: PathBuf,
    /// game.toml path (to read STORY if --story is omitted)
    #[arg(long, default_value = "game.toml")]
    game_toml: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let story = args
        .story
        .clone()
        .unwrap_or_else(|| read_story_from_game_toml(&args.game_toml));
    let dll_path = args.dll.clone().unwrap_or_else(default_dll_path);

    let mut warnings: Vec<lib::model::Warning> = Vec::new();

    // 1. DLL (fatal)
    let dll = lib::dll::load(&dll_path)
        .with_context(|| format!("loading DLL {}", dll_path.display()))?;

    // 2. API scan (fatal)
    let api = lib::api_scan::scan(&args.abi_src)
        .with_context(|| format!("scanning script-abi at {}", args.abi_src.display()))?;

    // 3. Coverage (soft)
    let world_names: HashSet<String> =
        api.world_methods.iter().map(|m| m.name.clone()).collect();
    let impls = match lib::coverage::scan_dir(&args.content_src, &world_names) {
        Ok(v) => v,
        Err(e) => {
            warnings.push(lib::model::Warning {
                source: args.content_src.display().to_string(),
                message: format!("coverage scan failed: {e}"),
            });
            Vec::new()
        }
    };

    // 4. entity.json (soft)
    let story_dir = args.story_root.join(&story);
    let entity = match lib::entity::load(&story_dir) {
        Ok(d) => d,
        Err(e) => {
            warnings.push(lib::model::Warning {
                source: story_dir.display().to_string(),
                message: format!("entity.json load failed: {e}"),
            });
            lib::entity::EntityData {
                heroes: Default::default(),
                creeps: Default::default(),
            }
        }
    };

    // 5. merge
    let meta = lib::model::BuildMeta {
        timestamp: now_rfc3339(),
        git_sha: git_short_sha().unwrap_or_else(|_| "unknown".into()),
        story: story.clone(),
        sources: vec![
            dll_path.display().to_string().replace('\\', "/"),
            story_dir.display().to_string().replace('\\', "/"),
            args.abi_src.display().to_string().replace('\\', "/"),
            args.content_src.display().to_string().replace('\\', "/"),
        ],
    };
    let catalog = lib::merge::merge(dll, entity, api, impls, warnings, meta);

    // 6. render & write
    let html = lib::render::page(&catalog);
    if let Some(parent) = args.out.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating dir {}", parent.display()))?;
    }
    std::fs::write(&args.out, &html)
        .with_context(|| format!("writing {}", args.out.display()))?;

    println!(
        "gen-docs: {} units, {} abilities, {} warnings -> {}",
        catalog.units.len(),
        catalog.abilities.len(),
        catalog.warnings.len(),
        args.out.display(),
    );
    Ok(())
}

fn read_story_from_game_toml(path: &Path) -> String {
    let src = std::fs::read_to_string(path).unwrap_or_default();
    for line in src.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("STORY") {
            if let Some(q1) = rest.find('"') {
                if let Some(q2) = rest[q1 + 1..].find('"') {
                    return rest[q1 + 1..q1 + 1 + q2].to_string();
                }
            }
        }
    }
    "TD_1".to_string()
}

fn default_dll_path() -> PathBuf {
    // Try common locations in order. First existing wins.
    // NOTE: staged DLL (scripts/base_content.dll, 由 run.bat / stress 腳本 copy
    // 過來) 是最權威的正本，必須放第一位；避免誤載到陳舊的 target/release/。
    let candidates: &[&str] = &[
        // run.bat / stress 腳本 stage 的 staged 正本 — 最權威
        "scripts/base_content.dll",
        // fallback: omb 自家 target（若未來整合進同 workspace）
        "target/release/base_content.dll",
        "target/debug/base_content.dll",
        // fallback: scripts/ 這個獨立 workspace 直接 build 出來的位置
        "../scripts/target/release/base_content.dll",
        "../scripts/target/debug/base_content.dll",
    ];
    for c in candidates {
        let p = PathBuf::from(c);
        if p.exists() {
            return p;
        }
    }
    // Fall back to the conventional staged path for a clear error message
    PathBuf::from("scripts/base_content.dll")
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn git_short_sha() -> Result<String> {
    use std::process::Stdio;
    let out = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .stderr(Stdio::null())
        .output()?;
    if !out.status.success() {
        anyhow::bail!("git rev-parse failed");
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
