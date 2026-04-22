//! SlowBuff tick：每 tick 扣 remaining，歸零時移除 Component。
//!
//! 由 Ice 塔 projectile 命中時透過 `Outcome::ApplySlow` 加上 SlowBuff，
//! 本 system 負責倒數。creep_tick 讀 SlowBuff 套用 msd 乘數。

use specs::{shred, Entities, Join, Read, ReadStorage, SystemData, Write, WriteStorage, World};
use crossbeam_channel::Sender;
use serde_json::json;
use crate::comp::*;
use crate::transport::OutboundMsg;

#[derive(SystemData)]
pub struct SlowBuffData<'a> {
    entities: Entities<'a>,
    dt: Read<'a, DeltaTime>,
    slow_buffs: WriteStorage<'a, SlowBuff>,
    cpropertys: ReadStorage<'a, CProperty>,
    mqtx: Write<'a, Vec<Sender<OutboundMsg>>>,
}

#[derive(Default)]
pub struct Sys;

impl<'a> System<'a> for Sys {
    type SystemData = SlowBuffData<'a>;

    const NAME: &'static str = "slow_buff";

    fn run(_job: &mut Job<Self>, mut data: Self::SystemData) {
        let dt = data.dt.0;
        let mut to_remove: Vec<specs::Entity> = Vec::new();
        for (e, buff) in (&data.entities, &mut data.slow_buffs).join() {
            buff.remaining -= dt;
            if buff.remaining <= 0.0 {
                to_remove.push(e);
            }
        }
        let tx = data.mqtx.get(0).cloned();
        for e in to_remove {
            data.slow_buffs.remove(e);
            // 廣播 creep/S 還原為原速度
            if let (Some(ref t), Some(cp)) = (&tx, data.cpropertys.get(e)) {
                let _ = t.try_send(OutboundMsg::new_s("td/all/res", "creep", "S", json!({
                    "id": e.id(),
                    "move_speed": cp.msd,
                })));
            }
        }
    }
}
