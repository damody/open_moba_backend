use specs::{
    shred, Entities, Join, Read, SystemData, Write, WriteStorage, ReadStorage,
};
use crossbeam_channel::Sender;
use vek::*;
use serde_json::json;

use crate::comp::*;
use crate::transport::OutboundMsg;
use crate::util::geometry::circle_hits_polygon;

#[derive(SystemData)]
pub struct HeroMoveRead<'a> {
    entities: Entities<'a>,
    dt: Read<'a, DeltaTime>,
    heroes: ReadStorage<'a, Hero>,
    propertys: ReadStorage<'a, CProperty>,
    turn_speeds: ReadStorage<'a, TurnSpeed>,
    radii: ReadStorage<'a, CollisionRadius>,
    regions: Read<'a, BlockedRegions>,
}

#[derive(SystemData)]
pub struct HeroMoveWrite<'a> {
    pos: WriteStorage<'a, Pos>,
    move_targets: WriteStorage<'a, MoveTarget>,
    facings: WriteStorage<'a, Facing>,
    mqtx: Write<'a, Vec<Sender<OutboundMsg>>>,
}

#[derive(Default)]
pub struct Sys;

/// 檢查若單位移動到 `new_center` 是否會撞進任何 blocked region。
fn hits_any(new_center: Vec2<f32>, radius: f32, regions: &BlockedRegions) -> bool {
    regions
        .0
        .iter()
        .any(|r| circle_hits_polygon(new_center, radius, &r.points))
}

/// 計算避開 region 的下一步位置：嘗試直接走 → 只走 X → 只走 Y → 停。
/// 回傳 (新位置, 是否抵達目標範圍)。
fn advance_with_collision(
    pos: Vec2<f32>,
    target: Vec2<f32>,
    step: f32,
    radius: f32,
    regions: &BlockedRegions,
) -> (Vec2<f32>, bool) {
    let diff = target - pos;
    let distance = diff.magnitude();
    if distance < 0.5 {
        return (target, true);
    }
    let direction = diff / distance;
    // 抵達：step 足夠蓋住剩餘距離
    if distance <= step.max(1.0) {
        if !hits_any(target, radius, regions) {
            return (target, true);
        }
        // target 本身在 region 內 → 走到剛好撞前為止（保留目前位置）
        return (pos, false);
    }
    let full = pos + direction * step;
    if !hits_any(full, radius, regions) {
        return (full, false);
    }
    // Wall sliding：只保留 x 或只保留 y 分量
    let only_x = pos + Vec2::new(direction.x * step, 0.0);
    if !hits_any(only_x, radius, regions) {
        return (only_x, false);
    }
    let only_y = pos + Vec2::new(0.0, direction.y * step);
    if !hits_any(only_y, radius, regions) {
        return (only_y, false);
    }
    // 全軸都撞到 → 本 tick 不動
    (pos, false)
}

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
        // (entity_id, x, y, facing)
        let mut broadcasts: Vec<(u32, f32, f32, f32)> = Vec::new();

        for (entity, _hero, property, pos, move_target, facing) in (
            &tr.entities,
            &tr.heroes,
            &tr.propertys,
            &mut tw.pos,
            &tw.move_targets,
            &mut tw.facings,
        ).join() {
            let target = move_target.0;
            let diff = target - pos.0;
            let distance = diff.magnitude();
            let step = property.msd * dt;

            // 先轉向目標方向
            if distance > 0.5 {
                let desired = diff.y.atan2(diff.x);
                let turn = tr
                    .turn_speeds
                    .get(entity)
                    .map(|t| t.0)
                    .unwrap_or(std::f32::consts::FRAC_PI_2);
                facing.0 = rotate_toward(facing.0, desired, turn * dt);

                // 面向夾角 < 30° 才能前進
                let angle_diff = normalize_angle(desired - facing.0).abs();
                if angle_diff < MOVE_ANGLE_THRESHOLD {
                    let radius = tr.radii.get(entity).map(|r| r.0).unwrap_or(30.0);
                    let (new_pos, reached) = advance_with_collision(
                        pos.0,
                        target,
                        step,
                        radius,
                        &tr.regions,
                    );
                    pos.0 = new_pos;
                    if reached {
                        arrived.push(entity);
                    }
                }
                // 角度太大 → 只轉向、不位移（本 tick 不動）
            } else {
                arrived.push(entity);
            }

            broadcasts.push((entity.id(), pos.0.x, pos.0.y, facing.0));
        }

        // 移除已到達的 MoveTarget
        for entity in arrived {
            tw.move_targets.remove(entity);
        }

        // 廣播位置 + facing 更新
        if !broadcasts.is_empty() {
            if let Some(tx) = tw.mqtx.get(0) {
                for (id, x, y, facing) in broadcasts {
                    let _ = tx.send(OutboundMsg::new_s(
                        "td/all/res",
                        "hero",
                        "M",
                        json!({"id": id, "x": x, "y": y, "facing": facing}),
                    ));
                }
            }
        }
    }
}
