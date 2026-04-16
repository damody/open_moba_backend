use specs::{
    shred, Entities, Join, Read, SystemData, Write, WriteStorage, ReadStorage,
};
use crossbeam_channel::Sender;
use vek::*;
use serde_json::json;

use crate::comp::*;
use crate::transport::OutboundMsg;

#[derive(SystemData)]
pub struct HeroMoveRead<'a> {
    entities: Entities<'a>,
    dt: Read<'a, DeltaTime>,
    heroes: ReadStorage<'a, Hero>,
    propertys: ReadStorage<'a, CProperty>,
}

#[derive(SystemData)]
pub struct HeroMoveWrite<'a> {
    pos: WriteStorage<'a, Pos>,
    move_targets: WriteStorage<'a, MoveTarget>,
    mqtx: Write<'a, Vec<Sender<OutboundMsg>>>,
}

#[derive(Default)]
pub struct Sys;

impl<'a> System<'a> for Sys {
    type SystemData = (
        HeroMoveRead<'a>,
        HeroMoveWrite<'a>,
    );

    const NAME: &'static str = "hero_move";

    fn run(_job: &mut Job<Self>, (tr, mut tw): Self::SystemData) {
        let dt = tr.dt.0;
        if dt <= 0.0 {
            return;
        }

        let mut arrived: Vec<specs::Entity> = Vec::new();
        let mut broadcasts: Vec<(u32, f32, f32)> = Vec::new();

        for (entity, _hero, property, pos, move_target) in (
            &tr.entities,
            &tr.heroes,
            &tr.propertys,
            &mut tw.pos,
            &tw.move_targets,
        ).join() {
            let target = move_target.0;
            let diff = target - pos.0;
            let distance = diff.magnitude();
            let step = property.msd * dt;

            if distance <= step.max(1.0) {
                // 到達目標
                pos.0 = target;
                arrived.push(entity);
            } else {
                // 向目標移動
                let direction = diff / distance;
                pos.0 += direction * step;
            }

            broadcasts.push((entity.id(), pos.0.x, pos.0.y));
        }

        // 移除已到達的 MoveTarget
        for entity in arrived {
            tw.move_targets.remove(entity);
        }

        // 廣播位置更新
        if !broadcasts.is_empty() {
            if let Some(tx) = tw.mqtx.get(0) {
                for (id, x, y) in broadcasts {
                    let _ = tx.send(OutboundMsg::new_s(
                        "td/all/res",
                        "hero",
                        "M",
                        json!({"id": id, "x": x, "y": y}),
                    ));
                }
            }
        }
    }
}
