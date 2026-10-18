//! 伺服器節奏鎖步的輸入緩衝。
//!
//! 玩家提交攜帶target_tick的InputSubmit資料包（目前是
//! client-selected future tick）。伺服器收集
//! 每刻他們。當蜱蟲觸發時，緩衝區會耗盡所有目標輸入
//! 在那一刻進入“TickBatch”。
//!
//! 剛晚到的輸入會被 server retarget 到下一個 tick，避免本機 thread
//! phase 差造成偶發掉 input；超過 grace window 的陳舊輸入才會丟棄。

use crate::lockstep::PlayerInput;
use std::collections::BTreeMap;
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputSubmitResult {
    /// A previously queued shop command; never enqueue or settle it again.
    DuplicateShop {
        effective_tick: u32,
    },
    /// Same ID with changed payload/tick, zero ID, or an evicted old ID.
    RejectedShopReplay,
    /// A previously queued ward command; never enqueue or settle it again.
    DuplicateWard {
        effective_tick: u32,
    },
    /// Same ward ID with changed payload/tick, zero ID, or an ID retired with its epoch.
    RejectedWardReplay,
    Accepted {
        effective_tick: u32,
    },
    Retargeted {
        original_tick: u32,
        effective_tick: u32,
    },
    RejectedLate {
        original_tick: u32,
        current_tick: u32,
    },
}

#[derive(Clone, Debug)]
pub struct BufferedPlayerInput {
    pub input: PlayerInput,
    pub input_id: u32,
    pub server_receive_tick: u32,
    pub server_receive_instant: Instant,
}

impl BufferedPlayerInput {
    /// Wire correlation stays outside gameplay resources and scripts.
    pub fn acceptance_correlation(&self) -> u32 {
        self.input_id
    }
}

#[derive(Default)]
pub struct InputBuffer {
    /// target_tick→player_id→inputs plus wire metadata.
    /// 外部 BTreeMap，因此在刻度鍵上，drain_for_tick 的複雜度為 O(log N)。
    /// 內部BTreeMap由player_id作為確定性迭代順序的鍵控
    /// 在 TickBatch 組合中。
    by_tick: BTreeMap<u32, BTreeMap<u32, Vec<BufferedPlayerInput>>>,
    shop_requests: BTreeMap<u32, ShopRequestHistory>,
    ward_requests: BTreeMap<u32, WardRequestHistory>,
    last_seen_input_ids: BTreeMap<u32, u32>,
}

const SHOP_REQUEST_HISTORY_LIMIT: usize = 1024;
const WARD_COMMAND_HISTORY_LIMIT: usize = 256;

#[derive(Default)]
struct ShopRequestHistory {
    expired_through: u32,
    pending_receipts: usize,
    records: BTreeMap<u32, ShopRequestRecord>,
}

struct ShopRequestRecord {
    original_tick: u32,
    input: PlayerInput,
    result: InputSubmitResult,
    receipt: Option<omoba_core::runtime::shop_receipt::ShopReceipt>,
    undo_receipt: Option<Vec<u8>>,
}

#[derive(Default)]
struct WardRequestHistory {
    epoch: u64,
    expired_through: u32,
    records: BTreeMap<u32, WardRequestRecord>,
}

struct WardRequestRecord {
    original_tick: u32,
    input: PlayerInput,
    result: InputSubmitResult,
}

impl ShopRequestHistory {
    fn remember(&mut self, input_id: u32, record: ShopRequestRecord) {
        if matches!(
            record.result,
            InputSubmitResult::Accepted { .. } | InputSubmitResult::Retargeted { .. }
        ) {
            self.pending_receipts += 1;
        }
        self.records.insert(input_id, record);
        if self.records.len() > SHOP_REQUEST_HISTORY_LIMIT {
            let (expired_id, expired) = self.records.pop_first().expect("overfull shop history");
            if expired.receipt.is_none()
                && expired.undo_receipt.is_none()
                && matches!(
                    expired.result,
                    InputSubmitResult::Accepted { .. } | InputSubmitResult::Retargeted { .. }
                )
            {
                self.pending_receipts -= 1;
            }
            self.expired_through = self.expired_through.max(expired_id);
        }
    }
}

