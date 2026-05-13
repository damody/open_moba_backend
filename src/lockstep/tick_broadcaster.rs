//! Configured-cadence 滴答廣播。在從 main.rs 產生的 tokio 任務中運行。
//!
//! 每個刻度：
//! 1. 前進 `LockstepState.current_tick`。
//! 2. 將該刻度的「InputBuffer」排空到「TickBatch」中。
//! 3. 透過 `OutboundMsg::lockstep_frame(...)` 發送批次，以便
//! kcp 傳輸的廣播線程（任務 2.3）向所有發送標籤 0x11
//! 連線的鎖步會話。
//! 4. 每個 `state_hash_interval` 滴答，也會發出一個 `StateHash` （標籤 0x12）
//! 用於不同步檢測。
//!
//! 第二階段狀態：
//! - `placeholder_state_hash` 是一個替代品 (`tick * Golden_ratio`)；第三階段
//! 將其替換為 `omoba_sim::state_hash::hash_sorted_by_id`
//! 真實的 ECS 狀態。
//! - 類比調度程式與 broadcaster 使用相同 configured lockstep cadence。
//! - 在第 2 階段，`server_events` 永遠為空；第5階段將注入
//! player_join / wave_start / 等伺服器權威事件。
//!
//! 3.4階段狀態：
//! - 可選的“state_hash_rx”通道由調度程序滴答循環提供
//! `state::core::State::tick`，呼叫 `compute_state_hash(&world)`
//! 每個“state_hash_interval”刻度（使用其自己的調度程序刻度號）。
//! 廣播者「try_recv」是其 configured-cadence 狀態時的最新待定值 -
//! 哈希間隔觸發。
//! - 調度員和廣播員都使用 configured cadence。哈希值帶有調度程序的時間戳
//! 計算時間；廣播公司逐字轉寄「(tick, hash)」。
//! - 當「state_hash_rx」為「None」（遺留/測試設定）時，廣播者
//! 回退到“placeholder_state_hash”，以便現有測試繼續通過。

use crossbeam_channel::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use tokio::time::{interval, Duration, MissedTickBehavior};

use crate::lockstep::{
    InputBuffer, InputForPlayer, LockstepFrame, LockstepState, StateHash, TickBatch,
};
use crate::transport::OutboundMsg;
use omoba_core::lockstep_timing::LockstepTiming;

/// 階段3.4：調度程序tick循環計算後發布的有效負載
/// `compute_state_hash(&world)`。廣播公司逐字轉發—‘tick’
/// 這是調度員的滴答聲（configured cadence），與
/// 廣播公司自己的 `LockstepState.current_tick`。
pub type StateHashSample = (u32, u64);

#[derive(Clone, Copy, Debug)]
pub struct TickBroadcasterConfig {
    /// 以微秒為單位的刻度週期。預設由 `LOCKSTEP_TPS` 推導。
    pub tick_period_us: u64,
    /// Server-authoritative step FPS used for diagnostics and tick windows.
    pub step_fps: u32,
    /// 每 N 個週期發出一個 StateHash。預設 10 秒 @ `LOCKSTEP_TPS`。
    pub state_hash_interval: u32,
    /// Every N ticks, old future inputs are evicted.
    pub input_evict_interval: u32,
    /// Keep about this many ticks of future input before eviction.
    pub input_retention_ticks: u32,
}

impl Default for TickBroadcasterConfig {
    fn default() -> Self {
        Self::from_timing(LockstepTiming::DEFAULT)
    }
}

impl TickBroadcasterConfig {
    pub fn from_timing(timing: LockstepTiming) -> Self {
        Self {
            tick_period_us: timing.tick_period_us(),
            step_fps: timing.step_fps(),
            state_hash_interval: timing.ticks_for_seconds(10),
            input_evict_interval: timing.ticks_for_seconds(1),
            input_retention_ticks: timing.ticks_for_seconds(2),
        }
    }
}

