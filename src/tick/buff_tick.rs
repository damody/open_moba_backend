//! Buff tick：每 tick 呼叫 `BuffStore::tick(dt)` 倒數、移除過期項。
//!
//! 取代舊 `slow_buff_tick`；`SlowBuff` component 已廢除，所有 buff 統一
//! 走 `ability_runtime::BuffStore` resource。過期 buff 若需廣播給 client
//! （例如 slow 解除時要回復移速），此處順便發訊息。

use crossbeam_channel::Sender;
use serde_json::json;
use specs::{shred, Entities, Read, ReadStorage, SystemData, Write, World};

use crate::ability_runtime::BuffStore;
use crate::comp::*;
use crate::transport::OutboundMsg;

#[derive(SystemData)]
pub struct BuffTickData<'a> {
    _entities: Entities<'a>,
    dt: Read<'a, DeltaTime>,
    buffs: Write<'a, BuffStore>,
    cpropertys: ReadStorage<'a, CProperty>,
    mqtx: Write<'a, Vec<Sender<OutboundMsg>>>,
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

        for (entity, buff_id) in expired {
            // 特定 buff 過期需要 client 端還原（例：slow 解除 → 廣播原速）
            if buff_id == "slow" {
                if let (Some(ref t), Some(cp)) = (&tx, data.cpropertys.get(entity)) {
                    let _ = t.try_send(OutboundMsg::new_s(
                        "td/all/res",
                        "creep",
                        "S",
                        json!({ "id": entity.id(), "move_speed": cp.msd }),
                    ));
                }
            }
        }
    }
}