impl InputBuffer {
    pub fn has_pending_shop_receipts(&self) -> bool {
        self.shop_requests
            .values()
            .any(|history| history.pending_receipts != 0)
    }
    /// Only authority-projected receipts can finalize admitted transactions.
    pub fn remember_shop_receipt(
        &mut self,
        receipt: omoba_core::runtime::shop_receipt::ShopReceipt,
    ) -> bool {
        use crate::lockstep::PlayerInputEnum;
        use omoba_core::runtime::shop_receipt::ShopReceipt;
        if ShopReceipt::decode(&receipt.encode()).as_ref() != Some(&receipt) {
            return false;
        }
        let Ok(id) = u32::try_from(receipt.input_id) else {
            return false;
        };
        let Some(history) = self.shop_requests.get_mut(&receipt.player_id) else {
            return false;
        };
        let Some(record) = history.records.get_mut(&id) else {
            return false;
        };
        let tick = match record.result {
            InputSubmitResult::Accepted { effective_tick }
            | InputSubmitResult::Retargeted { effective_tick, .. } => effective_tick,
            _ => return false,
        };
        if receipt.tick != u64::from(tick) {
            return false;
        }
        let matches = match record.input.action.as_ref() {
            Some(PlayerInputEnum::ItemBuy(buy)) => {
                let catalog_id = omoba_template_ids::MOBA_ITEM_CATALOG
                    .iter()
                    .find(|item| item.id == buy.item_id)
                    .map_or(0, |item| u32::from(item.catalog_id));
                receipt.action_kind == 17 && receipt.catalog_id == catalog_id && receipt.slot == 0
            }
            Some(PlayerInputEnum::ItemSell(sell)) => {
                receipt.action_kind == 18 && receipt.slot == sell.item_slot
            }
            _ => false,
        };
        if !matches {
            return false;
        }
        if let Some(original) = &record.receipt {
            return original == &receipt;
        }
        record.receipt = Some(receipt);
        history.pending_receipts -= 1;
        true
    }

    pub fn remember_shop_undo_receipt(&mut self, bytes: &[u8]) -> bool {
        use crate::lockstep::PlayerInputEnum;
        use omoba_core::runtime::shop_undo_wire::{pinned_undo_request, ShopUndoReceipt, SHOP_UNDO_MODE_SEQUENCE};
        let Some(receipt) = ShopUndoReceipt::decode(bytes) else { return false };
        if ShopUndoReceipt::decode(&receipt.encode()).as_ref() != Some(&receipt) { return false; }
        let Ok(id) = u32::try_from(receipt.input_id) else { return false };
        let Some(history) = self.shop_requests.get_mut(&receipt.player_id) else { return false };
        let Some(record) = history.records.get_mut(&id) else { return false };
        let tick = match record.result {
            InputSubmitResult::Accepted { effective_tick } | InputSubmitResult::Retargeted { effective_tick, .. } => effective_tick,
            _ => return false,
        };
        if receipt.tick != u64::from(tick) { return false; }
        let Some(PlayerInputEnum::ShopUndo(undo)) = record.input.action.as_ref() else { return false };
        if undo.mode == SHOP_UNDO_MODE_SEQUENCE && pinned_undo_request(&record.input) != Some(omoba_core::runtime::shop::ShopUndoRequest::Sequence(receipt.sequence)) {
            return false;
        }
        if let Some(original) = &record.undo_receipt {
            return original.as_slice() == bytes;
        }
        record.undo_receipt = Some(bytes.to_vec());
        history.pending_receipts -= 1;
        true
    }

    pub fn shop_receipt_replay(
        &self,
        player_id: u32,
        request_id: u64,
        input_id: u32,
    ) -> omoba_core::game_proto::ShopReceiptReplay {
        let mut reply = omoba_core::game_proto::ShopReceiptReplay {
            schema_version: 1,
            request_id,
            player_id,
            input_id,
            status: 0,
            receipt: Vec::new(),
        };
        if player_id == 0 || request_id == 0 || input_id == 0 {
            return reply;
        }
        let Some(history) = self.shop_requests.get(&player_id) else {
            return reply;
        };
        if input_id <= history.expired_through {
            reply.status = 3;
            return reply;
        }
        if let Some(record) = history.records.get(&input_id) {
            if matches!(record.input.action, Some(crate::lockstep::PlayerInputEnum::ShopUndo(_))) {
                reply.status = 0;
                reply.receipt.clear();
                return reply;
            }
            if let Some(receipt) = &record.receipt {
                reply.status = 2;
                reply.receipt = receipt.encode();
            } else {
                reply.status = if matches!(record.result, InputSubmitResult::RejectedLate { .. }) {
                    4
                } else {
                    1
                };
            }
        }
        reply
    }

    /// Version-2 replay for an admitted undo. Buy/sell ids stay on [`Self::shop_receipt_replay`].
    pub fn shop_undo_receipt_replay(&self, player_id: u32, request_id: u64, input_id: u32) -> omoba_core::game_proto::ShopReceiptReplay {
        let mut reply = omoba_core::game_proto::ShopReceiptReplay {
            schema_version: 2, request_id, player_id, input_id, status: 0, receipt: Vec::new(),
        };
        if player_id == 0 || request_id == 0 || input_id == 0 { return reply; }
        let Some(history) = self.shop_requests.get(&player_id) else { return reply };
        if input_id <= history.expired_through {
            reply.status = 3;
            return reply;
        }
        let Some(record) = history.records.get(&input_id) else { return reply };
        if !matches!(record.input.action, Some(crate::lockstep::PlayerInputEnum::ShopUndo(_))) {
            return reply;
        }
        if let Some(bytes) = &record.undo_receipt {
            reply.status = 2;
            reply.receipt = bytes.clone();
        } else {
            reply.status = if matches!(record.result, InputSubmitResult::RejectedLate { .. }) { 4 } else { 1 };
        }
        reply
    }

