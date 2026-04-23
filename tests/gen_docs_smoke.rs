//! Smoke test: run gen-docs end-to-end and check the output HTML contains
//! expected markers. Only runs when the base_content DLL is discoverable
//! via the same candidate list gen_docs uses; otherwise gracefully skips.
//!
//! Run explicitly with: `cargo test --features gen-docs -- --ignored`

#![cfg(feature = "gen-docs")]

use std::path::PathBuf;
use std::process::Command;

fn dll_present() -> bool {
    for c in &[
        "target/release/base_content.dll",
        "target/debug/base_content.dll",
        "scripts/base_content.dll",
        "../scripts/target/release/base_content.dll",
        "../scripts/target/debug/base_content.dll",
    ] {
        if PathBuf::from(c).exists() { return true; }
    }
    false
}

#[test]
#[ignore]
fn produces_html_with_known_content() {
    if !dll_present() {
        eprintln!("no base_content.dll discoverable; skipping smoke test");
        return;
    }

    let out = PathBuf::from("target/docs/smoke.html");
    let status = Command::new(env!("CARGO"))
        .args([
            "run", "--release",
            "-p", "omobab", "--bin", "gen-docs",
            "--features", "gen-docs", "--",
            "--out", out.to_str().unwrap(),
        ])
        .status()
        .expect("spawn gen-docs");
    assert!(status.success(), "gen-docs exited non-zero");

    let html = std::fs::read_to_string(&out).expect("read output");
    assert!(html.len() > 50_000,
        "HTML too small ({}B); render regression?", html.len());
    assert!(html.contains("omoba catalog"), "missing title");
    assert!(html.contains("UnitScript Hooks"), "missing API section");
    assert!(html.contains("Coverage Matrix"), "missing coverage section");
    assert!(html.contains("Stat Keys"), "missing stat keys section");
}
