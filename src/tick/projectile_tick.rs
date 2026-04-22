use specs::{
    shred, Entities, Join, LazyUpdate, Read, ReadExpect, ReadStorage, SystemData,
    Write, WriteStorage, ParJoin, World,
};
use crossbeam_channel::Sender;
use crate::comp::*;
use crate::transport::OutboundMsg;
use specs::prelude::ParallelIterator;
use specs::Entity;
use vek::Vec2;

#[derive(SystemData)]
pub struct ProjectileRead<'a> {
    entities: Entities<'a>,
    time: Read<'a, Time>,
    dt: Read<'a, DeltaTime>,
    searcher : Read<'a, Searcher>,
    hero_attacks: ReadStorage<'a, TAttack>,
}

#[derive(SystemData)]
pub struct ProjectileWrite<'a> {
    pos : WriteStorage<'a, Pos>,
    projs : WriteStorage<'a, Projectile>,
    outcomes: Write<'a, Vec<Outcome>>,
    taken_damages: Write<'a, Vec<TakenDamage>>,
    damage_instances: Write<'a, Vec<DamageInstance>>,
    mqtx: Write<'a, Vec<Sender<OutboundMsg>>>,
}

#[derive(Default)]
pub struct Sys;

impl<'a> System<'a> for Sys {
    type SystemData = (
        ProjectileRead<'a>,
        ProjectileWrite<'a>,
    );

    const NAME: &'static str = "projectile";

    fn run(_job: &mut Job<Self>, (tr, mut tw): Self::SystemData) {
        let time = tr.time.0;
        let dt = tr.dt.0;

        // Snapshot every entity's current Pos so projectiles can home toward the
        // target's LIVE position each tick (homing). Previously `tpos` was frozen
        // at firing time, so the bullet flew to where the target used to be — it
        // visually missed a moving target even though damage was still applied
        // via the stored `target` Entity.
        let target_positions: std::collections::HashMap<specs::Entity, vek::Vec2<f32>> = {
            use specs::Join;
            (&tr.entities, &tw.pos).join()
                .map(|(e, pos)| (e, pos.0))
                .collect()
        };

        //log::info!("projs count {}", tw.projs.count());
        let mut outcomes = (
            &tr.entities,
            &mut tw.projs,
            &mut tw.pos,
        )
            .par_join()
            .filter(|(e, proj, p)| proj.time_left > 0.)
            .map_init(
                || {
                    prof_span!(guard, "projectile update rayon job");
                    guard
                },
                |_guard, (e, proj, pos)| {
                    let mut outcomes: Vec<Outcome> = Vec::new();
                    // Home onto target's current position if still alive；
                    // target 消失時用 stale tpos，靠 time_left 安全閥讓彈道自然消失。
                    if let Some(target) = proj.target {
                        if let Some(&current_tpos) = target_positions.get(&target) {
                            proj.tpos = current_tpos;
                        }
                    }
                    let delta = proj.tpos - pos.0;
                    let dist = delta.magnitude();
                    let step = proj.msd * dt;

                    // 無 target 的方向性子彈（Tack 放射針）：飛行途中每 tick 掃描
                    // 命中半徑內任一敵人→直接扣血+消失；不需抵達終點
                    if proj.target.is_none() && proj.radius < 1.0 {
                        let near = tr.searcher.creep.SearchNN_XY(pos.0, crate::comp::TACK_NEEDLE_HIT_RADIUS, 1);
                        if let Some(hit) = near.first() {
                            create_projectile_damage(&proj, hit.e, &mut outcomes, pos.0);
                            outcomes.push(Outcome::Death { pos: pos.0.clone(), ent: e.clone() });
                            return outcomes;
                        }
                    }

                    // 命中判定：本 tick 的移動量已足夠抵達目標 → 直接 hit
                    let reached = dist <= step || dist < 1.0;
                    if reached {
                        // 命中點：優先用 target 的最新位置（snapshot = 本 tick 初的 Pos storage），
                        // 這樣 AoE 圓心和爆炸特效一定落在氣球身上，不會停在子彈剛發射時那一刻。
                        let hit_pos = if let Some(target) = proj.target {
                            target_positions.get(&target).copied().unwrap_or(proj.tpos)
                        } else {
                            proj.tpos
                        };
                        pos.0 = hit_pos;
                        if proj.radius > 1.0 {
                            // 範圍攻擊：以 hit_pos 為中心掃半徑內敵人
                            let targets = tr.searcher.creep.SearchNN_XY(hit_pos, proj.radius, 5);
                            for target_info in targets.iter() {
                                create_projectile_damage(&proj, target_info.e, &mut outcomes, hit_pos);
                            }
                            // 爆炸視覺由前端自己在子彈飛完時 spawn（projectile/C 有帶
                            // splash_radius，前端在 elapsed>=flight_time 時畫圈）。
                            // 後端不再廣播 game/explosion，避免雙重爆炸 + 位置不同步。
                        } else if let Some(target) = proj.target {
                            // 單體攻擊
                            create_projectile_damage(&proj, target, &mut outcomes, hit_pos);
                        }
                        // 方向性子彈：抵達 end_pos 但沒打到任何敵人 → 直接消失
                        outcomes.push(Outcome::Death { pos: hit_pos, ent: e.clone() });
                    } else {
                        // 還沒抵達：往目標方向前進一個 step
                        let vel = delta / dist * step;
                        pos.0 += vel;
                        // 安全閥：time_left 到期仍未命中（例如 target 死掉 tpos 凍結），讓 projectile 自然消失
                        proj.time_left -= dt;
                        if proj.time_left <= 0.0 {
                            outcomes.push(Outcome::Death { pos: pos.0.clone(), ent: e.clone() });
                        }
                    }
                    outcomes
                },
            )
            .fold(
                || Vec::new(),
                |mut all_outcomes, mut outcomes| {
                    all_outcomes.append(&mut outcomes);
                    all_outcomes
                },
            )
            .reduce(
                || Vec::new(),
                |mut outcomes_a, mut outcomes_b| {
                    outcomes_a.append(&mut outcomes_b);
                    outcomes_a
                },
            );
        tw.outcomes.append(&mut outcomes);

        // 前端已自管子彈動畫（收 C 時拿 target_id + flight_time_ms 後本地 pursuit lerp），
        // 不再廣播 projectile 每 tick 位置。
    }
}

/// 創建投射物傷害事件 - 使用新的傷害事件系統。
/// 若 projectile 帶有 slow_factor/slow_duration（Ice 塔）則同時 push ApplySlow。
fn create_projectile_damage(
    proj: &Projectile,
    target: specs::Entity,
    outcomes: &mut Vec<Outcome>,
    pos: vek::Vec2<f32>
) {
    log::debug!("彈道命中目標 {}，物理傷害: {:.1}，魔法傷害: {:.1}，真實傷害: {:.1}",
        target.id(), proj.damage_phys, proj.damage_magi, proj.damage_real);

    outcomes.push(Outcome::Damage {
        pos: pos,
        phys: proj.damage_phys,
        magi: proj.damage_magi,
        real: proj.damage_real,
        source: proj.owner,
        target: target,
    });

    // Ice 塔：附加減速 debuff 到目標
    if proj.slow_factor > 0.0 && proj.slow_factor < 1.0 && proj.slow_duration > 0.0 {
        outcomes.push(Outcome::ApplySlow {
            target,
            factor: proj.slow_factor,
            duration: proj.slow_duration,
        });
    }
}
