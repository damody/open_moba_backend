//! KCP network-bytes measurement harness.
//!
//! This file WAS supposed to be a hands-off integration test that spawns a
//! server + fake client in-process, runs TD_STRESS for 30 seconds, and asserts
//! `bytes/sec < CURRENT_BUDGET`. Making that work on Windows with the existing
//! DLL-staging pipeline (`scripts/base_content.dll`) + `game.toml` STORY swap
//! proved fragile for P1, so we fell back to a **manual-run harness**:
//!
//! 1. Add a temporary 5-second dumper in `src/main.rs` that logs
//!    `counter.snapshot()` deltas (see the pattern in P0/P1-checkpoint commits
//!    we keep reverting). Or expose counter via MCP.
//! 2. `run_stress.bat` from `D:\omoba`.
//! 3. Let it run ~60s so the scenario reaches late-game (~500 visible creep).
//! 4. Compare the last 2~3 5-second windows' `bytes/sec` + per-event breakdown
//!    against the budgets below.
//! 5. Revert the dumper before merging.
//!
//! Rationale for not implementing the in-process harness: P2 rewrites
//! `proto/game.proto` which moots the JSON-specific bytes. Building the
//! harness now, only to rewrite it for prost-encoded events in ~1 week,
//! is not worth the effort. If multi-run regression becomes a problem
//! before P2 lands, revisit.
//!
//! # Budgets (bytes/sec, measured on TD_STRESS ~500 visible creep window)
//!
//! | Phase | Budget | Notes |
//! |-------|--------|-------|
//! | P0 baseline | ~206_000 | Pre-optimization, Late-game 5s window |
//! | P1 partial (1.1~1.3) | ~114_000 | Measured 2026-04-24, -45% vs baseline |
//! | **P1 full (1.1~1.5)** | **~85_000** | Projected: +10% from dedupe, +10% heartbeat AOI |
//! | P2 end | ~62_000 | Projected -70%: prost binary + Quantization |
//! | P3 end | ~48_000 | Projected -78%: HeroStatic cache |
//! | P4 end | ~31_000 | Projected -85%: CreepMove velocity extrapolation |
//! | P5 end | ~25_000 | Projected -88%: per-player AOI broadphase |
//!
//! The `#[ignore]` test below exists so `cargo test` enumerates it; running it
//! panics intentionally to remind contributors to use the manual harness.

pub const BASELINE_BPS_STEADY: u64 = 206_000; // P0 measured
pub const P1_BUDGET_BPS: u64 = 85_000;        // Projected P1 full
pub const P2_BUDGET_BPS: u64 = 62_000;        // Projected P2
pub const P3_BUDGET_BPS: u64 = 48_000;        // Projected P3
pub const P4_BUDGET_BPS: u64 = 31_000;        // Projected P4
pub const P5_BUDGET_BPS: u64 = 25_000;        // Projected P5

#[test]
#[ignore]
fn kcp_bytes_budget_td_stress() {
    panic!(
        "manual-run harness. See module doc at top of file. \
         Budget for current phase: see BASELINE_BPS_STEADY and P*_BUDGET_BPS."
    );
}
