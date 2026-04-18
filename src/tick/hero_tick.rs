use instant_distance::Point;
use specs::{
    shred, Entities, Join, LazyUpdate, Read, ReadExpect, ReadStorage, SystemData,
    Write, WriteStorage, ParJoin, World,
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
    propertys : ReadStorage<'a, CProperty>,
    units : ReadStorage<'a, Unit>,
}

#[derive(SystemData)]  
pub struct HeroWrite<'a> {
    outcomes: Write<'a, Vec<Outcome>>,
    heroes : WriteStorage<'a, Hero>,
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
        
        // 獲取英雄的陣營信息和名稱用於敵友判斷和日誌記錄
        let hero_faction_map: std::collections::HashMap<specs::Entity, Faction> = (
            &tr.entities,
            &tr.factions,
            &tw.heroes,
        ).join().map(|(e, f, _)| (e, f.clone())).collect();
        
        // 獲取英雄名稱映射表
        let hero_name_map: std::collections::HashMap<specs::Entity, String> = (
            &tr.entities,
            &tw.heroes,
        ).join().map(|(e, hero)| (e, hero.name.clone())).collect();
        
        let mut outcomes = (
            &tr.entities,
            &mut tw.heroes,
            &tr.propertys,
            &mut tw.tatks,
            &tr.pos,
        )
            .par_join()
            .map_init(
                || {
                    prof_span!(guard, "hero update rayon job");
                    guard
                },
                |_guard, (e, hero, pty, atk, pos)| {
                    let mut outcomes: Vec<Outcome> = Vec::new();
                    
                    // 直接更新攻擊冷卻時間
                    if atk.asd_count < atk.asd.v {
                        atk.asd_count += dt;
                    }
                    
                    // 當攻擊冷卻時間到達時，嘗試攻擊
                    if atk.asd_count >= atk.asd.v {
                        let time2 = Instant::now();
                        let elpsed = time2.duration_since(time1);
                        
                        // 防止過度計算
                        if elpsed.as_secs_f32() < 0.05 {
                            // 搜尋攻擊範圍內的所有單位
                            let search_n = 10; // 搜尋最近的 10 個目標
                            let attack_range = atk.range.v; // 攻擊範圍
                            let search_range = attack_range + 50.0; // 稍微擴大搜尋範圍以確保不遺漏邊界目標
                            let (creep_targets, _) =
                                tr.searcher.creep.SearchNN_XY2(pos.0, attack_range, search_range, search_n);
                            let (tower_targets, _) =
                                tr.searcher.tower.SearchNN_XY2(pos.0, attack_range, search_range, search_n);
                            // 合併 creep + tower 候選，一起走敵友判斷
                            let mut potential_targets = Vec::with_capacity(creep_targets.len() + tower_targets.len());
                            potential_targets.extend(creep_targets);
                            potential_targets.extend(tower_targets);
                            potential_targets.sort_by(|a, b| a.dis.partial_cmp(&b.dis).unwrap_or(std::cmp::Ordering::Equal));
                            
                            // 偵錯：顯示搜尋結果
                            // 獲取英雄名稱
                            let hero_name = hero_name_map.get(&e)
                                .cloned()
                                .unwrap_or_else(|| format!("英雄 {}", e.id()));
                            
                            if potential_targets.len() > 0 {
                                log::trace!("{} 在位置 ({:.0}, {:.0}) 搜尋到 {} 個潛在目標，攻擊範圍: {}", 
                                    hero_name, pos.0.x, pos.0.y, potential_targets.len(), atk.range.v);
                            } else {
                                log::trace!("{} 沒有找到目標", hero_name);
                            }
                                
                            // 過濾出可攻擊的敵對目標（必須在攻擊範圍內）
                            let mut valid_targets = Vec::new();
                            let attack_range_squared = attack_range * attack_range; // 計算攻擊範圍的平方
                            
                            if let Some(hero_faction) = hero_faction_map.get(&e) {
                                for target_info in potential_targets.iter() {
                                    let target_distance_squared = target_info.dis;

                                    // 首先檢查距離是否在攻擊範圍內
                                    if target_info.dis <= attack_range_squared {
                                        if let Some(target_faction) = tr.factions.get(target_info.e) {
                                            // 嚴格敵友判定：只有 is_hostile_to = true 才算敵對
                                            if hero_faction.is_hostile_to(target_faction) {
                                                valid_targets.push(target_info);
                                            }
                                        }
                                        // 無 Faction → 不攻擊（安全預設，避免誤擊我方單位）
                                    }
                                }
                            } else {
                                log::warn!("{} 沒有陣營信息，無法進行敵友判斷", hero_name);
                            }
                                
                            log::info!("{} 有效目標數量: {}", hero_name, valid_targets.len());
                            
                            if valid_targets.len() > 0 {
                                log::debug!("{} 準備攻擊，當前攻擊冷卻: {:.2}/{:.2}", hero_name, atk.asd_count, atk.asd.v);
                                
                                // 直接重置攻擊冷卻
                                atk.asd_count -= atk.asd.v;
                                log::debug!("{} 重置攻擊冷卻至: {:.2}", hero_name, atk.asd_count);
                                
                                // 攻擊最近的敵人
                                let target = valid_targets[0].e;
                                log::info!("{} 選擇攻擊目標: {}", hero_name, target.id());
                                
                                // 產生彈道事件（彈道會攜帶傷害資訊並在到達後產生傷害事件）
                                outcomes.push(Outcome::ProjectileLine2 { 
                                    pos: pos.0.clone(), 
                                    source: Some(e.clone()), 
                                    target: Some(target) 
                                });
                                
                                // 簡單的攻擊距離日誌（詳細的傷害和血量資訊會在彈道命中後顯示）
                                let actual_distance = valid_targets[0].dis.sqrt();
                                log::error!("⚔️ {} 發射彈道攻擊，距離: {:.0}，攻擊力: {:.1}", hero_name, actual_distance, atk.atk_physic.v);
                            } else {
                                // 沒有有效目標時，減少一些攻擊冷卻時間避免過度檢查
                                atk.asd_count = atk.asd.v - 0.3 - fastrand::u8(..) as f32 * 0.001;
                                log::trace!("{} 沒有找到有效目標，減少攻擊冷卻時間: {:.3}", hero_name, atk.asd_count);
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