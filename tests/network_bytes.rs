//! 整合測試：跑 TD_STRESS 30s，assert kcp bytes/sec 低於 budget。
//! 目前只 scaffold，Phase 之後每個階段更新 budget。

#[test]
#[ignore]
fn kcp_bytes_budget_td_stress() {
    // TODO: Phase 1 之後填實作
    // 1. 用 game.toml.stress variant 啟 server in-process
    // 2. mock kcp client subscribe all topics
    // 3. sleep 30s，讀 KcpTransport::bytes_counter().snapshot()
    // 4. assert total_bytes / 30 < CURRENT_BUDGET
    panic!("not yet implemented");
}
