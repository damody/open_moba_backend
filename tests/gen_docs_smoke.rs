//! 冒煙測試：端對端執行 gen-docs 並檢查輸出 HTML 包含
//! 預期標記。僅在可發現 base_content DLL 時運行
//! 透過 gen_docs 使用的相同候選清單；否則優雅地跳過。
//!
//! 明確運作：`cargo test --features gen-docs -- --ignored`

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
