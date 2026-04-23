//! gen-docs — produce a self-contained HTML catalog of units, abilities,
//! and script API coverage. Design: docs/plans/2026-04-23-build-time-catalog-design.md
#![allow(dead_code)]

#[path = "gen_docs_lib/mod.rs"]
mod lib;

use lib::model;

fn main() -> anyhow::Result<()> {
    let _ = model::ApiSpec::default();
    println!("gen-docs placeholder (model wired)");
    Ok(())
}
