//! 階段 2 鎖步線路整合測試（用於手動驗證的佔位符）。
//!
//! 完整的快樂路徑工具 — 啟動真正的 tokio KCP 伺服器，
//! 連接兩個 GameClient 實例，交換 JoinRequest /
//! GameStart / InputSubmit / TickBatch — 被故意延後
//! 階段 2.5。在這裡旋轉進程內伺服器必須：
//!
//! 1.建構一個新的ECS World +調度程式（omb的main.rs組裝~30
//! 傳輸執行緒啟動之前的資源/系統），或者
//! 2. 產生 `target/debug/omobab.exe` 作為子程序 + 解析標準輸出
//! 撥號前「正在收聽…」。
//!
//! 兩條路徑都增加了真實的測試基礎設施表面（連接埠分配、子進程
//! Windows 上的生命週期，「scripts/base_content.dll」的 DLL 暫存，
//! game.toml STORY 覆蓋）僅在第 3 階段引入時才有回報
//! 真實的客戶端 SIM 消耗。直到那時電線層是
//! 行使者：
//!
//! - `omb/src/lockstep/input_buffer.rs` — 提交+耗盡+驅逐測試
//!   - `omb/src/lockstep/tick_broadcaster.rs` — fire_one_tick / hash /
//! 通道關閉/60 次驅逐測試（階段 2.5）
//! - `omb/src/transport/kcp_transport.rs` 測試 — 幀編碼，
//! InputSubmit 解碼往返、JoinRole 映射
//!
//! 下面的兩個「#[ignore]」佔位符固定網路的*形狀*
//! 階段 3 將填寫的往返測試，並記錄所需的內容
//! 函數體中的不變量作為註釋。
//!
//! 運行：`cargo test --test lockstep_integration -- --ignored`

#[tokio::test]
#[ignore] // requires real omb server; run via run.bat manually for Phase 2
async fn two_clients_receive_synchronized_tick_batch() {
    // 所需的不變量（第 3 階段將斷言這些）：
    // 1. `GameClient::connect("127.0.0.1:50062").await` 皆成功。
    // 2. 每個呼叫 `join_lockstep(name, JoinRole::Player)` 並接收
    // `GameStart {player_id, master_seed, start_tick }`。兩人
    // player_ids 不同，兩個 master_seeds 匹配。
    // 3. 每個人都透過 `subscribe_lockstep()` 訂閱並返回一個串流
    // `LockstepFrame`。
    // 4. 分別呼叫 `submit_input(target_tick = start_tick + 10,
    // PlayerInput::NoOp)`。
    // 5. 兩個流都會發出“TickBatch {tick: start_tick + 10, input }”
    // 其中 `inputs.len() == 2` 和 `inputs.iter().map(|i| i.player_id)
    // .collect::<BTreeSet<_>>()` 涵蓋了兩個player_ids。
    // 6. 兩個客戶端都以相同的順序看到 TickBatches（確定性
    // 階段 3 omoba-sim 消耗的重播前提條件）。
    panic!(
        "Phase 2.5 placeholder: spin up server + 2-client roundtrip is \
         deferred to Phase 3 when omoba-sim worker thread lands on omfx. \
         Manual smoke: run.bat + observe legacy GameEvent stream."
    );
}

#[tokio::test]
#[ignore]
async fn state_hash_broadcast_every_600_ticks() {
    // 所需的不變量：
    // 1. GameStart後，subscribe_lockstep串流接收第一個
    // StateHash 位於 configured 10s lockstep interval。
    // 2. 兩個客戶端收到相同的StateHash（佔位符哈希
    // 公式“tick * 0x9E3779B97F4A7C15”純粹依賴tick
    // 所以這在第 2 階段是非常正確的）。
    // 3. 第 3 階段交換到真正的 ECS 雜湊：客戶端運作相同
    // 相同 TickBatch 序列上的 omoba-sim 實例必須看到
    // 位元相同的哈希值——任何分歧都是一個不同步錯誤。
    panic!(
        "Phase 2.5 placeholder: see comment + tick_broadcaster unit tests \
         (`emits_state_hash_at_interval_multiples`, \
         `placeholder_state_hash_is_deterministic_pin`)."
    );
}