    pub fn shop_command_replay(&self, player_id: u32, request_id: u64, input_id: u32, undo_capable: bool) -> omoba_core::game_proto::ShopReceiptReplay {
        let undo = undo_capable && self.shop_requests.get(&player_id).and_then(|history| history.records.get(&input_id))
            .is_some_and(|record| matches!(record.input.action, Some(crate::lockstep::PlayerInputEnum::ShopUndo(_))));
        if undo {
            self.shop_undo_receipt_replay(player_id, request_id, input_id)
        } else {
            self.shop_receipt_replay(player_id, request_id, input_id)
        }
    }

    /// Match-lifetime wire admission floor, independent of gameplay and drain.
    pub fn last_seen_input_id(&self, player_id: u32) -> u32 {
        self.last_seen_input_ids
            .get(&player_id)
            .copied()
            .unwrap_or(0)
    }

    pub fn new() -> Self {
        Self {
            by_tick: BTreeMap::new(),
            shop_requests: BTreeMap::new(),
            ward_requests: BTreeMap::new(),
            last_seen_input_ids: BTreeMap::new(),
        }
    }

    /// Drop one owner's ward admission journal when the view epoch changes.
    /// Retired IDs cannot re-enter. A later ID above the previous floor still can.
    pub fn observe_ward_epoch(&mut self, player_id: u32, epoch: u64) {
        let floor = self.last_seen_input_ids.get(&player_id).copied().unwrap_or(0);
        let history = self.ward_requests.entry(player_id).or_default();
        if history.epoch != epoch {
            history.records.clear();
            history.expired_through = history.expired_through.max(floor);
            history.epoch = epoch;
        }
    }

    fn remember_ward(
        &mut self,
        player_id: u32,
        input_id: u32,
        original_tick: u32,
        input: PlayerInput,
        result: InputSubmitResult,
    ) {
        let history = self.ward_requests.entry(player_id).or_default();
        history.records.insert(
            input_id,
            WardRequestRecord {
                original_tick,
                input,
                result,
            },
        );
        if history.records.len() > WARD_COMMAND_HISTORY_LIMIT {
            let (expired_id, _) = history.records.pop_first().expect("overfull ward history");
            history.expired_through = history.expired_through.max(expired_id);
        }
    }

    /// 提交一項輸入。若 target tick 已被耗盡，但仍在 grace window 內，
    /// server 會把它排到下一個 tick；太舊則拒收。
    /// 如果同一玩家在同一 tick 提交多次，會保留所有 input_id，並按抵達順序
    /// 發進 TickBatch。MoveTo 之類的狀態型命令仍由 runtime 的最後一筆決定
    /// 最終狀態，但前端不會因為 input id 被覆蓋而誤判 stale。
    pub fn submit(
        &mut self,
        current_tick: u32,
        player_id: u32,
        target_tick: u32,
        input: PlayerInput,
        input_id: u32,
    ) -> bool {
        matches!(
            self.submit_with_late_grace(current_tick, player_id, target_tick, input, input_id, 0),
            InputSubmitResult::Accepted { .. }
        )
    }

