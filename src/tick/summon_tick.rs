//! Summon tick：倒數 `SummonedUnit.time_remaining`，歸零時 despawn。
//!
//! 召喚 AI（追敵、攻擊）暫由現有 `hero_tick` / `creep_tick` 模式之外獨立
//! 運作；初版先讓召喚物保持靜止 + 時效到自動移除，真實行為留待後續 phase。
//! 廣播 `unit/D` 給前端清除渲染。

use crossbeam_channel::Sender;
use serde_json::json;
use specs::{shred, Entities, Join, Read, SystemData, World, Write, WriteStorage};

use crate::comp::*;
use crate::transport::OutboundMsg;

#[derive(SystemData)]
pub struct SummonTickData<'a> {
    entities: Entities<'a>,
    dt: Read<'a, DeltaTime>,
    summoned: WriteStorage<'a, SummonedUnit>,
    mqtx: Write<'a, Vec<Sender<OutboundMsg>>>,
}

#[derive(Default)]
pub struct Sys;

impl<'a> System<'a> for Sys {
    type SystemData = SummonTickData<'a>;

    const NAME: &'static str = "summon";

    fn run(_job: &mut Job<Self>, mut data: Self::SystemData) {
        let dt = data.dt.0;
        let mut expired = Vec::new();
        for (e, s) in (&data.entities, &mut data.summoned).join() {
            if s.update(dt) {
                expired.push(e);
            }
        }

        let tx = data.mqtx.get(0).cloned();
        for e in expired {
            if let Some(ref t) = tx {
                let _ = t.try_send(OutboundMsg::new_s(
                    "td/all/res",
                    "unit",
                    "D",
                    json!({ "id": e.id() }),
                ));
            }
            let _ = data.entities.delete(e);
        }
    }
}
