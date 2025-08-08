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
    hero_attacks: ReadStorage<'a, TAttack>,
}

#[derive(SystemData)]
pub struct ProjectileWrite<'a> {
    pos : WriteStorage<'a, Pos>,
    projs : WriteStorage<'a, Projectile>,
    outcomes: Write<'a, Vec<Outcome>>,
    taken_damages: Write<'a, Vec<TakenDamage>>,
    damage_instances: Write<'a, Vec<DamageInstance>>,
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
                    let mut outcomes:Vec<Outcome> = Vec::new();
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
                            create_projectile_damage(&proj, target, &mut outcomes, pos.0);
                        }
                        outcomes.push(Outcome::Death { pos: pos.0.clone(), ent: e.clone() });
                    } else {
                        if dis < 1. {
                            // 投射物到達目標位置
                            if proj.radius > 1. { // 範圍攻擊（擴散炮）
                                let targets = tr.searcher.creep.SearchNN_XY(pos.0, proj.radius, 5);
                                for target_info in targets.iter() {
                                    create_projectile_damage(&proj, target_info.e, &mut outcomes, pos.0);
                                }
                            } else if let Some(target) = proj.target {
                                // 單體攻擊
                                create_projectile_damage(&proj, target, &mut outcomes, pos.0);
                            }
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
    }
}

/// 創建投射物傷害事件 - 使用新的傷害事件系統
fn create_projectile_damage(
    proj: &Projectile, 
    target: specs::Entity, 
    outcomes: &mut Vec<Outcome>,
    pos: vek::Vec2<f32>
) {
    // 使用彈道攜帶的傷害資訊創建傷害事件
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
}