    pub fn submit_with_late_grace(
        &mut self,
        current_tick: u32,
        player_id: u32,
        target_tick: u32,
        input: PlayerInput,
        input_id: u32,
        late_grace_ticks: u32,
    ) -> InputSubmitResult {
        let is_undo = matches!(input.action, Some(crate::lockstep::PlayerInputEnum::ShopUndo(_)));
        if is_undo {
            let shape_ok = matches!(input.action.as_ref(), Some(crate::lockstep::PlayerInputEnum::ShopUndo(value))
                if (value.mode == omoba_core::runtime::shop_undo_wire::SHOP_UNDO_MODE_NEWEST && value.sequence == 0)
                    || (value.mode == omoba_core::runtime::shop_undo_wire::SHOP_UNDO_MODE_SEQUENCE && value.sequence != 0));
            if !shape_ok || player_id == 0 || input_id == 0 {
                return InputSubmitResult::RejectedShopReplay;
            }
        }
        let is_shop = is_undo || matches!(
            input.action,
            Some(crate::lockstep::PlayerInputEnum::ItemBuy(_))
                | Some(crate::lockstep::PlayerInputEnum::ItemSell(_))
        );
        if is_shop {
            if player_id == 0 || input_id == 0 {
                return InputSubmitResult::RejectedShopReplay;
            }
            // The journal must not retain unbounded client-authored strings.
            if matches!(input.action.as_ref(), Some(crate::lockstep::PlayerInputEnum::ItemBuy(value)) if value.item_id.len() > 128)
            {
                return InputSubmitResult::RejectedShopReplay;
            }
            let history = self.shop_requests.entry(player_id).or_default();
            if input_id <= history.expired_through {
                return InputSubmitResult::RejectedShopReplay;
            }
            if let Some(record) = history.records.get(&input_id) {
                if record.original_tick != target_tick || record.input != input {
                    return InputSubmitResult::RejectedShopReplay;
                }
                return match record.result {
                    InputSubmitResult::Accepted { effective_tick }
                    | InputSubmitResult::Retargeted { effective_tick, .. } => {
                        InputSubmitResult::DuplicateShop { effective_tick }
                    }
                    other => other,
                };
            }
        }
        let is_ward = matches!(
            input.action,
            Some(crate::lockstep::PlayerInputEnum::WardPlacement(_))
        );
        if is_ward {
            if player_id == 0 || input_id == 0 {
                return InputSubmitResult::RejectedWardReplay;
            }
            let history = self.ward_requests.entry(player_id).or_default();
            if input_id <= history.expired_through {
                return InputSubmitResult::RejectedWardReplay;
            }
            if let Some(record) = history.records.get(&input_id) {
                if record.original_tick != target_tick || record.input != input {
                    return InputSubmitResult::RejectedWardReplay;
                }
                return match record.result {
                    InputSubmitResult::Accepted { effective_tick }
                    | InputSubmitResult::Retargeted { effective_tick, .. } => {
                        InputSubmitResult::DuplicateWard { effective_tick }
                    }
                    other => other,
                };
            }
        }
        if player_id != 0 && input_id != 0 {
            let floor = self.last_seen_input_ids.entry(player_id).or_default();
            *floor = (*floor).max(input_id);
        }
        let effective_tick = if target_tick > current_tick {
            target_tick
        } else {
            let late_by = current_tick.saturating_sub(target_tick).saturating_add(1);
            if late_by > late_grace_ticks {
                let result = InputSubmitResult::RejectedLate {
                    original_tick: target_tick,
                    current_tick,
                };
                if is_shop {
                    self.shop_requests.get_mut(&player_id).unwrap().remember(
                        input_id,
                        ShopRequestRecord {
                            original_tick: target_tick,
                            input,
                            result,
                            receipt: None,
                            undo_receipt: None,
                        },
                    );
                } else if is_ward {
                    self.remember_ward(player_id, input_id, target_tick, input, result);
                }
                return result;
            }
            current_tick.saturating_add(1)
        };
        let result = if effective_tick == target_tick {
            InputSubmitResult::Accepted { effective_tick }
        } else {
            InputSubmitResult::Retargeted {
                original_tick: target_tick,
                effective_tick,
            }
        };
        if is_shop {
            self.shop_requests.get_mut(&player_id).unwrap().remember(
                input_id,
                ShopRequestRecord {
                    original_tick: target_tick,
                    input: input.clone(),
                    result,
                    receipt: None,
                    undo_receipt: None,
                },
            );
        }
        if is_ward {
            self.remember_ward(player_id, input_id, target_tick, input.clone(), result);
        }
        self.by_tick
            .entry(effective_tick)
            .or_insert_with(BTreeMap::new)
            .entry(player_id)
            .or_insert_with(Vec::new)
            .push(BufferedPlayerInput {
                input,
                input_id,
                server_receive_tick: current_tick,
                server_receive_instant: Instant::now(),
            });
        result
    }

