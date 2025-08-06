use std::{thread, ops::Deref, collections::BTreeMap};
use rayon::{ThreadPool, ThreadPoolBuilder};
use specs::{
    prelude::Resource,
    shred::{Fetch, FetchMut},
    storage::{MaskedStorage as EcsMaskedStorage, Storage as EcsStorage},
    Component, DispatcherBuilder, Entity, WorldExt, Builder,
};
use specs::world::Generation;
use std::sync::Arc;
use vek::*;
use crate::{comp::*, msg::MqttMsg};
use super::last::Last;
use std::time::{Instant, SystemTime};
use core::{convert::identity, time::Duration};
use failure::{err_msg, Error};
use serde::{Deserialize, Serialize};

use crate::tick::*;
use crate::Outcome;
use crate::Projectile;
use crate::PlayerData;
use crate::ue4::import_map::CreepWaveData;
use crate::ue4::import_campaign::CampaignData;
use serde_json::json;

use specs::saveload::MarkerAllocator;
use rand::{thread_rng, Rng};
use rand::distributions::{Alphanumeric, Uniform, Standard};
use crossbeam_channel::{bounded, select, tick, Receiver, Sender};

// Import refactored modules
mod ecs_setup;
mod campaign_manager;
mod mqtt_handler;
mod game_processor;

use ecs_setup::EcsSetup;
use campaign_manager::CampaignManager;
use mqtt_handler::MqttHandler;
use game_processor::GameProcessor;

pub struct State {
    ecs: specs::World,
    cw: CreepWaveData,
    campaign: Option<CampaignData>, // 戰役資料
    mqtx: Sender<MqttMsg>,
    mqrx: Receiver<PlayerData>,
    // Avoid lifetime annotation by storing a thread pool instead of the whole dispatcher
    thread_pool: Arc<ThreadPool>,
}

/// How much faster should an in-game day be compared to a real day?
// TODO: Don't hard-code this.
const DAY_CYCLE_FACTOR: f64 = 24.0 * 1.0;
const MAX_DELTA_TIME: f32 = 1.0;

impl State {
    pub fn new(pcw: CreepWaveData, mqtx: Sender<MqttMsg>, mqrx: Receiver<PlayerData>) -> Self {
        let thread_pool = Arc::new(
            ThreadPoolBuilder::new()
                .num_threads(num_cpus::get())
                .thread_name(move |i| format!("rayon-{}", i))
                .build()
                .unwrap(),
        );
        let mut res = Self {
            ecs: EcsSetup::setup_ecs_world(&thread_pool),
            cw: pcw,
            campaign: None,
            mqtx: mqtx.clone(),
            mqrx: mqrx.clone(),
            thread_pool,
        };
        res.init_creep_wave();
        res.create_test_scene();
        res
    }
    
    /// 使用戰役資料創建新的 State
    pub fn new_with_campaign(campaign_data: CampaignData, mqtx: Sender<MqttMsg>, mqrx: Receiver<PlayerData>) -> Self {
        let thread_pool = Arc::new(
            ThreadPoolBuilder::new()
                .num_threads(num_cpus::get())
                .thread_name(move |i| format!("rayon-{}", i))
                .build()
                .unwrap(),
        );
        let mut res = Self {
            ecs: EcsSetup::setup_ecs_world_with_campaign(&thread_pool),
            cw: campaign_data.map.clone(),
            campaign: Some(campaign_data.clone()),
            mqtx: mqtx.clone(),
            mqrx: mqrx.clone(),
            thread_pool,
        };
        CampaignManager::init_campaign_data(&mut res.ecs, &campaign_data);
        res.init_creep_wave();
        CampaignManager::create_campaign_scene(&mut res.ecs, &campaign_data);
        res
    }
    fn init_creep_wave(&mut self) {
        self.ecs.insert(vec![self.mqtx.clone()]);
        self.ecs.insert(vec![self.mqrx.clone()]);
        let cps = {
            let mut cps = self.ecs.get_mut::<BTreeMap::<String, CheckPoint>>().unwrap();
            for p in self.cw.CheckPoint.iter() {
                cps.insert(p.Name.clone(), 
                    CheckPoint{name:p.Name.clone(), class: p.Class.clone(), pos: Vec2::new(p.X, p.Y)});
            }
            cps.clone()
        };
        {
            let mut paths = self.ecs.get_mut::<BTreeMap::<String, Path>>().unwrap();
            for p in self.cw.Path.iter() {
                let mut cp_in_path = vec![];
                for ps in p.Points.iter() {
                    if let Some(v) = cps.get(ps) {
                        cp_in_path.push(v.clone());
                    }
                }
                paths.insert(p.Name.clone(), 
                    Path {check_points: cp_in_path});
            }
        }
        {
            let mut ces = self.ecs.get_mut::<BTreeMap::<String, CreepEmiter>>().unwrap();
            for cp in self.cw.Creep.iter() {
                ces.insert(cp.Name.clone(), CreepEmiter { 
                    root: Creep{name: cp.Name.clone(), path: "".to_owned(), pidx: 0, block_tower: None, status: CreepStatus::Walk}, 
                    property: CProperty { hp: cp.HP, mhp: cp.HP, msd: cp.MoveSpeed, def_physic: cp.DefendPhysic, def_magic: cp.DefendMagic } });
            }
        }
        {
            let mut cws = self.ecs.get_mut::<Vec::<CreepWave>>().unwrap();
            for cw in self.cw.CreepWave.iter() {
                let mut tcw = CreepWave { time: cw.StartTime, path_creeps: vec![] };
                let mut pcs: &mut Vec<PathCreeps> = &mut tcw.path_creeps;
                for d in cw.Detail.iter() {
                    let mut es = vec![];
                    for cjd in d.Creeps.iter() {
                        es.push(CreepEmit{time: cjd.Time, name: cjd.Creep.clone()});
                    }
                    pcs.push(PathCreeps { creeps: es, path_name: d.Path.clone() });
                }
                cws.push(tcw);
            }
        }
    }
    // ECS setup methods moved to ecs_setup.rs module
    // Terrain heightmap loading moved to ecs_setup.rs module
    
