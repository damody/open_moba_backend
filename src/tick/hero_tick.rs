use instant_distance::Point;
use specs::{
    shred::{ResourceId, World}, Entities, Join, LazyUpdate, Read, ReadExpect, ReadStorage, SystemData,
    Write, WriteStorage, ParJoin, 
};
use crate::comp::*;
use specs::prelude::ParallelIterator;
use vek::*;
use std::{
    time::{Duration, Instant},
};
use specs::Entity;

#[derive(SystemData)]
pub struct HeroRead<'a> {
    entities: Entities<'a>,
    time: Read<'a, Time>,
    dt: Read<'a, DeltaTime>,
    pos : ReadStorage<'a, Pos>,
    searcher : Read<'a, Searcher>,
    factions: ReadStorage<'a, Faction>,
}

#[derive(SystemData)]  
pub struct HeroWrite<'a> {
    outcomes: Write<'a, Vec<Outcome>>,
    heroes : WriteStorage<'a, Hero>,
    units : WriteStorage<'a, Unit>,
    propertys : WriteStorage<'a, CProperty>,
    tatks : WriteStorage<'a, TAttack>,
}

#[derive(Default)]
pub struct Sys;

impl<'a> System<'a> for Sys {
    type SystemData = (
        HeroRead<'a>,
        HeroWrite<'a>,
    );

    const NAME: &'static str = "hero";

    fn run(_job: &mut Job<Self>, (tr, mut tw): Self::SystemData) {
        let time = tr.time.0;
        let dt = tr.dt.0;
        let time1 = Instant::now();
        
        // 獲取英雄的陣營信息用於敵友判斷
        let hero_faction_map: std::collections::HashMap<specs::Entity, Faction> = (
            &tr.entities,
            &tr.factions,
            &tw.heroes,
        ).join().map(|(e, f, _)| (e, f.clone())).collect();
        
        let mut outcomes = (
            &tr.entities,
            &mut tw.heroes,
            &mut tw.units,
            &mut tw.propertys,
            &mut tw.tatks,
            &tr.pos,
        )
            .par_join()
            .map_init(
                || {
                    prof_span!(guard, "hero update rayon job");
                    guard
                },
                |_guard, (e, hero, unit, pty, atk, pos)| {
                    let mut outcomes: Vec<Outcome> = Vec::new();
                    
                    // 更新攻擊冷卻時間
                    if atk.asd_count < atk.asd.val() {
                        atk.asd_count += dt;
                    }
                    
                    // 當攻擊冷卻時間到達時，嘗試攻擊
                    if atk.asd_count >= atk.asd.val() {
                        let time2 = Instant::now();
                        let elpsed = time2.duration_since(time1);
                        
                        // 防止過度計算
                        if elpsed.as_secs_f32() < 0.05 {
                            // 搜尋攻擊範圍內的所有單位
                            let search_n = 10; // 搜尋最近的 10 個目標
                            let (potential_targets, _) = 
                                tr.searcher.creep.SearchNN_XY2(pos.0, atk.range.val(), atk.range.val() + 30., search_n);
                                
                            // 過濾出可攻擊的敵對目標
                            let mut valid_targets = Vec::new();
                            if let Some(hero_faction) = hero_faction_map.get(&e) {
                                for target_info in potential_targets.iter() {
                                    if let Some(target_faction) = tr.factions.get(target_info.e) {
                                        // 檢查是否為敵對陣營
                                        if hero_faction.is_hostile_to(target_faction) {
                                            valid_targets.push(target_info);
                                        }
                                    } else {
                                        // 沒有陣營組件的目標默認可攻擊（向後兼容舊系統）
                                        valid_targets.push(target_info);
                                    }
                                }
                            }
                                
                            if valid_targets.len() > 0 {
                                // 重置攻擊冷卻時間
                                atk.asd_count -= atk.asd.val();
                                
                                // 攻擊最近的敵人
                                let target = valid_targets[0].e;
                                outcomes.push(Outcome::ProjectileLine2 { 
                                    pos: pos.0.clone(), 
                                    source: Some(e.clone()), 
                                    target: Some(target) 
                                });
                                
                                log::info!("Hero {} attacked hostile target at distance {:.1}", 
                                          hero.name, valid_targets[0].dis.sqrt());
                            } else {
                                // 沒有有效目標時，減少一些攻擊冷卻時間避免過度檢查
                                atk.asd_count = atk.asd.val() - 0.3 - fastrand::u8(..) as f32 * 0.001;
                            }
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