pub struct TickBroadcaster {
    config: TickBroadcasterConfig,
    input_buffer: Arc<Mutex<InputBuffer>>,
    state: Arc<Mutex<LockstepState>>,
    out_tx: Sender<OutboundMsg>,
    /// 階段 3.4：調度程序計算的狀態雜湊值的可選來源。什麼時候
    /// “Some”，廣播者“try_recv”在每個狀態哈希標記上並轉發
    /// 最新的待處理樣本（記錄警告 + 發出哈希=0，如果
    /// 頻道是空的）。 `None` 回退到 `placeholder_state_hash`。
    state_hash_rx: Option<Receiver<StateHashSample>>,
    /// Phase 5.x 橋接器：可選的 sidecar，可反映每次滴答耗盡的情況
    /// PlayerInputs 到主機自己的調度程式（State::tick 讀取
    /// 相應的橫樑接收器）。如果沒有這個，輸入將達到
    /// 客戶端透過 TickBatch 但絕不是主機的“PendingPlayerInputs”，
    /// 所以主機端遊戲狀態（例如 `CurrentCreepWave.is_running`）永遠不會
    /// 翻轉開始回合。
    host_input_tx: Option<crossbeam_channel::Sender<Vec<(u32, crate::lockstep::PlayerInput)>>>,
}

impl TickBroadcaster {
    pub fn new(
        config: TickBroadcasterConfig,
        input_buffer: Arc<Mutex<InputBuffer>>,
        state: Arc<Mutex<LockstepState>>,
        out_tx: Sender<OutboundMsg>,
    ) -> Self {
        Self {
            config,
            input_buffer,
            state,
            out_tx,
            state_hash_rx: None,
            host_input_tx: None,
        }
    }

    /// 阶段 3.4：附加调度程序端状态哈希源。建造者風格
    /// 因此現有的呼叫站點（包括測試）不會中斷。
    pub fn with_state_hash_rx(mut self, rx: Receiver<StateHashSample>) -> Self {
        self.state_hash_rx = Some(rx);
        self
    }

    /// 階段 5.x：附加一個 sidecar，將耗盡的輸入轉送到主機
    /// 調度程序的 `State::tick`。主機透過配套橫樑讀取
    /// 接收器並將它們鏡像到其“PendingPlayerInputs”資源中。
    pub fn with_host_input_tx(
        mut self,
        tx: crossbeam_channel::Sender<Vec<(u32, crate::lockstep::PlayerInput)>>,
    ) -> Self {
        self.host_input_tx = Some(tx);
        self
    }

    /// 產生 configured-cadence 滴答循環。運行直到“out_tx”關閉（通道
    /// 作為發送錯誤斷開表面，然後我們記錄+退出）。
    pub async fn run(self) {
        // 階段 4：如果未連接 state_hash_rx，則會發出響亮的啟動時警告。
        // 生產代碼必須連接它（請參閱 main.rs `with_state_hash_rx`）；
        // 遺失的 rx 會默默地回退到佔位符哈希，這將
        // 屏蔽跨客戶端不同步。測試設定故意保留 None 和
        // 接受單元測試穩定性的佔位符。
        if self.state_hash_rx.is_none() {
            log::error!(
                "TickBroadcaster: state_hash_rx is None — broadcasting placeholder \
                 state hash (tick * golden_ratio). This is OK for tests but a \
                 wire-up regression in production. Lockstep desync detection is DEGRADED."
            );
        } else {
            log::info!("TickBroadcaster: state_hash_rx wired — using authoritative ECS hash");
        }
        let mut ticker = interval(Duration::from_micros(self.config.tick_period_us));
        // Preserve the configured average cadence. On Windows, individual timer
        // wakeups can land late enough that Delay mode drifts 120Hz down toward
        // 80-90Hz; Burst mode catches the next deadline back up instead.
        ticker.set_missed_tick_behavior(MissedTickBehavior::Burst);
        // tokio 的第一個間隔刻度立即觸發；跳過它所以
        // 第一個發布的價格變動出現在 +period，而不是 t=0。
        ticker.tick().await;
        loop {
            ticker.tick().await;
            if !self.fire_one_tick() {
                log::info!("TickBroadcaster: outbound channel closed, exiting tick loop");
                break;
            }
        }
    }