    /// 註冊英雄技能到新系統 (暫時註解掉，等待新系統整合)
    // fn register_hero_abilities_to_new_system(&mut self, hero_entity: specs::Entity, ability_ids: &[String]) {
    //     let mut ability_bridge = self.ecs.get_mut::<AbilityBridge>().unwrap();
    //     
    //     for ability_id in ability_ids {
    //         ability_bridge.register_ability(hero_entity, ability_id.clone());
    //         
    //         // 為基礎技能升級
    //         if ability_id == "sniper_mode" || ability_id == "saika_reinforcements" {
    //             ability_bridge.level_up_ability(hero_entity, ability_id);
    //         }
    //     }
    //     
    //     log::info!("Registered {} abilities to new system for hero entity", ability_ids.len());
    // }
    
    // Campaign scene creation moved to campaign_manager.rs module
    
    // Tutorial and training scene creation moved to campaign_manager.rs module
    // Hero property creation methods moved to campaign_manager.rs module
    
    /// 創建英雄技能實體
    fn create_hero_abilities(&mut self, hero_entity: specs::Entity, ability_ids: &[String], campaign_data: &CampaignData) {
        for ability_id in ability_ids {
            if let Some(ability_data) = campaign_data.ability.abilities.get(ability_id) {
                let mut ability = Ability::from_campaign_data(ability_data);
                
                // 根據英雄等級設置技能等級（訓練模式下預設給一些技能點）
                let initial_level = if ability_id == "sniper_mode" || ability_id == "saika_reinforcements" {
                    1 // 基礎技能給 1 級
                } else {
                    0
                };
                ability.current_level = initial_level;
                
                let ability_entity = self.ecs.create_entity()
                    .with(ability)
                    .build();
                    
                // 創建對應的技能實例
                let mut skill = Skill::new(ability_id.clone(), hero_entity);
                skill.current_level = initial_level;
                skill.level_up(); // 應用技能等級特殊屬性
                
                let skill_entity = self.ecs.create_entity()
                    .with(skill)
                    .build();
                    
                log::info!("Created ability '{}' and skill instance for hero", ability_data.name);
            }
        }
    }
    