    /// 耗盡針對此刻度的所有輸入。返回按player_id排序
    /// （BTreeMap 迭代是按關鍵順序進行的——確定性所必需的
    /// 所有對等點的 TickBatch 組合）。
    pub fn drain_for_tick(&mut self, tick: u32) -> Vec<(u32, BufferedPlayerInput)> {
        self.by_tick
            .remove(&tick)
            .map(|m| {
                m.into_iter()
                    .flat_map(|(player_id, inputs)| {
                        inputs.into_iter().map(move |input| (player_id, input))
                    })
                    .collect()
            })
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
        self.by_tick
            .values()
            .map(|m| m.values().map(Vec::len).sum::<usize>())
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generated_catalog_kernel_projection_and_recovery_never_charge_twice() {
        use omoba_core::runtime::{
            native::{
                comp::{Gold, Inventory},
                item::ItemRegistry,
            },
            shop::{buy_item, ShopCommand, ShopSettlement},
            shop_receipt::{project_shop_receipts, ShopReceipt},
            CanonicalAcceptedInput,
        };
        use prost::Message;
        let mut buffer = InputBuffer::new();
        let input = buy("moba_sword");
        assert!(buffer.submit(0, 7, 5, input.clone(), 42));
        let admitted = buffer.drain_for_tick(5);
        assert_eq!(admitted.len(), 1);
        // Unit fixture only; no test balance is granted to actual matches.
        let mut gold = Gold(1000);
        let mut inventory = Inventory::default();
        let result = buy_item(
            &ItemRegistry::generated_moba(),
            &mut inventory,
            &mut gold,
            "moba_sword",
        )
        .map(|_| ());
        assert!(result.is_ok());
        assert_eq!(gold.0, 650);
        let accepted = CanonicalAcceptedInput::from_authoritative_acceptance(
            1,
            7,
            42,
            17,
            99,
            None,
            input.encode_to_vec(),
        );
        let events = project_shop_receipts(
            1,
            5,
            &[accepted],
            &[ShopSettlement {
                player_id: 7,
                command: ShopCommand::Buy("moba_sword".into()),
                result,
            }],
        )
        .unwrap();
        let receipt = ShopReceipt::decode(&events[0].sanitized_payload).unwrap();
        assert_eq!(receipt.result_code, 0);
        assert!(buffer.remember_shop_receipt(receipt.clone()));
        for request in 1..=5 {
            assert_eq!(
                buffer.submit_with_late_grace(50, 7, 5, input.clone(), 42, 0),
                InputSubmitResult::DuplicateShop { effective_tick: 5 }
            );
            assert_eq!(
                buffer.shop_receipt_replay(7, request, 42).receipt,
                receipt.encode()
            );
            assert_eq!(buffer.pending_count(), 0);
            assert_eq!(gold.0, 650);
            assert_eq!(
                inventory.slots.iter().filter(|slot| slot.is_some()).count(),
                1
            );
        }
    }
    #[test]
    fn original_terminal_receipt_survives_drain_and_never_requeues_or_changes() {
        use omoba_core::runtime::shop_receipt::ShopReceipt;
        let mut buffer = InputBuffer::new();
        assert_eq!(buffer.shop_receipt_replay(7, 1, 42).status, 0);
        assert!(buffer.submit(0, 7, 5, buy("moba_sword"), 42));
        assert_eq!(buffer.shop_receipt_replay(7, 1, 42).status, 1);
        assert_eq!(buffer.drain_for_tick(5).len(), 1);
        let receipt = ShopReceipt {
            player_id: 7,
            input_id: 42,
            tick: 5,
            action_kind: 17,
            catalog_id: 1,
            slot: 0,
            result_code: 6,
        };
        assert!(buffer.has_pending_shop_receipts());
        let mut wrong = receipt.clone();
        wrong.tick = 6;
        assert!(!buffer.remember_shop_receipt(wrong));
        let mut wrong = receipt.clone();
        wrong.catalog_id = 2;
        assert!(!buffer.remember_shop_receipt(wrong));
        assert!(buffer.remember_shop_receipt(receipt.clone()));
        assert!(buffer.remember_shop_receipt(receipt.clone()));
        assert!(!buffer.has_pending_shop_receipts());
        let mut changed = receipt.clone();
        changed.result_code = 0;
        assert!(!buffer.remember_shop_receipt(changed));
        assert_eq!(
            buffer.submit_with_late_grace(50, 7, 5, buy("moba_sword"), 42, 0),
            InputSubmitResult::DuplicateShop { effective_tick: 5 }
        );
        assert_eq!(buffer.pending_count(), 0);
        for request in [1, 2, 3] {
            let replay = buffer.shop_receipt_replay(7, request, 42);
            assert_eq!(replay.request_id, request);
            assert_eq!(replay.status, 2);
            assert_eq!(replay.receipt, receipt.encode());
        }
        assert_eq!(buffer.shop_receipt_replay(2, 1, 42).status, 0);
        assert!(!buffer.submit(50, 7, 1, buy("moba_sword"), 43));
        assert_eq!(buffer.shop_receipt_replay(7, 1, 43).status, 4);
        for id in 44..=(44 + SHOP_REQUEST_HISTORY_LIMIT as u32) {
            assert!(buffer.submit(50, 7, 60, buy("moba_sword"), id));
        }
        assert_eq!(buffer.shop_receipt_replay(7, 1, 42).status, 3);
        assert!(!buffer.remember_shop_receipt(receipt));
    }

    #[test]
    fn sell_receipt_requires_original_slot_player_and_retarget_tick() {
        use omoba_core::runtime::shop_receipt::ShopReceipt;
        let mut buffer = InputBuffer::new();
        let sell = PlayerInput {
            action: Some(PlayerInputEnum::ItemSell(
                omoba_core::game_proto::ItemSell { item_slot: 2 },
            )),
        };
        assert!(matches!(
            buffer.submit_with_late_grace(5, 7, 5, sell, 9, 1),
            InputSubmitResult::Retargeted {
                effective_tick: 6,
                ..
            }
        ));
        let receipt = ShopReceipt {
            player_id: 7,
            input_id: 9,
            tick: 6,
            action_kind: 18,
            catalog_id: 0,
            slot: 2,
            result_code: 0,
        };
        let mut wrong = receipt.clone();
        wrong.slot = 1;
        assert!(!buffer.remember_shop_receipt(wrong));
        let mut wrong = receipt.clone();
        wrong.player_id = 2;
        assert!(!buffer.remember_shop_receipt(wrong));
        assert!(buffer.remember_shop_receipt(receipt.clone()));
        assert_eq!(
            buffer.shop_receipt_replay(7, 10, 9).receipt,
            receipt.encode()
        );
    }
    #[test]
    fn admission_floor_survives_drain_eviction_and_rejected_late_inputs() {
        let mut buffer = InputBuffer::new();
        assert_eq!(buffer.last_seen_input_id(7), 0);
        assert!(buffer.submit(0, 7, 1, noop(), 42));
        assert!(buffer.submit(0, 7, 1, noop(), 3));
        assert!(buffer.submit(0, 2, 1, noop(), 8));
        buffer.drain_for_tick(1);
        buffer.evict_older(100);
        assert_eq!(buffer.last_seen_input_id(7), 42);
        assert_eq!(buffer.last_seen_input_id(2), 8);
        assert!(!buffer.submit(100, 7, 1, noop(), 99));
        assert_eq!(buffer.last_seen_input_id(7), 99);
        assert!(buffer.submit(100, 7, 101, noop(), u32::MAX));
        assert_eq!(buffer.last_seen_input_id(7), u32::MAX);
        assert_eq!(buffer.last_seen_input_id(2), 8);
        assert!(buffer.submit(100, 0, 101, noop(), 12));
        assert_eq!(buffer.last_seen_input_id(0), 0);
    }
    use crate::lockstep::{NoOp, PlayerInputEnum};

    fn noop() -> PlayerInput {
        PlayerInput {
            action: Some(PlayerInputEnum::NoOp(NoOp {})),
        }
    }

    fn buy(id: &str) -> PlayerInput {
        PlayerInput {
            action: Some(PlayerInputEnum::ItemBuy(omoba_core::game_proto::ItemBuy {
                item_id: id.into(),
            })),
        }
    }

    #[test]
    fn shop_retry_is_not_requeued_before_or_after_drain() {
        let mut buffer = InputBuffer::new();
        assert!(buffer.submit(0, 7, 5, buy("moba_sword"), 41));
        assert_eq!(
            buffer.submit_with_late_grace(1, 7, 5, buy("moba_sword"), 41, 2),
            InputSubmitResult::DuplicateShop { effective_tick: 5 }
        );
        assert_eq!(buffer.pending_count(), 1);
        assert_eq!(buffer.drain_for_tick(5).len(), 1);
        assert_eq!(
            buffer.submit_with_late_grace(100, 7, 5, buy("moba_sword"), 41, 2),
            InputSubmitResult::DuplicateShop { effective_tick: 5 }
        );
        assert_eq!(buffer.pending_count(), 0);
        assert!(buffer.submit(100, 2, 101, buy("moba_sword"), 41));
        assert_eq!(
            buffer.drain_for_tick(101).len(),
            1,
            "IDs are player-scoped, not team-scoped"
        );
    }

    #[test]
    fn shop_retry_rejects_mutation_zero_ids_and_keeps_original_late_result() {
        let mut buffer = InputBuffer::new();
        assert_eq!(
            buffer.submit_with_late_grace(10, 7, 9, buy("moba_sword"), 1, 2),
            InputSubmitResult::Retargeted {
                original_tick: 9,
                effective_tick: 11
            }
        );
        assert_eq!(
            buffer.submit_with_late_grace(20, 7, 9, buy("moba_sword"), 1, 0),
            InputSubmitResult::DuplicateShop { effective_tick: 11 }
        );
        for (player, tick, input, id) in [
            (7, 12, buy("moba_sword"), 1),
            (7, 9, buy("moba_armor"), 1),
            (7, 12, buy("moba_sword"), 0),
            (0, 12, buy("moba_sword"), 2),
        ] {
            assert_eq!(
                buffer.submit_with_late_grace(10, player, tick, input, id, 2),
                InputSubmitResult::RejectedShopReplay
            );
        }
        let rejected = buffer.submit_with_late_grace(10, 7, 1, buy("moba_sword"), 2, 0);
        assert!(matches!(rejected, InputSubmitResult::RejectedLate { .. }));
        assert_eq!(
            buffer.submit_with_late_grace(20, 7, 1, buy("moba_sword"), 2, 100),
            rejected
        );
        assert_eq!(buffer.drain_for_tick(11).len(), 1);
    }

    #[test]
    fn shop_history_is_bounded_and_eviction_cannot_reenable_old_transactions() {
        let mut buffer = InputBuffer::new();
        for id in 1..=SHOP_REQUEST_HISTORY_LIMIT as u32 + 1 {
            assert!(buffer.submit(0, 7, 1, buy("moba_sword"), id));
        }
        assert_eq!(
            buffer.shop_requests[&7].records.len(),
            SHOP_REQUEST_HISTORY_LIMIT
        );
        buffer.evict_older(2);
        assert_eq!(
            buffer.submit_with_late_grace(2, 7, 1, buy("moba_sword"), 1, 3),
            InputSubmitResult::RejectedShopReplay
        );
        assert_eq!(buffer.pending_count(), 0);
        assert_eq!(
            buffer.submit_with_late_grace(2, 7, 1, buy("moba_sword"), 2, 3),
            InputSubmitResult::DuplicateShop { effective_tick: 1 }
        );
    }

    #[test]
    fn distinct_shop_requests_keep_arrival_order_and_sell_retry_is_deduplicated() {
        let mut buffer = InputBuffer::new();
        let sell = PlayerInput {
            action: Some(PlayerInputEnum::ItemSell(
                omoba_core::game_proto::ItemSell { item_slot: 2 },
            )),
        };
        assert!(buffer.submit(0, 7, 5, buy("moba_sword"), 8));
        assert!(buffer.submit(0, 7, 5, buy("moba_armor"), 7));
        assert!(buffer.submit(0, 7, 5, sell.clone(), 9));
        assert_eq!(
            buffer.submit_with_late_grace(0, 7, 5, sell, 9, 0),
            InputSubmitResult::DuplicateShop { effective_tick: 5 }
        );
        assert_eq!(
            buffer
                .drain_for_tick(5)
                .into_iter()
                .map(|(_, input)| input.input_id)
                .collect::<Vec<_>>(),
            vec![8, 7, 9]
        );
    }

    #[test]
    fn oversized_shop_payload_does_not_allocate_history_or_queue() {
        let mut buffer = InputBuffer::new();
        assert_eq!(
            buffer.submit_with_late_grace(0, 7, 1, buy(&"x".repeat(129)), 1, 0),
            InputSubmitResult::RejectedShopReplay
        );
        assert!(buffer.shop_requests.is_empty());
        assert_eq!(buffer.pending_count(), 0);
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
    fn late_input_within_grace_retargets_to_next_tick() {
        let mut b = InputBuffer::new();
        let result = b.submit_with_late_grace(10, 1, 9, noop(), 7, 2);

        assert_eq!(
            result,
            InputSubmitResult::Retargeted {
                original_tick: 9,
                effective_tick: 11
            }
        );
        assert!(b.drain_for_tick(9).is_empty());
        let drained = b.drain_for_tick(11);
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].1.input_id, 7);
        assert_eq!(drained[0].1.server_receive_tick, 10);
    }

