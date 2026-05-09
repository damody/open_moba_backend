//! 伺服器節奏鎖步的輸入緩衝。
//!
//! 玩家提交攜帶target_tick的InputSubmit資料包（目前是
//! current_server_tick + 2；120Hz 時輸入延遲約 16.7ms）。伺服器收集
//! 每刻他們。當蜱蟲觸發時，緩衝區會耗盡所有目標輸入
//! 在那一刻進入“TickBatch”。
//!
//! 遲到的輸入（target_tick 已經過去）會被刪除並帶有日誌行 —
//! 玩家的客戶錯過了截止日期，將會看到該操作失敗。
//! 第 3+ 階段可能會新增「軟延遲擴展」政策。

use crate::lockstep::PlayerInput;
use std::collections::BTreeMap;
use std::time::Instant;

#[derive(Clone, Debug)]
pub struct BufferedPlayerInput {
    pub input: PlayerInput,
    pub input_id: u32,
    pub server_receive_tick: u32,
    pub server_receive_instant: Instant,
}

#[derive(Default)]
pub struct InputBuffer {
    /// target_tick→player_id→輸入加上線邊元資料。
    /// 外部 BTreeMap，因此在刻度鍵上，drain_for_tick 的複雜度為 O(log N)。
    /// 內部BTreeMap由player_id作為確定性迭代順序的鍵控
    /// 在 TickBatch 組合中。
    by_tick: BTreeMap<u32, BTreeMap<u32, BufferedPlayerInput>>,
}

impl InputBuffer {
    pub fn new() -> Self {
        Self {
            by_tick: BTreeMap::new(),
        }
    }

    /// 提交一項輸入。如果「target_tick <= current_tick」（晚）則回傳 false。
    /// `current_tick` 已經被廣播公司耗盡，所以相等
    /// 否則會將孤兒輸入停放在永遠不會再次觸發的蜱下。
    /// 如果同一玩家在同一時間提交兩次，則後者獲勝
    /// （覆蓋政策－客戶不應重複提交；如果這樣做，
    /// 第二個被解釋為更正）。
    pub fn submit(
        &mut self,
        current_tick: u32,
        player_id: u32,
        target_tick: u32,
        input: PlayerInput,
        input_id: u32,
    ) -> bool {
        if target_tick <= current_tick {
            return false; // late
        }
        self.by_tick
            .entry(target_tick)
            .or_insert_with(BTreeMap::new)
            .insert(
                player_id,
                BufferedPlayerInput {
                    input,
                    input_id,
                    server_receive_tick: current_tick,
                    server_receive_instant: Instant::now(),
                },
            );
        true
    }

    /// 耗盡針對此刻度的所有輸入。返回按player_id排序
    /// （BTreeMap 迭代是按關鍵順序進行的——確定性所必需的
    /// 所有對等點的 TickBatch 組合）。
    pub fn drain_for_tick(&mut self, tick: u32) -> Vec<(u32, BufferedPlayerInput)> {
        self.by_tick
            .remove(&tick)
            .map(|m| m.into_iter().collect())
            .unwrap_or_default()
    }

    /// 刪除所有早於 `before_tick` 的內容 — 定期調用
    /// 清理，以便緩衝區不會累積孤兒提交
    /// 擁有蜱以某種方式被跳過。
    pub fn evict_older(&mut self, before_tick: u32) {
        self.by_tick.retain(|&t, _| t >= before_tick);
    }

    /// 所有未來報價的待處理輸入總數（用於診斷）。
    pub fn pending_count(&self) -> usize {
        self.by_tick.values().map(|m| m.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lockstep::{NoOp, PlayerInputEnum};

    fn noop() -> PlayerInput {
        PlayerInput {
            action: Some(PlayerInputEnum::NoOp(NoOp {})),
        }
    }

    #[test]
    fn submit_and_drain() {
        let mut b = InputBuffer::new();
        assert!(b.submit(0, 1, 5, noop(), 41));
        assert!(b.submit(0, 2, 5, noop(), 42));
        let drained = b.drain_for_tick(5);
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].0, 1); // sorted by player_id
        assert_eq!(drained[1].0, 2);
        assert_eq!(drained[0].1.input_id, 41);
        assert_eq!(drained[1].1.input_id, 42);
        assert_eq!(drained[0].1.server_receive_tick, 0);
        assert_eq!(drained[1].1.server_receive_tick, 0);
        // 已排空 — 第二個排水管已空。
        assert!(b.drain_for_tick(5).is_empty());
    }

    #[test]
    fn late_input_rejected() {
        let mut b = InputBuffer::new();
        assert!(!b.submit(10, 1, 5, noop(), 1)); // target=5 < current=10
        assert!(!b.submit(10, 1, 10, noop(), 2)); // target=10 already drained
        assert_eq!(b.pending_count(), 0);
    }

    #[test]
    fn evict_older() {
        let mut b = InputBuffer::new();
        b.submit(0, 1, 1, noop(), 0);
        b.submit(0, 1, 2, noop(), 0);
        b.submit(0, 1, 3, noop(), 0);
        b.evict_older(2);
        assert!(b.drain_for_tick(1).is_empty());
        assert_eq!(b.drain_for_tick(2).len(), 1);
        assert_eq!(b.drain_for_tick(3).len(), 1);
    }
}