    /// 創建訓練用單位（包含敵人和小兵）
    fn create_training_enemies(&mut self, campaign_data: &CampaignData) {
        // 創建敵人單位
        let enemy_positions = [
            (800.0, 0.0),   // 800 距離處
            (1000.0, 100.0), // 1000 距離處
            (1200.0, -50.0), // 1200 距離處
        ];
        
        for (i, (x, y)) in enemy_positions.iter().enumerate() {
            if let Some(enemy_data) = campaign_data.entity.enemies.get(i % campaign_data.entity.enemies.len()) {
                let unit = Unit::from_enemy_data(enemy_data);
                let enemy_faction = Faction::new(FactionType::Enemy, 1); // 敵對陣營，隊伍1
                let unit_pos = Pos(Vec2::new(*x, *y));
                let unit_vel = Vel(Vec2::new(0.0, 0.0));
                
                // 創建單位的戰鬥屬性
                let unit_properties = CProperty {
                    hp: unit.current_hp as f32,
                    mhp: unit.max_hp as f32,
                    msd: unit.move_speed,
                    def_physic: unit.base_armor,
                    def_magic: unit.magic_resistance,
                };
                
                let unit_attack = TAttack {
                    atk_physic: Vf32::new(unit.base_damage as f32),
                    asd: Vf32::new(1.0 / unit.attack_speed), // 攻擊間隔
                    range: Vf32::new(unit.attack_range),
                    asd_count: 0.0,
                    bullet_speed: 800.0,
                };
                
                // 創建敵人圓形視野組件
                let enemy_vision = CircularVision::new(
                    unit.attack_range + 150.0, // 敵人視野比攻擊範圍大150米
                    20.0 // 敵人高度20米
                ).with_precision(360); // 標準精度視野

                let unit_entity = self.ecs.create_entity()
                    .with(unit_pos)
                    .with(unit_vel)
                    .with(unit)
                    .with(enemy_faction)
                    .with(unit_properties)
                    .with(unit_attack)
                    .with(enemy_vision)
                    .build();
                    
                log::info!("Created training enemy unit '{}' at position ({}, {})", enemy_data.name, x, y);
            }
        }
        
        // 創建訓練用小兵單位（練習假人等）
        let creep_positions = [
            (600.0, 50.0),   // 近距離假人
            (1500.0, 0.0),   // 遠距離假人
            (1300.0, 150.0), // 側翼假人
        ];
        
        for (i, (x, y)) in creep_positions.iter().enumerate() {
            if let Some(creep_data) = campaign_data.entity.creeps.get(i % campaign_data.entity.creeps.len()) {
                let unit = Unit::from_creep_data(creep_data);
                let creep_faction = Faction::new(FactionType::Enemy, 2); // 敵對陣營，但不同隊伍
                let unit_pos = Pos(Vec2::new(*x, *y));
                let unit_vel = Vel(Vec2::new(0.0, 0.0));
                
                // 創建單位的戰鬥屬性
                let unit_properties = CProperty {
                    hp: unit.current_hp as f32,
                    mhp: unit.max_hp as f32,
                    msd: unit.move_speed,
                    def_physic: unit.base_armor,
                    def_magic: unit.magic_resistance,
                };
                
                let unit_attack = TAttack {
                    atk_physic: Vf32::new(unit.base_damage as f32),
                    asd: Vf32::new(1.0 / unit.attack_speed),
                    range: Vf32::new(unit.attack_range),
                    asd_count: 0.0,
                    bullet_speed: 600.0,
                };
                
                let unit_entity = self.ecs.create_entity()
                    .with(unit_pos)
                    .with(unit_vel)
                    .with(unit)
                    .with(creep_faction)
                    .with(unit_properties)
                    .with(unit_attack)
                    .build();
                    
                log::info!("Created training creep unit '{}' at position ({}, {})", creep_data.name, x, y);
            }
        }
    }
    
    /// 創建地形遮擋物實體
    fn create_terrain_blockers(&mut self) {
        // 根據地形高度圖配置創建遮擋物
        // 暫時註解掉舊的視野遮擋物系統
        // 新的圓形視野系統將使用不同的遮擋物表示方式
        /*
        let terrain_blockers = vec![
            // 大樹
            ObstacleInfo {
                position: Vec2::new(350.0, 1200.0),
                obstacle_type: ObstacleType::Circular { radius: 80.0 },
                height: 250.0,
                properties: ObstacleProperties {
                    blocks_completely: false,
                    opacity: 0.8,
                    shadow_multiplier: 2.0,
                }
            },
            // 其他遮擋物將在新系統實現後添加
        ];
        */
        
        log::info!("Terrain blockers creation skipped (old system), will be implemented in new circular vision system");
    }
    
    fn create_test_scene(&mut self) {
        let count = 0;
        // 移除不必要的 Vec<Outcome> 借用，避免與其他系統衝突
        /*
        let mut ocs = self.ecs.get_mut::<Vec<Outcome>>().unwrap();
        for x in (0..200).step_by(100) {
            for y in (0..200).step_by(100) {
                count += 1;
                ocs.push(Outcome::Tower { pos: Vec2::new(x as f32+200., y as f32+200.),
                    td: TowerData {
                    tpty: TProperty::new(10, 3, 100.),
                    tatk: TAttack::new(3., 1., 300., 100.),
                } });
            }    
        }
        */
        log::warn!("count {}", count);
    }
    
    /// Get a reference to the internal ECS world.
    pub fn ecs(&self) -> &specs::World { &self.ecs }

    /// Get a mutable reference to the internal ECS world.
    pub fn ecs_mut(&mut self) -> &mut specs::World { &mut self.ecs }

    pub fn thread_pool(&self) -> &Arc<ThreadPool> { &self.thread_pool }

    /// Get the current in-game time of day.
    ///
    /// Note that this should not be used for physics, animations or other such
    /// localised timings.
    pub fn get_time_of_day(&self) -> f64 { self.ecs.read_resource::<TimeOfDay>().0 }