    #[test]
    fn late_input_beyond_grace_rejected() {
        let mut b = InputBuffer::new();
        let result = b.submit_with_late_grace(10, 1, 8, noop(), 7, 2);

        assert_eq!(
            result,
            InputSubmitResult::RejectedLate {
                original_tick: 8,
                current_tick: 10
            }
        );
        assert_eq!(b.pending_count(), 0);
    }

    #[test]
    fn same_player_same_tick_preserves_input_ids_in_order() {
        let mut b = InputBuffer::new();
        assert!(b.submit(0, 1, 5, noop(), 41));
        assert!(b.submit(0, 1, 5, noop(), 42));
        assert_eq!(b.pending_count(), 2);

        let drained = b.drain_for_tick(5);
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].0, 1);
        assert_eq!(drained[1].0, 1);
        assert_eq!(drained[0].1.input_id, 41);
        assert_eq!(drained[1].1.input_id, 42);
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

    fn undo_newest() -> PlayerInput {
        PlayerInput {
            action: Some(PlayerInputEnum::ShopUndo(omoba_core::game_proto::ShopUndo {
                mode: omoba_core::runtime::shop_undo_wire::SHOP_UNDO_MODE_NEWEST,
                sequence: 0,
            })),
        }
    }

    #[test]
    fn shop_undo_protocol_duplicate_newest_is_not_requeued() {
        let mut buffer = InputBuffer::new();
        let input = undo_newest();
        assert!(buffer.submit(0, 7, 5, input.clone(), 11));
        assert_eq!(
            buffer.submit_with_late_grace(1, 7, 5, input.clone(), 11, 2),
            InputSubmitResult::DuplicateShop { effective_tick: 5 }
        );
        assert_eq!(buffer.pending_count(), 1);
        assert_eq!(buffer.drain_for_tick(5).len(), 1);
        assert_eq!(
            buffer.submit_with_late_grace(20, 7, 5, input, 11, 0),
            InputSubmitResult::DuplicateShop { effective_tick: 5 }
        );
        assert_eq!(buffer.pending_count(), 0);
        let replay = buffer.shop_receipt_replay(7, 1, 11);
        assert_eq!(replay.schema_version, 1);
        assert_eq!(replay.status, 0);
        assert!(replay.receipt.is_empty());
        assert_eq!(
            buffer.submit_with_late_grace(20, 7, 6, PlayerInput {
                action: Some(PlayerInputEnum::ShopUndo(omoba_core::game_proto::ShopUndo { mode: 1, sequence: 4 })),
            }, 12, 0),
            InputSubmitResult::RejectedShopReplay
        );
        assert_eq!(buffer.submit_with_late_grace(20, 7, 6, undo_newest(), 0, 0), InputSubmitResult::RejectedShopReplay);
        assert_eq!(buffer.pending_count(), 0);
        assert!(buffer.submit(20, 7, 21, buy("moba_sword"), 30));
        assert_eq!(buffer.pending_count(), 1);
        assert_eq!(buffer.shop_receipt_replay(7, 1, 30).status, 1);
        assert_eq!(buffer.shop_command_replay(7, 1, 11, true).status, 1);
        assert_eq!(buffer.shop_command_replay(7, 1, 11, false).status, 0);
    }

    #[test]
    fn shop_undo_protocol_schema2_replay_keeps_canonical_economy() {
        use omoba_core::runtime::shop_undo_wire::ShopUndoReceipt;
        let mut buffer = InputBuffer::new();
        assert!(buffer.submit(0, 7, 5, undo_newest(), 11));
        let receipt = ShopUndoReceipt {
            player_id: 7,
            input_id: 11,
            tick: 5,
            sequence: 1,
            result_code: 0,
            gold: 650,
            catalogs: [0; 6],
            cooldowns: [0; 6],
        };
        let bytes = receipt.encode();
        assert!(buffer.remember_shop_undo_receipt(&bytes));
        assert!(buffer.remember_shop_undo_receipt(&bytes));
        let mut changed = receipt.clone();
        changed.gold = 651;
        assert!(!buffer.remember_shop_undo_receipt(&changed.encode()));
        let replay = buffer.shop_command_replay(7, 4, 11, true);
        assert_eq!(replay.schema_version, 2);
        assert_eq!(replay.status, 2);
        assert_eq!(ShopUndoReceipt::decode(&replay.receipt).unwrap().gold, 650);
        assert_eq!(ShopUndoReceipt::decode(&replay.receipt).unwrap().result_code, 0);
        let hidden = buffer.shop_command_replay(7, 4, 11, false);
        assert_eq!(hidden.schema_version, 1);
        assert_eq!(hidden.status, 0);
        assert!(hidden.receipt.is_empty());
        assert_eq!(buffer.shop_receipt_replay(7, 4, 11).status, 0);
    }

    #[test]
    fn ward_submit_reuses_authenticated_identity_and_rejects_duplicate_conflict_and_expired() {
        use crate::lockstep::PlayerInputEnum;
        let place = |x: i64| PlayerInput {
            action: Some(PlayerInputEnum::WardPlacement(omoba_core::game_proto::WardPlacement {
                slot: 0,
                x_raw: Some(x),
                y_raw: Some(1),
            })),
        };
        let mut buffer = InputBuffer::new();
        assert_eq!(
            buffer.submit_with_late_grace(0, 0, 3, place(1), 81, 0),
            InputSubmitResult::RejectedWardReplay
        );
        assert_eq!(
            buffer.submit_with_late_grace(0, 7, 3, place(1), 0, 0),
            InputSubmitResult::RejectedWardReplay
        );
        assert_eq!(buffer.pending_count(), 0);
        assert!(matches!(
            buffer.submit_with_late_grace(0, 7, 3, place(4), 1, 0),
            InputSubmitResult::Accepted { effective_tick: 3 }
        ));
        assert_eq!(buffer.pending_count(), 1);
        assert_eq!(buffer.drain_for_tick(3)[0].1.acceptance_correlation(), 1);
        assert_eq!(
            buffer.submit_with_late_grace(4, 7, 3, place(4), 1, 0),
            InputSubmitResult::DuplicateWard { effective_tick: 3 }
        );
        assert_eq!(
            buffer.submit_with_late_grace(4, 7, 3, place(9), 1, 0),
            InputSubmitResult::RejectedWardReplay
        );
        assert_eq!(buffer.pending_count(), 0);
        for id in 2..=(WARD_COMMAND_HISTORY_LIMIT as u32 + 1) {
            assert!(buffer.submit(4, 7, 8, place(i64::from(id)), id));
        }
        assert_eq!(
            buffer.submit_with_late_grace(9, 7, 8, place(4), 1, 0),
            InputSubmitResult::RejectedWardReplay
        );
        buffer.drain_for_tick(8);
        assert_eq!(buffer.pending_count(), 0);
        let retired = WARD_COMMAND_HISTORY_LIMIT as u32 + 1;
        buffer.observe_ward_epoch(7, 2);
        buffer.observe_ward_epoch(7, 2);
        assert_eq!(
            buffer.submit_with_late_grace(9, 7, 10, place(4), retired, 0),
            InputSubmitResult::RejectedWardReplay
        );
        let fresh = retired + 1;
        assert!(matches!(
            buffer.submit_with_late_grace(9, 7, 10, place(4), fresh, 0),
            InputSubmitResult::Accepted { effective_tick: 10 }
        ));
        assert_eq!(buffer.pending_count(), 1);
    }
}
