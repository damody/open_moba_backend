//! 階段 3.4：每次調度程式勾選時都會耗盡「PendingPlayerInputs」。
//!
//! 鎖步線（或 omfx sim_runner）寫入新的地圖
//! 每個 TickBatch 上的資源中都包含「player_id → PlayerInput」。這
//! 系統消耗它們（清除資源，這樣過時的輸入就不會
//! 累積）並將每個變體路由到適當的遊戲端
//! 處理程序。
//!
//! 階段 3.4 路由有意成為一個存根：每個變體都記錄在
//! `trace` 或以其他方式丟棄。第四階段將：
//! - `MoveTo` → 在玩家的英雄實體上寫入`MoveTarget`。
//! - `CastAbility` → 透過以下方式將能力腳本呼叫排入佇列
//! 現有的“scripting::dispatch”邊界。
//!   - `TowerPlace` /
//! `塔升級*` /
//! `TowerSell` → 透過 `comp::game_processor::GameProcessor` 路由
//! 它已經實現了每個 TD 命令。
//! - `ItemUse` → 透過庫存效果管道進行路由。
//!
//! 在第 3.4 階段（使用存根主體）處理消費者的要點是
//! 調度程序總是耗盡資源，因此 kcp 鎖步線路測試
//! 在階段 3.5 中沒有看到洩漏。

use specs::{Read, Write};

use crate::comp::ecs::{Job, System};
use crate::comp::{CurrentCreepWave, PendingPlayerInputs, Time};
#[cfg(feature = "kcp")]
use crate::comp::{PendingItemUseQueue, PendingMoveQueue, PendingTowerSellQueue, PendingTowerSpawnQueue, PendingTowerUpgradeQueue};

#[derive(Default)]
pub struct Sys;

#[cfg(feature = "kcp")]
impl<'a> System<'a> for Sys {
    type SystemData = (
        Write<'a, PendingPlayerInputs>,
        Write<'a, CurrentCreepWave>,
        Read<'a, Time>,
        Write<'a, PendingTowerSpawnQueue>,
        Write<'a, PendingTowerSellQueue>,
        Write<'a, PendingTowerUpgradeQueue>,
        Write<'a, PendingItemUseQueue>,
        Write<'a, PendingMoveQueue>,
    );

    const NAME: &'static str = "player_input";

    fn run(
        _job: &mut Job<Self>,
        (mut pending, mut cw, time, mut tower_q, mut sell_q, mut upgrade_q, mut item_q, mut move_q): Self::SystemData,
    ) {
        if pending.by_player.is_empty() {
            return;
        }
        let target_tick = pending.tick;
        let totaltime = time.0 as f32;
        let drained: Vec<_> = pending.by_player.drain().collect();
        log::trace!(
            "player_input_tick: draining {} inputs for tick {}",
            drained.len(),
            target_tick
        );
        for (player_id, input) in drained {
            route_input(
                player_id,
                target_tick,
                input,
                &mut cw,
                totaltime,
                &mut tower_q,
                &mut sell_q,
                &mut upgrade_q,
                &mut item_q,
                &mut move_q,
            );
        }
    }
}

#[cfg(not(feature = "kcp"))]
impl<'a> System<'a> for Sys {
    // Non-kcp builds have an empty marker resource;沒有什麼可以排出的。
    type SystemData = specs::Read<'a, PendingPlayerInputs>;

    const NAME: &'static str = "player_input";

    fn run(_job: &mut Job<Self>, _: Self::SystemData) {}
}

