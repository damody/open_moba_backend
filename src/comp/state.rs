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
use std::time::{Instant};
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
            ecs: Self::setup_ecs_world(&thread_pool),
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
            ecs: Self::setup_ecs_world_with_campaign(&thread_pool),
            cw: campaign_data.map.clone(),
            campaign: Some(campaign_data.clone()),
            mqtx: mqtx.clone(),
            mqrx: mqrx.clone(),
            thread_pool,
        };
        res.init_campaign_data(&campaign_data);
        res.init_creep_wave();
        res.create_campaign_scene(&campaign_data);
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
    fn setup_ecs_world(thread_pool: &Arc<ThreadPool>) -> specs::World {
        let mut ecs = specs::World::new();
        // Register all components.
        ecs.register::<Pos>();
        ecs.register::<Vel>();
        ecs.register::<TProperty>();
        ecs.register::<TAttack>();
        ecs.register::<CProperty>();
        ecs.register::<Tower>();
        ecs.register::<Creep>();
        ecs.register::<Projectile>();
        // Register unsynced resources used by the ECS.
        ecs.insert(TimeOfDay(0.0));
        ecs.insert(Time(0.0));
        ecs.insert(DeltaTime(0.0));
        ecs.insert(Tick(0));
        ecs.insert(TickStart(Instant::now()));
        ecs.insert(SysMetrics::default());
        ecs.insert(Vec::<Outcome>::new());
        ecs.insert(Vec::<TakenDamage>::new());
        ecs.insert(Vec::<CreepWave>::new());
        ecs.insert(CurrentCreepWave{wave: 0, path: vec![]});
        ecs.insert(BTreeMap::<String, Player>::new());
        ecs.insert(BTreeMap::<String, CreepEmiter>::new());
        ecs.insert(BTreeMap::<String, Path>::new());
        ecs.insert(BTreeMap::<String, CheckPoint>::new());
        ecs.insert(Searcher::default());
        let e = ecs.entities_mut().create();

        // Set starting time for the server.
        ecs.write_resource::<TimeOfDay>().0 = 0.0;
        ecs
    }
    
    /// 設置支援戰役的 ECS 世界
    fn setup_ecs_world_with_campaign(thread_pool: &Arc<ThreadPool>) -> specs::World {
        let mut ecs = Self::setup_ecs_world(thread_pool);
        
        // 註冊戰役相關組件
        ecs.register::<Hero>();
        ecs.register::<Ability>();
        ecs.register::<AbilityEffect>();
        ecs.register::<Enemy>();
        ecs.register::<Campaign>();
        ecs.register::<Stage>();
        ecs.register::<Unit>();
        ecs.register::<Faction>();
        ecs.register::<DamageInstance>();
        ecs.register::<DamageResult>();
        ecs.register::<Skill>();
        ecs.register::<SkillEffect>();
        
        // 戰役相關資源
        ecs.insert(BTreeMap::<String, Hero>::new());
        ecs.insert(BTreeMap::<String, Ability>::new());
        ecs.insert(BTreeMap::<String, Enemy>::new());
        ecs.insert(Vec::<AbilityEffect>::new());
        ecs.insert(Vec::<DamageInstance>::new());
        ecs.insert(Vec::<SkillInput>::new());
        // ecs.insert(AbilityBridge::new());  // 暫時註解掉舊的AbilityBridge
        
        ecs
    }
    
    /// 初始化戰役資料到 ECS
    fn init_campaign_data(&mut self, campaign_data: &CampaignData) {
        log::info!("Initializing campaign data for: {}", campaign_data.mission.campaign.name);
        
        // 初始化英雄
        let mut heroes = self.ecs.get_mut::<BTreeMap<String, Hero>>().unwrap();
        for hero_data in &campaign_data.entity.heroes {
            let hero = Hero::from_campaign_data(hero_data);
            log::info!("Loading hero: {} - {}", hero.name, hero.title);
            heroes.insert(hero.id.clone(), hero);
        }
        
        // 初始化技能
        let mut abilities = self.ecs.get_mut::<BTreeMap<String, Ability>>().unwrap();
        for (ability_id, ability_data) in &campaign_data.ability.abilities {
            let ability = Ability::from_campaign_data(ability_data);
            log::info!("Loading ability: {} ({})", ability.name, ability.key_binding);
            abilities.insert(ability_id.clone(), ability);
        }
        
        // 初始化敵人
        let mut enemies = self.ecs.get_mut::<BTreeMap<String, Enemy>>().unwrap();
        for enemy_data in &campaign_data.entity.enemies {
            let enemy = Enemy::from_campaign_data(enemy_data);
            log::info!("Loading enemy: {} ({})", enemy.name, enemy.id);
            enemies.insert(enemy.id.clone(), enemy);
        }
        
        // 創建戰役組件
        let campaign = Campaign::from_campaign_data(&campaign_data.mission.campaign);
        let campaign_entity = self.ecs.create_entity().with(campaign).build();
        
        // 創建關卡組件
        for stage_data in &campaign_data.mission.stages {
            let stage = Stage::from_campaign_data(stage_data, campaign_data.mission.campaign.id.clone());
            let stage_entity = self.ecs.create_entity().with(stage).build();
            log::info!("Loading stage: {} ({})", stage_data.name, stage_data.id);
        }
        
        log::info!("Campaign initialization completed");
    }
    
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
    
    /// 創建戰役場景
    fn create_campaign_scene(&mut self, campaign_data: &CampaignData) {
        log::info!("Creating campaign scene for: {}", campaign_data.mission.campaign.name);
        
        // 根據戰役類型創建相應的場景
        match campaign_data.mission.campaign.difficulty.as_str() {
            "tutorial" => self.create_tutorial_scene(campaign_data),
            _ => self.create_training_scene(campaign_data),
        }
    }
    
    /// 創建教學場景
    fn create_tutorial_scene(&mut self, campaign_data: &CampaignData) {
        log::info!("Setting up tutorial scene");
        // 教學場景特殊設置
    }
    
    /// 創建訓練場景
    fn create_training_scene(&mut self, campaign_data: &CampaignData) {
        log::info!("Setting up training scene for sniper practice");
        
        // 創建主要英雄實體
        if let Some(hero_data) = campaign_data.entity.heroes.first() {
            let hero = Hero::from_campaign_data(hero_data);
            
            // 創建英雄的戰鬥屬性
            let hero_properties = self.create_hero_properties(&hero, hero_data);
            let hero_attack = self.create_hero_attack(&hero, hero_data);
            
            // 創建英雄的 Unit 組件
            let hero_unit = Unit {
                id: hero.id.clone(),
                name: hero.name.clone(),
                unit_type: UnitType::Hero,
                max_hp: hero.get_max_hp() as i32,
                current_hp: hero.get_max_hp() as i32,
                base_armor: hero_data.base_armor,
                magic_resistance: 0.0,
                base_damage: hero.get_base_damage() as i32,
                attack_range: hero_data.attack_range,
                move_speed: hero.get_move_speed(),
                attack_speed: hero.get_attack_speed_multiplier(),
                ai_type: unit::AiType::None, // 英雄由玩家控制，不需要AI
                aggro_range: hero_data.attack_range + 200.0,
                abilities: hero_data.abilities.clone(),
                current_target: None,
                last_attack_time: 0.0,
                spawn_position: (0.0, 0.0),
                exp_reward: 0,
                gold_reward: 0,
                bounty_type: BountyType::None,
            };
            
            let hero_faction = Faction::new(FactionType::Player, 0); // 玩家陣營，隊伍0
            
            // 英雄起始位置
            let hero_pos = Pos(Vec2::new(0.0, 0.0));
            let hero_vel = Vel(Vec2::new(0.0, 0.0));
            
            let hero_entity = self.ecs.create_entity()
                .with(hero_pos)
                .with(hero_vel)
                .with(hero)
                .with(hero_unit)
                .with(hero_faction)
                .with(hero_properties)
                .with(hero_attack)
                .build();
                
            log::info!("Created hero entity '{}' with full combat components", hero_data.name);
            
            // 初始化英雄的技能實體（舊系統）
            self.create_hero_abilities(hero_entity, &hero_data.abilities, campaign_data);
            
            // 註冊到新的ability系統 (暫時註解掉)
            // self.register_hero_abilities_to_new_system(hero_entity, &hero_data.abilities);
            
            // 創建訓練用敵人
            self.create_training_enemies(campaign_data);
        }
    }
    
    /// 創建英雄屬性組件
    fn create_hero_properties(&self, hero: &Hero, hero_data: &crate::ue4::import_campaign::HeroJD) -> CProperty {
        let max_hp = hero.get_max_hp();
        let move_speed = hero.get_move_speed();
        
        CProperty {
            hp: max_hp,
            mhp: max_hp,
            msd: move_speed,
            def_physic: hero_data.base_armor,
            def_magic: 0.0, // 基礎魔抗為 0
        }
    }
    
    /// 創建英雄攻擊組件
    fn create_hero_attack(&self, hero: &Hero, hero_data: &crate::ue4::import_campaign::HeroJD) -> TAttack {
        let base_damage = hero.get_base_damage();
        let attack_speed_multiplier = hero.get_attack_speed_multiplier();
        let attack_interval = 1.0 / attack_speed_multiplier; // 攻擊間隔
        
        TAttack {
            atk_physic: Vf32::new(base_damage),
            asd: Vf32::new(attack_interval),
            range: Vf32::new(hero_data.attack_range),
            asd_count: 0.0,
            bullet_speed: 1000.0, // 投射物速度
        }
    }
    
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
                
                let unit_entity = self.ecs.create_entity()
                    .with(unit_pos)
                    .with(unit_vel)
                    .with(unit)
                    .with(enemy_faction)
                    .with(unit_properties)
                    .with(unit_attack)
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
        
        // 第二階段：需要 Vec<Outcome> 的系統，按依賴順序執行
        dispatch::<projectile_tick::Sys>(&mut dispatch_builder, &["nearby_sys", "player_sys"]);
        dispatch::<tower_tick::Sys>(&mut dispatch_builder, &["projectile_sys"]);
        dispatch::<hero_tick::Sys>(&mut dispatch_builder, &["tower_sys"]);
        dispatch::<skill_tick::Sys>(&mut dispatch_builder, &["hero_sys"]);
        dispatch::<creep_tick::Sys>(&mut dispatch_builder, &["skill_sys"]);
        dispatch::<creep_wave::Sys>(&mut dispatch_builder, &["creep_sys"]);
        dispatch::<damage_tick::Sys>(&mut dispatch_builder, &["creep_wave_sys"]);
        dispatch::<death_tick::Sys>(&mut dispatch_builder, &["damage_sys"]);

        let mut dispatcher = dispatch_builder.build();
        dispatcher.dispatch(&self.ecs);

        self.creep_wave();
        self.process_outcomes();
        self.process_playerdatas();
        self.ecs.maintain();
        Ok(())
    }
    pub fn handle_tower(&mut self, pd: PlayerData) -> Result<(), Error> {
        match pd.a.as_str() {
            "R" => {
                self.mqtx.try_send(MqttMsg::new_s("td/all/res", "tower", "R", json!({"msg":"ok"})))?;
            }
            "C" => {
                #[derive(Serialize, Deserialize)]
                struct JData {
                    tid: i32,
                    x: f32,
                    y: f32,
                };
                let mut v: JData = serde_json::from_value(pd.d)?;
                let t = {
                    let mut pmap = self.ecs.get_mut::<BTreeMap<String, Player>>().unwrap();
                    if let Some(p) = pmap.get_mut(&pd.name) {
                        if let Some(t) = p.towers.get(v.tid as usize) {
                            Some(t.clone())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };
                let mut ocs = self.ecs.get_mut::<Vec<Outcome>>().unwrap();
                if let Some(t) = t {
                    ocs.push(Outcome::Tower { pos: Vec2::new(v.x,v.y), td: TowerData { tpty: t.tpty, tatk: t.tatk } });
                    self.mqtx.try_send(MqttMsg::new_s("td/all/res", "tower", "C", json!({"msg":"ok"})))?;
                } else {
                    self.mqtx.try_send(MqttMsg::new_s("td/all/res", "tower", "C", json!({"msg":"fail"})))?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub fn handle_player(&mut self, pd: PlayerData) -> Result<(), Error> {
        let mut pmap = self.ecs.get_mut::<BTreeMap<String, Player>>().unwrap();
        match pd.a.as_str() {
            "C" => {
                let mut p = Player { name: pd.name.clone(), cost: 100., towers: vec![] };
                p.towers.push(TowerData { tpty: TProperty::new(10., 1, 100.), tatk: TAttack::new(3., 0.3, 300., 100.) });
                pmap.insert(pd.name.clone(), p);
                self.mqtx.try_send(MqttMsg::new_s("td/all/res", "player", "C", json!({"msg":"ok"})))?;
            }
            _ => {}
        }
        Ok(())
    }
    pub fn process_playerdatas(&mut self) -> Result<(), Error> {
        let n = self.mqrx.len();
        for i in 0..n {
            let data = self.mqrx.try_recv();
            if let Ok(d) = data {
                log::warn!("{:?}", d);
                match d.t.as_str() {
                    "tower" => {
                        self.handle_tower(d)?;
                    }
                    "player" => {
                        self.handle_player(d)?;
                    }
                    _ => {}
                }
            } else {
                log::warn!("json error");
            }
        }
        Ok(())
    }
    pub fn process_outcomes(&mut self) -> Result<(), Error> {
        let mut remove_uids = vec![];
        let mut next_outcomes = vec![];
        {
            let mut ocs = self.ecs.get_mut::<Vec<Outcome>>().unwrap();
            let mut outcomes = vec![];
            outcomes.append(ocs);
            for out in outcomes {
                match out {
                    Outcome::Death { pos: p, ent: e } => {
                        remove_uids.push(e);
                        let mut creeps = self.ecs.write_storage::<Creep>();
                        let mut towers = self.ecs.write_storage::<Tower>();
                        let mut projs = self.ecs.write_storage::<Projectile>();
                        let t = if let Some(c) = creeps.get_mut(e) {
                            if let Some(bt) = c.block_tower {
                                if let Some(t) = towers.get_mut(bt) { 
                                    t.block_creeps.retain(|&x| x != e);
                                }
                            }
                            "creep"
                        } else if let Some(t) = towers.get_mut(e) {
                            for ce in t.block_creeps.iter() {
                                if let Some(c) = creeps.get_mut(*ce) { 
                                    c.block_tower = None;
                                    next_outcomes.push(Outcome::CreepWalk { target: ce.clone() });
                                }
                            }
                            "tower"
                        } else if let Some(p) = projs.get_mut(e) {
                            "projectile"
                        } else { "" };
                        if t != "" {
                            self.mqtx.send(MqttMsg::new_s("td/all/res", t, "D", json!({"id": e.id()})));
                        }
                    }
                    Outcome::ProjectileLine2{ pos, source, target } => { 
                        let mut e1 = source.ok_or(err_msg("err"))?;
                        let mut e2 = target.ok_or(err_msg("err"))?;
                        let (msd, p2) = {
                            let positions = self.ecs.read_storage::<Pos>();
                            let tproperty = self.ecs.read_storage::<TAttack>();
                            
                            let p1 = positions.get(e1).ok_or(err_msg("err"))?;
                            let p2 = positions.get(e2).ok_or(err_msg("err"))?;
                            let tp = tproperty.get(e1).ok_or(err_msg("err"))?;
                            (tp.bullet_speed, p2.0)
                        };
                        let ntarget = if let Some(t) = target {
                            t.id()
                        } else { 0 };
                        let e = self.ecs.create_entity().with(Pos(pos))
                            .with(Projectile { time_left: 3., owner: e1.clone(), tpos: p2, target: target, radius: 0., msd: msd }).build();
                        let mut pjs = json!(ProjectileData {
                            id: e.id(), pos: pos.clone(), msd: msd,
                            time_left: 3., owner: e1.id(), target: ntarget, radius: 0.,
                        });
                        self.mqtx.try_send(MqttMsg::new_s("td/all/res", "projectile", "C", json!(pjs)));
                    }
                    Outcome::Creep { cd } => {
                        let mut cjs = json!(cd);
                        let e = self.ecs.create_entity().with(Pos(cd.pos)).with(cd.creep).with(cd.cdata).build();
                        cjs.as_object_mut().unwrap().insert("id".to_owned(), json!(e.id()));
                        self.mqtx.try_send(MqttMsg::new_s("td/all/res", "creep", "C", json!(cjs)));
                    }
                    Outcome::Tower { pos, td } => {
                        let mut cjs = json!(td);
                        let e = self.ecs.create_entity().with(Pos(pos)).with(Tower::new()).with(td.tpty).with(td.tatk).build();
                        cjs.as_object_mut().unwrap().insert("id".to_owned(), json!(e.id()));
                        cjs.as_object_mut().unwrap().insert("pos".to_owned(), json!(pos));
                        self.mqtx.try_send(MqttMsg::new_s("td/all/res", "tower", "C", json!(cjs)));
                        self.ecs.get_mut::<Searcher>().unwrap().tower.needsort = true;
                    }
                    Outcome::CreepStop { source, target } => {
                        let mut creeps = self.ecs.write_storage::<Creep>();
                        let c = creeps.get_mut(target).ok_or(err_msg("err"))?;
                        c.block_tower = Some(source);
                        c.status = CreepStatus::Stop;
                        let positions = self.ecs.read_storage::<Pos>();
                        let pos = positions.get(target).ok_or(err_msg("err"))?;
                        self.mqtx.try_send(MqttMsg::new_s("td/all/res", "creep", "M", json!({
                            "id": target.id(),
                            "x": pos.0.x,
                            "y": pos.0.y,
                        })));
                    }
                    Outcome::CreepWalk { target } => {
                        let mut creeps = self.ecs.write_storage::<Creep>();
                        let creep = creeps.get_mut(target).ok_or(err_msg("err"))?;
                        creep.status = CreepStatus::PreWalk;
                    }
                    Outcome::Damage { pos, phys, magi, real, source, target } => {
                        let mut properties = self.ecs.write_storage::<CProperty>();
                        if let Some(target_props) = properties.get_mut(target) {
                            let hp_before = target_props.hp;
                            let total_damage = phys + magi + real;
                            target_props.hp -= total_damage;
                            let hp_after = target_props.hp;
                            
                            // 獲取攻擊者和目標名稱用於日誌
                            let (source_name, target_name) = {
                                let creeps = self.ecs.read_storage::<Creep>();
                                let heroes = self.ecs.read_storage::<Hero>();
                                let units = self.ecs.read_storage::<Unit>();
                                
                                // 獲取攻擊者名稱
                                let source_name = if let Some(creep) = creeps.get(source) {
                                    creep.name.clone()
                                } else if let Some(hero) = heroes.get(source) {
                                    hero.name.clone()
                                } else if let Some(unit) = units.get(source) {
                                    unit.name.clone()
                                } else {
                                    "Unknown".to_string()
                                };
                                
                                // 獲取目標名稱
                                let target_name = if let Some(creep) = creeps.get(target) {
                                    creep.name.clone()
                                } else if let Some(hero) = heroes.get(target) {
                                    hero.name.clone()
                                } else if let Some(unit) = units.get(target) {
                                    unit.name.clone()
                                } else {
                                    "Unknown".to_string()
                                };
                                
                                (source_name, target_name)
                            };
                            
                            // 整合的攻擊傷害日誌 - 只顯示非零傷害
                            let damage_parts = {
                                let mut parts = Vec::new();
                                if phys > 0.0 { parts.push(format!("Phys {:.1}", phys)); }
                                if magi > 0.0 { parts.push(format!("Magi {:.1}", magi)); }
                                if real > 0.0 { parts.push(format!("Pure {:.1}", real)); }
                                if parts.is_empty() { 
                                    parts.push(format!("Total {:.1}", total_damage)); 
                                }
                                parts.join(", ")
                            };
                            
                            log::info!("⚔️ {} 攻擊 {} | {} damage | HP: {:.1} → {:.1}/{:.1}", 
                                source_name, target_name, damage_parts, hp_before, hp_after, target_props.mhp
                            );
                            
                            // 檢查是否死亡
                            if target_props.hp <= 0.0 {
                                target_props.hp = 0.0;
                                log::info!("💀 {} died from damage!", target_name);
                                next_outcomes.push(Outcome::Death { 
                                    pos: pos,
                                    ent: target 
                                });
                            }
                        }
                    }
                    Outcome::Heal { pos, target, amount } => {
                        let mut properties = self.ecs.write_storage::<CProperty>();
                        if let Some(target_props) = properties.get_mut(target) {
                            target_props.hp = (target_props.hp + amount).min(target_props.mhp);
                        }
                    }
                    Outcome::UpdateAttack { target, asd_count, cooldown_reset } => {
                        let mut attacks = self.ecs.write_storage::<TAttack>();
                        if let Some(attack) = attacks.get_mut(target) {
                            if let Some(new_count) = asd_count {
                                attack.asd_count = new_count;
                            }
                            if cooldown_reset {
                                attack.asd_count = attack.asd.v;
                            }
                        }
                    }
                    Outcome::GainExperience { target, amount } => {
                        let mut heroes = self.ecs.write_storage::<Hero>();
                        if let Some(hero) = heroes.get_mut(target) {
                            let leveled_up = hero.add_experience(amount);
                            if leveled_up {
                                log::info!("Hero '{}' gained {} experience and leveled up!", hero.name, amount);
                            } else {
                                log::info!("Hero '{}' gained {} experience", hero.name, amount);
                            }
                        }
                    }
                    _=>{}
                }
            }
        }
        self.ecs.delete_entities(&remove_uids[..]);
        self.ecs.write_resource::<Vec<Outcome>>().clear();
        self.ecs.write_resource::<Vec<Outcome>>().append(&mut next_outcomes);
        Ok(())
    }
    pub fn creep_wave(&mut self) -> Result<(), Error> {
        Ok(())
    }
}