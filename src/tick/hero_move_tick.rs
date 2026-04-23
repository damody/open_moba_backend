use specs::{
    shred, Entities, Join, Read, SystemData, Write, WriteStorage, ReadStorage,
};
use crossbeam_channel::Sender;
use vek::*;
use serde_json::json;

use crate::comp::*;
use crate::comp::phys::MAX_COLLISION_RADIUS;
use crate::transport::OutboundMsg;
use crate::util::geometry::point_in_polygon;
use std::sync::atomic::{AtomicU64, Ordering};

static TICK_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(SystemData)]
pub struct HeroMoveRead<'a> {
    entities: Entities<'a>,
    dt: Read<'a, DeltaTime>,
    heroes: ReadStorage<'a, Hero>,
    propertys: ReadStorage<'a, CProperty>,
    turn_speeds: ReadStorage<'a, TurnSpeed>,
    radii: ReadStorage<'a, CollisionRadius>,
    searcher: Read<'a, Searcher>,
    /// Debug only：驗證 hero 是否進入 polygon 但未被 blocker 擋
    regions: Read<'a, BlockedRegions>,
    buff_store: Read<'a, crate::ability_runtime::BuffStore>,
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

/// 檢查若單位移動到 `new_center` 是否會撞進任何其他有 CollisionRadius 的實體。
/// Region 阻擋透過 blocker entities 一起走 Searcher 查詢，不再需要 polygon 測試。
pub(crate) fn hits_any(
    new_center: Vec2<f32>,
    radius: f32,
    searcher: &Searcher,
    radii: &ReadStorage<CollisionRadius>,
    self_entity: specs::Entity,
    _regions: &BlockedRegions,
) -> bool {
    let q_r = radius + MAX_COLLISION_RADIUS;
    for di in searcher.search_collidable(new_center, q_r, 16) {
        if di.e == self_entity { continue; }
        let Some(other_r) = radii.get(di.e).map(|cr| cr.0) else { continue };
        let touch = radius + other_r;
        if di.dis < touch * touch {
            return true;
        }
    }
    false
}

/// 計算避開其他單位的下一步位置：嘗試直接走 → 只走 X → 只走 Y → 停。
/// 回傳 (新位置, 是否抵達目標範圍)。
pub(crate) fn advance_with_collision(
    pos: Vec2<f32>,
    target: Vec2<f32>,
    step: f32,
    radius: f32,
    searcher: &Searcher,
    radii: &ReadStorage<CollisionRadius>,
    self_entity: specs::Entity,
    regions: &BlockedRegions,
) -> (Vec2<f32>, bool) {
    let diff = target - pos;
    let distance = diff.magnitude();
    if distance < 0.5 {
        return (target, true);
    }
    let direction = diff / distance;
    if distance <= step.max(1.0) {
        if !hits_any(target, radius, searcher, radii, self_entity, regions) {
            return (target, true);
        }
        return (pos, false);
    }
    let full = pos + direction * step;
    if !hits_any(full, radius, searcher, radii, self_entity, regions) {
        return (full, false);
    }
    let only_x = pos + Vec2::new(direction.x * step, 0.0);
    if !hits_any(only_x, radius, searcher, radii, self_entity, regions) {
        return (only_x, false);
    }
    let only_y = pos + Vec2::new(0.0, direction.y * step);
    if !hits_any(only_y, radius, searcher, radii, self_entity, regions) {
        return (only_y, false);
    }
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

        // 每 120 tick (~2s) log 一次 searcher 各 index 大小，確認 region 已載入
        let t = TICK_COUNTER.fetch_add(1, Ordering::Relaxed);
        if t % 120 == 0 {
            log::warn!(
                "🔍 searcher sizes: hero={}, creep={}, tower={}, region={}",
                tr.searcher.hero.xpos.len(),
                tr.searcher.creep.xpos.len(),
                tr.searcher.tower.xpos.len(),
                tr.searcher.region.xpos.len()
            );
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
            // Root / stun 狀態：完全凍結（不轉向、不位移、不消耗 MoveTarget）
            if tr.buff_store.is_rooted(entity) {
                broadcasts.push((entity.id(), pos.0.x, pos.0.y, facing.0));
                continue;
            }

            let target = move_target.0;
            let diff = target - pos.0;
            let distance = diff.magnitude();
            // 移動速度乘數：buff 的 move_speed_multiplier 連乘（例：sniper 0.5 = 半速）
            let msd_mult = tr.buff_store.product_mult(entity, "move_speed_multiplier");
            let step = property.msd * msd_mult * dt;

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
                        &tr.searcher,
                        &tr.radii,
                        entity,
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