#[cfg(feature = "kcp")]
fn route_input(
    player_id: u32,
    tick: u32,
    input: crate::lockstep::PlayerInput,
    cw: &mut CurrentCreepWave,
    totaltime: f32,
    tower_q: &mut PendingTowerSpawnQueue,
    sell_q: &mut PendingTowerSellQueue,
    upgrade_q: &mut PendingTowerUpgradeQueue,
    item_q: &mut PendingItemUseQueue,
    move_q: &mut PendingMoveQueue,
) {
    use crate::lockstep::PlayerInputEnum;

    match input.action {
        Some(PlayerInputEnum::StartRound(_)) => {
            // TD：客戶按下「開始回合」。翻轉 is_running 所以 cree_wave_tick
            // 開始發射下一波。 wave_start_time 每個蠕動錨點
            // 回合開始時出現延誤。
            if !cw.is_running {
                cw.is_running = true;
                cw.wave_start_time = totaltime;
                log::info!(
                    "player_input_tick: pid={} tick={} StartRound → wave={} start_time={:.2}",
                    player_id, tick, cw.wave, totaltime,
                );
            } else {
                log::warn!(
                    "player_input_tick: pid={} tick={} StartRound ignored (round already running)",
                    player_id, tick,
                );
            }
        }
        Some(PlayerInputEnum::NoOp(_)) => {
            // 僅確認 - 保持活動心跳，沒有副作用。
        }
        Some(PlayerInputEnum::MoveTo(m)) => {
            let (x, y) = m.target.map(|v| (v.x, v.y)).unwrap_or((0, 0));
            log::info!(
                "player_input_tick: pid={} tick={} MoveTo target_raw=({}, {})",
                player_id, tick, x, y,
            );
            // 遵循 PendingMoveQueue：將 MoveTarget 寫入玩家的佇列中
            // 英雄需要加入系統儲存的（英雄、派系）
            // 已經可以了，但是我們保持隊列模式的對稱性
            // 與 TowerPlace/Sell/Upgrade/ItemUse — 透過
            // 雙方調度後的“GameProcessor::drain_pending_moves”
            // 主機和副本。
            let pos = omoba_sim::Vec2::new(
                omoba_sim::Fixed64::from_raw(x as i64),
                omoba_sim::Fixed64::from_raw(y as i64),
            );
            move_q.requests.push(crate::comp::PendingMoveTo {
                pos,
                owner_pid: player_id,
            });
        }
        Some(PlayerInputEnum::AttackTarget(a)) => {
            log::trace!(
                "player_input_tick: pid={} tick={} AttackTarget target_id={}",
                player_id,
                tick,
                a.target_id
            );
        }
        Some(PlayerInputEnum::CastAbility(c)) => {
            log::trace!(
                "player_input_tick: pid={} tick={} CastAbility ability_index={} target_entity={:?}",
                player_id,
                tick,
                c.ability_index,
                c.target_entity
            );
            // 第 4 階段：使用玩家英雄編寫腳本::dispatch
            // 實體作為施法者。
        }
        Some(PlayerInputEnum::TowerPlace(t)) => {
            let pos_raw = t.pos.as_ref();
            let (px, py) = pos_raw.map(|v| (v.x, v.y)).unwrap_or((0, 0));
            log::info!(
                "player_input_tick: pid={} tick={} TowerPlace kind_id={} pos_raw=({}, {})",
                player_id, tick, t.tower_kind_id, px, py,
            );
            // 遵循 PendingTowerSpawnQueue：spawn_td_tower 需要 &mut World
            // （TowerTemplateRegistry 尋找 + 實體建立 + ScriptEvent::
            // 產生推送），這是“系統”規格無法借用的。隊列是
            // 在主機和副本上調度後立即耗盡
            // `GameProcessor::drain_pending_tower_spawns`。
            let pos = omoba_sim::Vec2::new(
                omoba_sim::Fixed64::from_raw(px as i64),
                omoba_sim::Fixed64::from_raw(py as i64),
            );
            tower_q.requests.push(crate::comp::PendingTowerSpawn {
                kind_id: t.tower_kind_id,
                pos,
                owner_pid: player_id,
            });
        }
        Some(PlayerInputEnum::TowerUpgrade(u)) => {
            log::info!(
                "player_input_tick: pid={} tick={} TowerUpgrade eid={} path={} level={}",
                player_id, tick, u.tower_entity_id, u.path, u.level,
            );
            // 遵循 PendingTowerUpgradeQueue：規則驗證 + Gold
            // 扣除 + Tower.upgrade_levels 寫入 + BuffStore 添加
            // StatMod 效果都需要“&mut World”，其中一個規格為“System”
            // 借不到。在主機和主機上發送後立即排空
            // 透過“GameProcessor::drain_pending_tower_upgrades”複製。
            //
            // `level` 被處理程序視為提示 - 實際的
            // 目標液位是根據塔的當前電流計算的
            // `upgrade_levels[path] + 1` 所以一個過時的客戶端（一個沒有
            // 但仍透過快照觀察到實體的upgrade_levels）
            // 產生正確的結果。
            upgrade_q.requests.push(crate::comp::PendingTowerUpgrade {
                tower_entity_id: u.tower_entity_id,
                path: u.path as u8,
                level: u.level as u8,
                owner_pid: player_id,
            });
        }
        Some(PlayerInputEnum::TowerSell(s)) => {
            log::info!(
                "player_input_tick: pid={} tick={} TowerSell tower_entity_id={}",
                player_id, tick, s.tower_entity_id,
            );
            // 遵循 PendingTowerSellQueue：退款 + 實體刪除 + buff
            // 清理所有需要 `&mut World` （閱讀 TowerTemplateRegistry +
            // TowerUpgradeRegistry，寫入Gold存儲，寫入BuffStore，
            // 刪除實體），這是「系統」規範無法借用的。瀝乾
            // 在主機和副本上分派後立即透過
            // `GameProcessor::drain_pending_tower_sells`。
            sell_q.requests.push(crate::comp::PendingTowerSell {
                tower_entity_id: s.tower_entity_id,
                owner_pid: player_id,
            });
        }
        Some(PlayerInputEnum::ItemUse(i)) => {
            log::info!(
                "player_input_tick: pid={} tick={} ItemUse slot={} target_entity={:?}",
                player_id, tick, i.item_slot, i.target_entity,
            );
            // 遵循 PendingItemUseQueue：ItemRegistry 讀取 + Inventory
            // write + CProperty (HP / msd) 寫入所有需要的`&mut World`。
            // 透過在主機和副本上調度後立即耗盡
            // `GameProcessor::drain_pending_item_uses`。
            let target_pos = i.target_pos.as_ref().map(|v| {
                omoba_sim::Vec2::new(
                    omoba_sim::Fixed64::from_raw(v.x as i64),
                    omoba_sim::Fixed64::from_raw(v.y as i64),
                )
            });
            item_q.requests.push(crate::comp::PendingItemUse {
                item_slot: i.item_slot,
                target_pos,
                target_entity: i.target_entity,
                owner_pid: player_id,
            });
        }
        None => {
            log::warn!(
                "player_input_tick: pid={} tick={} input action is None (malformed proto?)",
                player_id,
                tick
            );
        }
    }
}
