//! KCP 網路位元組測量工具。
//!
//! 該文件本來應該是不干涉的整合測試，會產生一個
//! 伺服器 + 進程中的假客戶端，執行 TD_STRESS 30 秒，並斷言
//! `位元組/秒 < CURRENT_BUDGET`。使其在 Windows 上使用現有的
//! DLL 暫存管道 (`scripts/base_content.dll`) + `game.toml` STORY 交換
//! 事實證明 P1 很脆弱，所以我們又使用了**手動運行安全帶**：
//!
//! 1. 在 `src/main.rs` 中新增一個臨時的 5 秒轉儲程式來記錄
//! `counter.snapshot()` 增量（請參閱 P0/P1 檢查點提交中的模式
//! 我們不斷恢復）。或透過 MCP 暴露計數器。
//! 2. 來自「D:\omoba」的「run_stress.bat」。
//! 3. 讓它運行約 60 秒，以便場景到達遊戲後期（約 500 個可見的蠕動）。
//! 4. 比較最後 2~3 個 5 秒視窗的「位元組/秒」+每個事件的細分
//! 對照以下預算。
//! 5. 並道前將翻斗車恢復原狀。
//!
//! 不實施進程內工具的理由：P2 重寫
//! `proto/game.proto` 模擬 JSON 特定的位元組。建置
//! 現在利用，只是為了在大約 1 週內重寫它以用於 prost 編碼的事件，
//! 不值得付出努力。如果多次回歸成為問題
//! P2落地前，重訪。
//!
//! # 預算（位元組/秒，在 TD_STRESS ~500 可見蠕動視窗上測量）
//!
//! | Phase | Budget | Notes |
//! |-------|--------|-------|
//! | P0 baseline | ~206_000 | Pre-optimization, Late-game 5s window |
//! | P1 partial (1.1~1.3) | ~114_000 | Measured 2026-04-24, -45% vs baseline |
//! | **P1全(1.1~1.5)** | **~85_000** |預估：重複資料刪除 +10%，心跳 AOI +10% |
//! | P2 end | ~62_000 | Projected -70%: prost binary + Quantization |
//! | P3 end | ~48_000 | Projected -78%: HeroStatic cache |
//! | P4 end | ~31_000 | Projected -85%: CreepMove velocity extrapolation |
//! | P5 end | ~25_000 | Projected -88%: per-player AOI broadphase |
//!
//! 下面的“#[ignore]”測試存在，因此“cargo test”枚舉它；運行它
//! 故意恐慌提醒貢獻者使用手動安全帶。

pub const BASELINE_BPS_STEADY: u64 = 206_000; // P0 measured
pub const P1_BUDGET_BPS: u64 = 85_000; // Projected P1 full
pub const P2_BUDGET_BPS: u64 = 62_000; // Projected P2
pub const P3_BUDGET_BPS: u64 = 48_000; // Projected P3
pub const P4_BUDGET_BPS: u64 = 31_000; // Projected P4
pub const P5_BUDGET_BPS: u64 = 25_000; // Projected P5

#[test]
#[ignore]
fn kcp_bytes_budget_td_stress() {
    panic!(
        "manual-run harness. See module doc at top of file. \
         Budget for current phase: see BASELINE_BPS_STEADY and P*_BUDGET_BPS."
    );
}