    /// 激發一滴。如果出站通道關閉則回傳 false
    /// （表示傳輸已關閉 - 呼叫者退出循環）。
    fn fire_one_tick(&self) -> bool {
        // 提前刻度計數器。
        let tick = {
            let mut s = self.state.lock().unwrap();
            s.current_tick = s.current_tick.wrapping_add(1);
            s.current_tick
        };

        // 針對此刻度的漏極輸入。
        let inputs = self.input_buffer.lock().unwrap().drain_for_tick(tick);

        // 階段 5.x：將耗盡的輸入鏡像到主機調度程式（State::tick
        // 透過橫樑接收器讀取）。在使用“輸入”之前發送
        // 下面的原始轉換。
        if !inputs.is_empty() {
            if let Some(tx) = self.host_input_tx.as_ref() {
                let host_inputs: Vec<_> = inputs
                    .iter()
                    .map(|(player_id, buffered)| (*player_id, buffered.input.clone()))
                    .collect();
                if let Err(e) = tx.send(host_inputs) {
                    log::warn!("TickBroadcaster: host_input_tx send failed: {e}");
                }
            }
        }

        let inputs_proto: Vec<InputForPlayer> = inputs
            .into_iter()
            .map(|(player_id, buffered)| InputForPlayer {
                player_id,
                input: Some(buffered.input),
                input_id: buffered.input_id,
                server_receive_tick: buffered.server_receive_tick,
                server_drain_tick: tick,
                server_queue_us: buffered
                    .server_receive_instant
                    .elapsed()
                    .as_micros()
                    .min(u64::MAX as u128) as u64,
            })
            .collect();

        let batch = TickBatch {
            tick,
            inputs: inputs_proto,
            // 第2階段：空；第 5+ 階段注入 PlayerJoin/WaveStart/等。
            server_events: vec![],
            lua_content_generation: omoba_template_ids::runtime_lua_content_generation()
                .ok()
                .flatten()
                .unwrap_or(0),
            lua_content_hash: omoba_template_ids::runtime_lua_content_hash()
                .ok()
                .flatten()
                .unwrap_or_default(),
        };

        let msg = OutboundMsg::lockstep_frame(LockstepFrame::TickBatch(batch));
        if let Err(e) = self.out_tx.send(msg) {
            log::warn!("TickBroadcaster failed to send TickBatch: {e}");
            return false;
        }

        // 週期性狀態哈希。第 3.4 階段的雜湊值來源為
        // `state_hash_rx`（調度程序）可用時；否則就會回落
        // 到“placeholder_state_hash”。
        if tick % self.config.state_hash_interval == 0 {
            let (hash_tick, hash) = self.latest_state_hash(tick);
            let sh = StateHash {
                tick: hash_tick,
                hash,
            };
            let msg = OutboundMsg::lockstep_frame(LockstepFrame::StateHash(sh));
            if let Err(e) = self.out_tx.send(msg) {
                log::warn!("TickBroadcaster failed to send StateHash: {e}");
                return false;
            }
        }

        // 定期清理過時的未來輸入（例如提交的內容）
        // 引用了我們已經通過的勾號，因為玩家是
        // 斷開連接並重新連接）。
        if tick % self.config.input_evict_interval == 0 {
            self.input_buffer
                .lock()
                .unwrap()
                .evict_older(tick.saturating_sub(self.config.input_retention_ticks));
        }

        true
    }

    /// 第 3 階段：替換為真正的 `omoba_sim::state_hash::hash_sorted_by_id`
    /// 超過權威的 ECS 狀態。佔位符是確定性的
    /// 因此可以在第二階段整合測試中使用線路路徑。
    fn placeholder_state_hash(&self, tick: u32) -> u64 {
        (tick as u64).wrapping_mul(0x9E3779B97F4A7C15)
    }

