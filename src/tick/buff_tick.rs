//! Buff tick：每 tick 呼叫 `BuffStore::tick(dt)` 倒數、移除過期項。
//!
//! 取代舊 `slow_buff_tick`；所有 buff 統一走 `ability_runtime::BuffStore`。
//! 過期 buff 若 payload 含 `move_speed_bonus` 且 target 還活著且是 Creep →
//! 廣播 `creep/S { id, move_speed }` 讓 client 重算 lerp（buff_id 不再限定
//! "slow"，因為現在 slow buff_id 是 `slow_{attacker_id}` 多 instance）。

use crossbeam_channel::Sender;
use serde_json::json;
use specs::{shred, Entities, Read, ReadStorage, SystemData, Write, World};

use crate::ability_runtime::BuffStore;
use crate::comp::*;
use crate::scripting::{ScriptEvent, ScriptEventQueue};
use crate::transport::OutboundMsg;

#[derive(SystemData)]
pub struct BuffTickData<'a> {
    _entities: Entities<'a>,
    dt: Read<'a, DeltaTime>,
    buffs: Write<'a, BuffStore>,
    creeps: ReadStorage<'a, Creep>,
    cpropertys: ReadStorage<'a, CProperty>,
    mqtx: Write<'a, Vec<Sender<OutboundMsg>>>,
    script_events: Write<'a, ScriptEventQueue>,
}

#[derive(Default)]
pub struct Sys;

impl<'a> System<'a> for Sys {
    type SystemData = BuffTickData<'a>;

    const NAME: &'static str = "buff";

    fn run(_job: &mut Job<Self>, mut data: Self::SystemData) {
        let dt = data.dt.0;
        let expired = data.buffs.tick(dt);
        let tx = data.mqtx.get(0).cloned();

        for (entity, _buff_id, payload) in expired {
            // 每條過期 buff push ModifierRemoved 事件，讓腳本的 on_modifier_removed
            // 能 hook 到（例：某 stacking debuff 過期時補一個 refresh buff）。
            data.script_events.push(ScriptEvent::ModifierRemoved {
                e: entity,
                modifier_id: _buff_id.clone(),
            });

            // 含 move_speed_bonus 的 buff 過期 → 對 creep 發 creep/S，重算 effective
            // （此時 sum_add 已不含過期那筆 → 自然還原或剩餘疊加）
            if payload.get("move_speed_bonus").is_some() {
                let is_creep = data.creeps.get(entity).is_some();
                if is_creep {
                    if let (Some(ref t), Some(cp)) = (&tx, data.cpropertys.get(entity)) {
                        let sum = data.buffs.sum_add(entity, "move_speed_bonus");
                        let effective = cp.msd * (1.0 + sum).clamp(0.01, 1.0);
                        let _ = t.try_send(OutboundMsg::new_s(
                            "td/all/res",
                            "creep",
                            "S",
                            json!({ "id": entity.id(), "move_speed": effective }),
                        ));
                    }
                }
            }
        }
    }
}
