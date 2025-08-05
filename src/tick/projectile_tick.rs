use specs::{
    shred::{ResourceId, World}, Entities, Join, LazyUpdate, Read, ReadExpect, ReadStorage, SystemData,
    Write, WriteStorage, ParJoin, 
};
use crate::comp::*;
use specs::prelude::ParallelIterator;
use specs::saveload::MarkerAllocator;
use specs::Entity;
use vek::Vec2;

#[derive(SystemData)]
pub struct ProjectileRead<'a> {
    entities: Entities<'a>,
    time: Read<'a, Time>,
    dt: Read<'a, DeltaTime>,
    searcher : Read<'a, Searcher>,
}

#[derive(SystemData)]
pub struct ProjectileWrite<'a> {
    pos : WriteStorage<'a, Pos>,
    projs : WriteStorage<'a, Projectile>,
    outcomes: Write<'a, Vec<Outcome>>,
    taken_damages: Write<'a, Vec<TakenDamage>>,
    damage_instances: Write<'a, Vec<DamageInstance>>,
    hero_attacks: ReadStorage<'a, TAttack>,
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
        //log::info!("projs count {}", tw.projs.count());
        let (mut outcomes, mut taken_damages) = (
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
                    let mut outcomes:Vec<Outcome> = Vec::new();
                    let mut taken_damages:Vec<TakenDamage> = Vec::new();
                    let mut vel = (proj.tpos - pos.0);
                    vel.normalize();
                    vel *= proj.msd;
                    vel *= dt;
                    let dis = (proj.tpos - pos.0).magnitude_squared();
                    if vel.magnitude_squared() > dis || dis < 1.{
                        pos.0 = proj.tpos;
                    } else {
                        pos.0 += vel;
                    }
                    proj.time_left -= dt;
                    if proj.time_left <= 0. {
                        // 投射物到達目標或超時，造成傷害
                        if let Some(target) = proj.target {
                            create_projectile_damage(&proj, target, &mut taken_damages);
                        }
                        outcomes.push(Outcome::Death { pos: pos.0.clone(), ent: e.clone() });
                    } else {
                        if dis < 1. {
                            // 投射物到達目標位置
                            if proj.radius > 1. { // 範圍攻擊（擴散炮）
                                let targets = tr.searcher.creep.SearchNN_XY(pos.0, proj.radius, 5);
                                for target_info in targets.iter() {
                                    create_projectile_damage(&proj, target_info.e, &mut taken_damages);
                                }
                            } else if let Some(target) = proj.target {
                                // 單體攻擊
                                create_projectile_damage(&proj, target, &mut taken_damages);
                            }
                            outcomes.push(Outcome::Death { pos: pos.0.clone(), ent: e.clone() });
                        }
                    }
                    (outcomes, taken_damages)
                },
            )
            .fold(
                || (Vec::new(), Vec::new()),
                |(mut all_outcomes, mut all_taken_damages), 
                    (mut outcomes, mut taken_damages)| {
                    all_outcomes.append(&mut outcomes);
                    all_taken_damages.append(&mut taken_damages);
                    (all_outcomes, all_taken_damages)
                },
            )
            .reduce(
                || (Vec::new(), Vec::new()),
                |( mut outcomes_a, mut taken_damages_a),
                 ( mut outcomes_b, mut taken_damages_b)| {
                    outcomes_a.append(&mut outcomes_b);
                    taken_damages_a.append(&mut taken_damages_b);
                    (outcomes_a, taken_damages_a)
                },
            );
        tw.taken_damages.append(&mut taken_damages);
        tw.outcomes.append(&mut outcomes);
    }
}

/// 創建投射物傷害 - 將會被新的傷害系統取代
fn create_projectile_damage(
    proj: &Projectile, 
    target: specs::Entity, 
    taken_damages: &mut Vec<TakenDamage>
) {
    // 暫時使用舊的 TakenDamage 系統，直到完全遷移到新系統
    // TODO: 根據投射物來源計算實際傷害值
    let damage = if proj.owner.id() > 0 {
        // 來自英雄的攻擊
        TakenDamage { ent: target, phys: 45.0, magi: 0.0, real: 0.0 }
    } else {
        // 來自塔或其他來源的攻擊
        TakenDamage { ent: target, phys: 25.0, magi: 0.0, real: 0.0 }
    };
    
    taken_damages.push(damage);
}