    /// 階段 3.4：返回「(tick_to_broadcast, hash)」。
    ///
    /// - 如果 `state_hash_rx` 已連線：排空頻道並轉送
    /// 最新的待處理樣本（較新的調度程序樣本丟棄較舊的樣本）。
    /// 傳回的刻度是計算時調度程序的刻度，
    /// 可能會滯後廣播公司的「tick」最多一個調度程式 tick。清空通道 → 記錄警告 + 回傳 `(tick, 0)`。
    /// - 如果 `state_hash_rx` 為 None：回退到 `placeholder_state_hash`。
    fn latest_state_hash(&self, broadcaster_tick: u32) -> (u32, u64) {
        match &self.state_hash_rx {
            Some(rx) => {
                let mut latest: Option<StateHashSample> = None;
                while let Ok(sample) = rx.try_recv() {
                    latest = Some(sample);
                }
                match latest {
                    Some((t, h)) => (t, h),
                    None => {
                        log::warn!(
                            "TickBroadcaster: no fresh state_hash sample at tick {}, broadcasting hash=0",
                            broadcaster_tick
                        );
                        (broadcaster_tick, 0)
                    }
                }
            }
            None => (
                broadcaster_tick,
                self.placeholder_state_hash(broadcaster_tick),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    //! TickBroadcaster 的第 2.5 階段單元測試。
    //!
    //! 這些直接透過建立廣播器來練習“fire_one_tick”
    //! 使用類比出站通道 (crossbeam_channel::unbounded)，播種
    //! 具有合成提交的InputBuffer，並斷言
    //! 產生的“OutboundMsg::lockstep_frame”有效負載。避開東京
    //! 透過跳過“run()”並呼叫“fire_one_tick”來運行時依賴
    //! 同步地。
    use super::*;
    use crate::lockstep::{NoOp, PlayerInput, PlayerInputEnum};
    use crossbeam_channel::unbounded;

    fn noop_input() -> PlayerInput {
        PlayerInput {
            action: Some(PlayerInputEnum::NoOp(NoOp {})),
        }
    }

    /// Helper：將來自 rx 的每個訊息拉入 Vec，並按幀類型分類。
    fn drain_frames(rx: &crossbeam_channel::Receiver<OutboundMsg>) -> Vec<LockstepFrame> {
        let mut out = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            if let Some(frame) = msg.lockstep_frame {
                out.push(frame);
            }
        }
        out
    }

    fn make_broadcaster(
        config: TickBroadcasterConfig,
    ) -> (
        TickBroadcaster,
        Arc<Mutex<InputBuffer>>,
        Arc<Mutex<LockstepState>>,
        crossbeam_channel::Receiver<OutboundMsg>,
    ) {
        let buf = Arc::new(Mutex::new(InputBuffer::new()));
        let state = Arc::new(Mutex::new(LockstepState::new(0xCAFE_BABE_DEAD_BEEF)));
        let (tx, rx) = unbounded();
        let bc = TickBroadcaster::new(config, buf.clone(), state.clone(), tx);
        (bc, buf, state, rx)
    }

    #[test]
    fn fires_tick_batches_with_buffered_inputs() {
        let cfg = TickBroadcasterConfig {
            tick_period_us: LockstepTiming::DEFAULT.tick_period_us(),
            step_fps: LockstepTiming::DEFAULT.step_fps(),
            state_hash_interval: 600,
            input_evict_interval: LockstepTiming::DEFAULT.ticks_for_seconds(1),
            input_retention_ticks: LockstepTiming::DEFAULT.ticks_for_seconds(2),
        };
        let (bc, buf, _state, rx) = make_broadcaster(cfg);

        // 種子：在刻度 5 處，2 個玩家的輸入（玩家 7 + 玩家 3），所以我們
        // 可以確認 BTreeMap 排序透過 TickBatch 進行。
        {
            let mut b = buf.lock().unwrap();
            assert!(b.submit(0, 7, 5, noop_input(), 107));
            assert!(b.submit(0, 3, 5, noop_input(), 103));
        }

        // 發射 5 個刻度。勾選 1..=4 應為空 TickBatch，勾選 5 有 2 個輸入。
        for _ in 0..5 {
            assert!(
                bc.fire_one_tick(),
                "fire_one_tick returned false (channel closed?)"
            );
        }

        let frames = drain_frames(&rx);
        assert_eq!(
            frames.len(),
            5,
            "expected 5 TickBatch frames, got {}",
            frames.len()
        );

        for (i, frame) in frames.iter().enumerate() {
            let expect_tick = (i + 1) as u32;
            match frame {
                LockstepFrame::TickBatch(b) => {
                    assert_eq!(b.tick, expect_tick);
                    if expect_tick == 5 {
                        assert_eq!(b.inputs.len(), 2, "tick 5 should carry 2 inputs");
                        // BTreeMap 迭代順序：3，然後 7。
                        assert_eq!(b.inputs[0].player_id, 3);
                        assert_eq!(b.inputs[1].player_id, 7);
                        assert_eq!(b.inputs[0].input_id, 103);
                        assert_eq!(b.inputs[1].input_id, 107);
                        assert_eq!(b.inputs[0].server_receive_tick, 0);
                        assert_eq!(b.inputs[1].server_receive_tick, 0);
                        assert_eq!(b.inputs[0].server_drain_tick, 5);
                        assert_eq!(b.inputs[1].server_drain_tick, 5);
                    } else {
                        assert!(b.inputs.is_empty(), "tick {} should be empty", expect_tick);
                    }
                }
                other => panic!("expected TickBatch frame, got {:?}", other),
            }
        }
    }

    #[test]
    fn emits_state_hash_at_interval_multiples() {
        // 使用較小的間隔 (3) 以避免在測試中觸發 600 個刻度。
        let cfg = TickBroadcasterConfig {
            tick_period_us: LockstepTiming::DEFAULT.tick_period_us(),
            step_fps: LockstepTiming::DEFAULT.step_fps(),
            state_hash_interval: 3,
            input_evict_interval: LockstepTiming::DEFAULT.ticks_for_seconds(1),
            input_retention_ticks: LockstepTiming::DEFAULT.ticks_for_seconds(2),
        };
        let (bc, _buf, _state, rx) = make_broadcaster(cfg);

        // 觸發 7 個刻度 → 預計 StateHash 在第 3 和 6 個刻度。
        for _ in 0..7 {
            assert!(bc.fire_one_tick());
        }

        let frames = drain_frames(&rx);
        // 7 TickBatch + 2 StateHash = 9 幀。
        assert_eq!(frames.len(), 9, "frames = {:?}", frames);

        let mut tick_batch_count = 0;
        let mut state_hash_ticks = Vec::new();
        for frame in &frames {
            match frame {
                LockstepFrame::TickBatch(_) => tick_batch_count += 1,
                LockstepFrame::StateHash(sh) => state_hash_ticks.push(sh.tick),
                other => panic!("unexpected frame variant: {:?}", other),
            }
        }
        assert_eq!(tick_batch_count, 7);
        assert_eq!(state_hash_ticks, vec![3, 6]);
    }

    #[test]
    fn placeholder_state_hash_is_deterministic_pin() {
        // 固定確切的佔位符公式。第三階段改變了這一點——當
        // 確實如此，這個測試應該會失敗並淘汰（請參閱
        // 佔位符_狀態_哈希）。
        let cfg = TickBroadcasterConfig::default();
        let (bc, _buf, _state, _rx) = make_broadcaster(cfg);

        // 刻度=600 * 0x9E3779B97F4A7C15（黃金比例常數）。
        let expected_600: u64 = 600u64.wrapping_mul(0x9E3779B97F4A7C15);
        assert_eq!(bc.placeholder_state_hash(600), expected_600);

        // tick=1：常量本身。
        assert_eq!(bc.placeholder_state_hash(1), 0x9E3779B97F4A7C15);

        // tick=0 始終哈希為 0（乘法恆等式）。
        assert_eq!(bc.placeholder_state_hash(0), 0);
    }

    #[test]
    fn returns_false_when_outbound_channel_closes() {
        let cfg = TickBroadcasterConfig::default();
        let (bc, _buf, _state, rx) = make_broadcaster(cfg);
        drop(rx); // close the channel
                  // 第一次傳送失敗 → fire_one_tick 回傳 false。
        assert!(!bc.fire_one_tick());
    }

    #[test]
    fn evicts_old_inputs_every_second() {
        // 驗證定期清理分支（`tick % LOCKSTEP_ONE_SECOND_TICKS_U32 == 0`）。
        let cfg = TickBroadcasterConfig {
            tick_period_us: LockstepTiming::DEFAULT.tick_period_us(),
            step_fps: LockstepTiming::DEFAULT.step_fps(),
            state_hash_interval: 100_000, // disable state hash for this test
            input_evict_interval: LockstepTiming::DEFAULT.ticks_for_seconds(1),
            input_retention_ticks: LockstepTiming::DEFAULT.ticks_for_seconds(2),
        };
        let (bc, buf, _state, _rx) = make_broadcaster(cfg);

        // 在tick=10時提交一個孤立輸入（將在tick 10耗盡），
        // 還有一個在tick=200時的孤兒，我們永遠無法透過火到達。
        // 然後手動重新提交過時的未來輸入進行測試
        // 驅逐。
        // 更簡單：直接插入並檢查pending_count行為。
        {
            let mut b = buf.lock().unwrap();
            b.submit(0, 1, 200, noop_input(), 0);
            assert_eq!(b.pending_count(), 1);
        }

        // 發射一秒的刻度。在第一秒時，保留兩秒視窗，因此 tick=200 輸入仍存在。
        // 被驅逐（tick=200 輸入仍然存在）。
        for _ in 0..LockstepTiming::DEFAULT.ticks_for_seconds(1) {
            assert!(bc.fire_one_tick());
        }
        assert_eq!(
            buf.lock().unwrap().pending_count(),
            1,
            "tick=200 input should survive"
        );

        // 觸發到 tick=180，仍然未達 tick=200 條目。
        for _ in LockstepTiming::DEFAULT.ticks_for_seconds(1)..180 {
            assert!(bc.fire_one_tick());
        }
        assert_eq!(buf.lock().unwrap().pending_count(), 1);

        // 火到刻度 240 — 經過刻度 200，因此它會自然排出。
        for _ in 180..240 {
            assert!(bc.fire_one_tick());
        }
        assert_eq!(buf.lock().unwrap().pending_count(), 0);
    }

    /// 階段 3.5：當連線 `state_hash_rx` 時（生產
    /// 配置），廣播者轉發調度程序發布的哈希值
    /// 逐字代替佔位符。驗證（勾選，散列）對
    /// 登陸“LockstepFrame::StateHash”與發送的內容匹配
    /// 通道，並且該佔位符公式被繞過。
    #[test]
    fn broadcaster_uses_real_hash_when_rx_provided() {
        let cfg = TickBroadcasterConfig {
            tick_period_us: LockstepTiming::DEFAULT.tick_period_us(),
            step_fps: LockstepTiming::DEFAULT.step_fps(),
            state_hash_interval: 3, // small interval so we hit it quickly
            input_evict_interval: LockstepTiming::DEFAULT.ticks_for_seconds(1),
            input_retention_ticks: LockstepTiming::DEFAULT.ticks_for_seconds(2),
        };
        let (buf, state) = (
            Arc::new(Mutex::new(InputBuffer::new())),
            Arc::new(Mutex::new(LockstepState::new(0xCAFE_BABE_DEAD_BEEF))),
        );
        let (out_tx, out_rx) = unbounded();
        let (hash_tx, hash_rx) = unbounded::<StateHashSample>();

        let bc = TickBroadcaster::new(cfg, buf.clone(), state.clone(), out_tx)
            .with_state_hash_rx(hash_rx);

        // 傳送已知的調度程式樣本：tick=42，hash=0xCAFE_FOOD_DEAD_FEED。
        let known_hash: u64 = 0xCAFE_F00D_DEAD_FEED;
        let dispatcher_tick: u32 = 42;
        hash_tx
            .send((dispatcher_tick, known_hash))
            .expect("send hash sample");

        // 觸發 3 個刻度 → 在刻度 = 3 時，廣播公司觸發其狀態雜湊
        // 間隔並排空通道。
        for _ in 0..3 {
            assert!(bc.fire_one_tick());
        }

        // 尋找 StateHash 框架。
        let frames = drain_frames(&out_rx);
        let mut found_state_hash = false;
        for frame in &frames {
            if let LockstepFrame::StateHash(sh) = frame {
                assert_eq!(
                    sh.hash, known_hash,
                    "broadcaster must forward the dispatcher's real hash, not placeholder"
                );
                assert_eq!(
                    sh.tick, dispatcher_tick,
                    "broadcaster must forward the dispatcher's tick stamp, not its own tick"
                );
                // 理智：broadcaster_tick=3 的佔位符是
                // 3 * 0x9E3779B97F4A7C15 — 不應匹配。
                let placeholder_3: u64 = 3u64.wrapping_mul(0x9E3779B97F4A7C15);
                assert_ne!(
                    sh.hash, placeholder_3,
                    "broadcaster fell back to placeholder despite rx wired"
                );
                found_state_hash = true;
            }
        }
        assert!(
            found_state_hash,
            "expected a StateHash frame in {:?}",
            frames
        );
    }
}