    /// Get the current in-game day period (period of the day/night cycle)
    /// Get the current in-game day period (period of the day/night cycle)
    pub fn get_day_period(&self) -> DayPeriod { self.get_time_of_day().into() }

    /// Get the current in-game time.
    ///
    /// Note that this does not correspond to the time of day.
    pub fn get_time(&self) -> f64 { self.ecs.read_resource::<Time>().0 }

    /// Get the current delta time.
    pub fn get_delta_time(&self) -> f32 { self.ecs.read_resource::<DeltaTime>().0 }

    /// Given mutable access to the resource R, assuming the resource
    /// component exists (this is already the behavior of functions like `fetch`
    /// and `write_component_ignore_entity_dead`).  Since all of our resources
    /// are generated up front, any failure here is definitely a code bug.
    pub fn mut_resource<R: Resource>(&mut self) -> &mut R {
        self.ecs.get_mut::<R>().expect(
            "Tried to fetch an invalid resource even though all our resources should be known at \
             compile time.",
        )
    }


    pub fn send_chat(&mut self, msg: String) {

    }

    pub fn tick(&mut self, dt: Duration) -> Result<(), Error> {
        self.ecs.write_resource::<Tick>().0 += 1;
        self.ecs.write_resource::<TickStart>().0 = Instant::now();
        self.ecs.write_resource::<TimeOfDay>().0 += dt.as_secs_f64() * DAY_CYCLE_FACTOR;
        self.ecs.write_resource::<Time>().0 += dt.as_secs_f64();
        self.ecs.write_resource::<DeltaTime>().0 = dt.as_secs_f32().min(MAX_DELTA_TIME);
        
        let mut dispatch_builder = DispatcherBuilder::new().with_pool(Arc::clone(&self.thread_pool));
        
        // 第一階段：不需要 Vec<Outcome> 的系統，可以並行執行
        dispatch::<nearby_tick::Sys>(&mut dispatch_builder, &[]);
        dispatch::<player_tick::Sys>(&mut dispatch_builder, &[]);
        
        // 視野系統：在遊戲邏輯之前更新 (暫時註解掉直到實現完整)
        // dispatch::<VisionSystem>(&mut dispatch_builder, &["nearby_sys", "player_sys"]);
        
        // 第二階段：需要 Vec<Outcome> 的系統，按依賴順序執行
        dispatch::<projectile_tick::Sys>(&mut dispatch_builder, &["nearby_sys", "player_sys"]);
        dispatch::<tower_tick::Sys>(&mut dispatch_builder, &["projectile_sys"]);
        dispatch::<hero_tick::Sys>(&mut dispatch_builder, &["tower_sys"]);
        dispatch::<skill_tick::Sys>(&mut dispatch_builder, &["hero_sys"]);
        dispatch::<creep_tick::Sys>(&mut dispatch_builder, &["skill_sys"]);
        dispatch::<creep_wave::Sys>(&mut dispatch_builder, &["creep_sys"]);
        dispatch::<damage_tick::Sys>(&mut dispatch_builder, &["creep_wave_sys"]);
        dispatch::<death_tick::Sys>(&mut dispatch_builder, &["damage_sys"]);
        
        // 戰爭迷霧整合系統：在所有其他系統完成後處理事件 (暫時註解掉)
        // dispatch::<FogOfWarIntegrationSystem>(&mut dispatch_builder, &["death_sys"]);

        let mut dispatcher = dispatch_builder.build();
        dispatcher.dispatch(&self.ecs);

        self.creep_wave();
        self.process_outcomes();
        self.process_playerdatas();
        self.ecs.maintain();
        Ok(())
    }
    
    pub fn handle_screen_request(&mut self, pd: PlayerData) -> Result<(), Error> {
        MqttHandler::handle_screen_request(&mut self.ecs, &self.mqtx, pd)
    }
    // Screen area data method moved to mqtt_handler.rs module
    pub fn handle_tower(&mut self, pd: PlayerData) -> Result<(), Error> {
        MqttHandler::handle_tower(&mut self.ecs, &self.mqtx, pd)
    }
    pub fn handle_player(&mut self, pd: PlayerData) -> Result<(), Error> {
        MqttHandler::handle_player(&mut self.ecs, &self.mqtx, pd)
    }
    pub fn process_playerdatas(&mut self) -> Result<(), Error> {
        MqttHandler::process_playerdatas(&mut self.ecs, &self.mqtx, &self.mqrx)
    }
    pub fn process_outcomes(&mut self) -> Result<(), Error> {
        GameProcessor::process_outcomes(&mut self.ecs, &self.mqtx)
    }
    pub fn creep_wave(&mut self) -> Result<(), Error> {
        Ok(())
    }
}