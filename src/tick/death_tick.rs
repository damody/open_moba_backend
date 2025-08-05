use specs::{
    shred::{ResourceId, World}, Entities, Join, LazyUpdate, Read, ReadExpect, ReadStorage, SystemData,
    Write, WriteStorage, ParJoin, Entity,
};
use crate::comp::*;
use specs::prelude::ParallelIterator;
use std::{
    time::{Duration, Instant},
    collections::HashMap,
};

#[derive(SystemData)]
pub struct DeathRead<'a> {
    entities: Entities<'a>,
    time: Read<'a, Time>,
    units: ReadStorage<'a, Unit>,
    heroes: ReadStorage<'a, Hero>,
    factions: ReadStorage<'a, Faction>,
    positions: ReadStorage<'a, Pos>,
    properties: ReadStorage<'a, CProperty>,
}

#[derive(SystemData)]
pub struct DeathWrite<'a> {
    outcomes: Write<'a, Vec<Outcome>>,
}

#[derive(Default)]
pub struct Sys;

impl<'a> System<'a> for Sys {
    type SystemData = (
        DeathRead<'a>,
        DeathWrite<'a>,
    );

    const NAME: &'static str = "death";

    fn run(_job: &mut Job<Self>, (tr, mut tw): Self::SystemData) {
        let time = tr.time.0;
        
        // 收集所有需要檢查死亡的實體
        let mut dead_entities = Vec::new();
        let mut death_rewards = Vec::new();
        
        // 檢查所有有 Unit 組件和 CProperty 組件的實體
        for (entity, unit, properties, pos) in (&tr.entities, &tr.units, &tr.properties, &tr.positions).join() {
            if properties.hp <= 0.0 {
                dead_entities.push(entity);
                
                // 記錄死亡獎勵信息
                death_rewards.push(DeathReward {
                    dead_entity: entity,
                    dead_unit: unit.clone(),
                    position: pos.0,
                    exp_reward: unit.exp_reward,
                    gold_reward: unit.gold_reward,
                    bounty_type: unit.bounty_type.clone(),
                });
                
                log::info!("Unit '{}' died at position ({:.1}, {:.1})", 
                          unit.name, pos.0.x, pos.0.y);
            }
        }
        
        // 處理死亡獎勵 - 分配給附近的友方英雄
        for reward in death_rewards {
            distribute_death_rewards(&reward, &tr, &mut tw);
        }
        
        // 生成死亡事件
        for dead_entity in dead_entities {
            if let Some(pos) = tr.positions.get(dead_entity) {
                tw.outcomes.push(Outcome::Death { 
                    pos: pos.0, 
                    ent: dead_entity 
                });
            }
        }
    }
}

/// 死亡獎勵信息
#[derive(Clone, Debug)]
struct DeathReward {
    dead_entity: Entity,
    dead_unit: Unit,
    position: vek::Vec2<f32>,
    exp_reward: i32,
    gold_reward: i32,
    bounty_type: BountyType,
}

/// 分配死亡獎勵給附近的友方英雄
fn distribute_death_rewards(
    reward: &DeathReward,
    tr: &DeathRead,
    tw: &mut DeathWrite,
) {
    const EXPERIENCE_RANGE: f32 = 1200.0; // 經驗值獲取範圍
    const GOLD_RANGE: f32 = 800.0;       // 金錢獲取範圍
    
    let mut eligible_heroes = Vec::new();
    
    // 找到範圍內的友方英雄
    for (hero_entity, hero, hero_pos, hero_faction) in (&tr.entities, &tr.heroes, &tr.positions, &tr.factions).join() {
        let distance_sq = (hero_pos.0 - reward.position).magnitude_squared();
        
        // 檢查是否在範圍內且為敵對陣營（可以獲得獎勵）
        if let Some(dead_faction) = tr.factions.get(reward.dead_entity) {
            if hero_faction.is_hostile_to(dead_faction) && distance_sq <= EXPERIENCE_RANGE * EXPERIENCE_RANGE {
                eligible_heroes.push((hero_entity, distance_sq));
            }
        } else {
            // 沒有陣營的單位默認給玩家陣營獎勵
            if hero_faction.faction_id == FactionType::Player && distance_sq <= EXPERIENCE_RANGE * EXPERIENCE_RANGE {
                eligible_heroes.push((hero_entity, distance_sq));
            }
        }
    }
    
    if eligible_heroes.is_empty() {
        return;
    }
    
    // 按距離排序，最近的獲得更多獎勵
    eligible_heroes.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    
    // 計算獎勵分配
    let (primary_exp, shared_exp) = match reward.bounty_type {
        BountyType::Boss => (reward.exp_reward, reward.exp_reward / 2), // Boss 經驗分享給所有人
        BountyType::Siege => (reward.exp_reward, reward.exp_reward / 3), // 攻城單位部分分享
        _ => (reward.exp_reward, 0), // 普通單位只給最近的英雄
    };
    
    // 分配經驗值
    for (i, (hero_entity, distance_sq)) in eligible_heroes.iter().enumerate() {
        if let Some(hero) = tr.heroes.get(*hero_entity) {
            let exp_to_give = if i == 0 {
                // 最近的英雄獲得主要經驗
                primary_exp
            } else if shared_exp > 0 {
                // 其他英雄獲得分享經驗
                shared_exp
            } else {
                0
            };
            
            if exp_to_give > 0 {
                // 生成經驗獲得事件
                tw.outcomes.push(Outcome::GainExperience {
                    target: *hero_entity,
                    amount: exp_to_give,
                });
            }
        }
    }
    
    // 金錢獎勵只給最近的英雄（在金錢範圍內）
    if let Some((closest_hero, distance_sq)) = eligible_heroes.first() {
        if *distance_sq <= GOLD_RANGE * GOLD_RANGE && reward.gold_reward > 0 {
            // TODO: 實現金錢系統
            log::info!("Hero would receive {} gold for killing '{}'", 
                      reward.gold_reward, reward.dead_unit.name);
        }
    }
}