//! Summon tick：
//! 1. 倒數 `SummonedUnit.time_remaining`，歸零時 despawn（廣播 `unit/D` 清除渲染）。
//! 2. 處理玩家對召喚物下的 MoveTarget：simple lerp 位移 + facing 更新，抵達後 remove。
//!    召喚物的自動攻擊 / 追敵 AI 在 UnitScript::on_tick（script 端）跑，這裡只做
//!    「玩家命令移動」的通用覆蓋層，不處理碰撞（summoned units 允許穿越其他單位）。

use crossbeam_channel::Sender;
use serde_json::json;
use specs::{shred, Entities, Join, Read, SystemData, World, Write, WriteStorage, ReadStorage};

use crate::comp::*;
use crate::transport::OutboundMsg;

/// 召喚物被玩家命令移動時使用的固定移速。
/// 與 saika_gunner.rs 內的 MOVE_SPEED 對齊，避免 AI chase 跟玩家 command 移速差太多。
const SUMMON_COMMAND_SPEED: f32 = 280.0;

#[derive(SystemData)]
pub struct SummonTickData<'a> {
    entities: Entities<'a>,
    dt: Read<'a, DeltaTime>,
    summoned: WriteStorage<'a, SummonedUnit>,
    pos: WriteStorage<'a, Pos>,
    move_targets: WriteStorage<'a, MoveTarget>,
    facings: WriteStorage<'a, Facing>,
    mqtx: Write<'a, Vec<Sender<OutboundMsg>>>,
}

#[derive(Default)]
pub struct Sys;

impl<'a> System<'a> for Sys {
    type SystemData = SummonTickData<'a>;

    const NAME: &'static str = "summon";

    fn run(_job: &mut Job<Self>, mut data: Self::SystemData) {
        // TODO Phase 1[d]: drop conversion when summon tick goes Fixed32-native.
        let dt = data.dt.0.to_f32_for_render();

        // 1) MoveTarget 處理：先收集每個有 SummonedUnit + MoveTarget 的 entity 要改到哪個 pos /
        //    是否 arrive。arrive 之後額外在下一輪刷 MoveTarget.remove()。
        let mut arrived: Vec<specs::Entity> = Vec::new();
        for (e, _summ, pos, mt, facing) in (
            &data.entities,
            &data.summoned,
            &mut data.pos,
            &data.move_targets,
            &mut data.facings,
        )
            .join()
        {
            let (px, py) = pos.xy_f32();
            let (mx, my) = (mt.0.x.to_f32_for_render(), mt.0.y.to_f32_for_render());
            let dx = mx - px;
            let dy = my - py;
            let dist_sq = dx * dx + dy * dy;
            if dist_sq < 0.0001 {
                arrived.push(e);
                continue;
            }
            let step = SUMMON_COMMAND_SPEED * dt;
            let dist = dist_sq.sqrt();
            if dist <= step {
                pos.0 = mt.0;
                *facing = Facing::from_rad_f32(dy.atan2(dx));
                arrived.push(e);
            } else {
                let inv = 1.0 / dist;
                let new_x = px + dx * inv * step;
                let new_y = py + dy * inv * step;
                *pos = Pos::from_xy_f32(new_x, new_y);
                *facing = Facing::from_rad_f32(dy.atan2(dx));
            }
        }
        for e in arrived {
            data.move_targets.remove(e);
        }

        // 2) 時效倒數 + despawn
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
