/// 遊戲狀態核心結構

use std::sync::Arc;
use rayon::ThreadPool;
use specs::{World, WorldExt};
use crossbeam_channel::{Receiver, Sender};
use failure::Error;
use core::time::Duration;

use crate::{comp::*, msg::MqttMsg, CreepWave};
use crate::ue4::import_map::CreepWaveData;
use crate::ue4::import_campaign::CampaignData;
use crate::msg::PlayerData;

use super::{
    StateInitializer, TimeManager, ResourceManager, SystemDispatcher
};

/// 遊戲核心狀態
pub struct State {
    /// ECS 世界
    ecs: World,
    /// 小兵波資料
    cw: CreepWaveData,
    /// 戰役資料（可選）
    campaign: Option<CampaignData>,
    /// MQTT 發送通道
    mqtx: Sender<MqttMsg>,
    /// 玩家資料接收通道
    mqrx: Receiver<PlayerData>,
    /// 執行緒池
    thread_pool: Arc<ThreadPool>,
    /// 時間管理器
    time_manager: TimeManager,
    /// 資源管理器
    resource_manager: ResourceManager,
    /// 系統分派器
    system_dispatcher: SystemDispatcher,
    /// 上次心跳發送的遊戲時間
    last_heartbeat_time: f64,
    /// 心跳間隔（秒）
    heartbeat_interval: f64,
}

impl State {
    /// 創建新的遊戲狀態（標準模式）
    pub fn new(
        creep_wave_data: CreepWaveData,
        mqtx: Sender<MqttMsg>,
        mqrx: Receiver<PlayerData>,
    ) -> Self {
        let thread_pool = StateInitializer::create_thread_pool();
        let mut ecs = StateInitializer::setup_standard_ecs_world(&thread_pool);
        
        // 設置 MQTT 發送器
        {
            let mut mqtx_vec = ecs.write_resource::<Vec<Sender<MqttMsg>>>();
            mqtx_vec.push(mqtx.clone());
        }
        
        let mut state = Self {
            ecs,
            cw: creep_wave_data,
            campaign: None,
            mqtx: mqtx.clone(),
            mqrx: mqrx.clone(),
            thread_pool: thread_pool.clone(),
            time_manager: TimeManager::new(),
            resource_manager: ResourceManager::new(mqtx),
            system_dispatcher: SystemDispatcher::new(thread_pool),
            last_heartbeat_time: 0.0,
            heartbeat_interval: 2.0, // 每 2 秒發送一次心跳
        };

        state.initialize_standard_game();

        // 立即發送初始心跳，讓前端知道後端已啟動
        state.send_heartbeat();
        log::info!("📡 初始心跳已發送，後端準備就緒");

        state
    }

    /// 創建新的遊戲狀態（戰役模式）
    pub fn new_with_campaign(
        campaign_data: CampaignData,
        mqtx: Sender<MqttMsg>,
        mqrx: Receiver<PlayerData>,
    ) -> Self {
        let thread_pool = StateInitializer::create_thread_pool();
        let mut ecs = StateInitializer::setup_campaign_ecs_world(&thread_pool);
        
        // 設置 MQTT 發送器
        {
            let mut mqtx_vec = ecs.write_resource::<Vec<Sender<MqttMsg>>>();
            mqtx_vec.push(mqtx.clone());
        }
        
        let mut state = Self {
            ecs,
            cw: campaign_data.map.clone(),
            campaign: Some(campaign_data.clone()),
            mqtx: mqtx.clone(),
            mqrx: mqrx.clone(),
            thread_pool: thread_pool.clone(),
            time_manager: TimeManager::new(),
            resource_manager: ResourceManager::new(mqtx),
            system_dispatcher: SystemDispatcher::new(thread_pool),
            last_heartbeat_time: 0.0,
            heartbeat_interval: 2.0, // 每 2 秒發送一次心跳
        };

        state.initialize_campaign_game(&campaign_data);

        // 立即發送初始心跳，讓前端知道後端已啟動
        state.send_heartbeat();
        log::info!("📡 初始心跳已發送，後端準備就緒");

        state
    }

    /// 遊戲主循環 tick
    pub fn tick(&mut self, dt: Duration) -> Result<(), Error> {
        // 更新時間管理
        self.time_manager.update(&mut self.ecs, dt)?;

        // 運行遊戲系統
        self.system_dispatcher.run_systems(&self.ecs)?;

        // 處理小兵波
        self.resource_manager.process_creep_waves(&mut self.ecs)?;

        // 處理遊戲結果
        self.resource_manager.process_outcomes(&mut self.ecs)?;

        // 處理玩家資料
        self.resource_manager.process_player_data(&mut self.ecs, &self.mqrx)?;

        // 發送心跳（每 2 秒一次）
        self.send_heartbeat_if_needed();

        // 維護 ECS
        self.ecs.maintain();

        Ok(())
    }

    /// 檢查並發送心跳
    fn send_heartbeat_if_needed(&mut self) {
        let current_time = self.time_manager.get_time();

        if current_time - self.last_heartbeat_time >= self.heartbeat_interval {
            self.send_heartbeat();
            self.last_heartbeat_time = current_time;
        }
    }

    /// 發送心跳訊息到 MQTT
    fn send_heartbeat(&self) {
        use specs::Join;
        use serde_json::json;

        // 統計實體數量
        let entities = self.ecs.entities();
        let heroes = self.ecs.read_storage::<Hero>();
        let units = self.ecs.read_storage::<Unit>();
        let creeps = self.ecs.read_storage::<Creep>();

        let hero_count = (&entities, &heroes).join().count();
        let unit_count = (&entities, &units).join().count();
        let creep_count = (&entities, &creeps).join().count();
        let entity_count = hero_count + unit_count + creep_count;

        // 取得當前 tick 數
        let tick = self.ecs.read_resource::<Tick>().0;

        let heartbeat_data = json!({
            "tick": tick,
            "game_time": self.time_manager.get_time(),
            "entity_count": entity_count,
            "hero_count": hero_count,
            "unit_count": unit_count,
            "creep_count": creep_count
        });

        if let Err(e) = self.mqtx.send(MqttMsg::new_s("td/all/res", "heartbeat", "tick", heartbeat_data)) {
            log::error!("無法發送心跳訊息: {}", e);
        } else {
            log::trace!("💓 心跳已發送 - tick: {}, entities: {}", tick, entity_count);
        }
    }

    /// 獲取 ECS 世界引用
    pub fn ecs(&self) -> &World {
        &self.ecs
    }

    /// 獲取 ECS 世界可變引用
    pub fn ecs_mut(&mut self) -> &mut World {
        &mut self.ecs
    }

    /// 獲取執行緒池
    pub fn thread_pool(&self) -> &Arc<ThreadPool> {
        &self.thread_pool
    }

    /// 獲取時間資訊
    pub fn get_time_of_day(&self) -> f64 {
        self.time_manager.get_time_of_day()
    }

    /// 獲取遊戲時間
    pub fn get_time(&self) -> f64 {
        self.time_manager.get_time()
    }

    /// 獲取增量時間
    pub fn get_delta_time(&self) -> f32 {
        self.time_manager.get_delta_time()
    }

    /// 獲取當前日期週期
    pub fn get_day_period(&self) -> DayPeriod {
        self.time_manager.get_day_period()
    }

    /// 取得資源的可變引用
    pub fn mut_resource<R: specs::prelude::Resource>(&mut self) -> &mut R {
        self.ecs.get_mut::<R>().expect(
            "Tried to fetch an invalid resource even though all our resources should be known at compile time."
        )
    }

    /// 發送聊天消息
    pub fn send_chat(&mut self, msg: String) {
        // 實現聊天功能
        log::info!("Chat message: {}", msg);
    }

    /// 處理塔相關請求
    pub fn handle_tower(&mut self, pd: PlayerData) -> Result<(), Error> {
        self.resource_manager.handle_tower_request(&mut self.ecs, pd)
    }

    /// 處理玩家相關請求
    pub fn handle_player(&mut self, pd: PlayerData) -> Result<(), Error> {
        self.resource_manager.handle_player_request(&mut self.ecs, pd)
    }

    /// 處理畫面請求
    pub fn handle_screen_request(&mut self, pd: PlayerData) -> Result<(), Error> {
        self.resource_manager.handle_screen_request(&mut self.ecs, pd)
    }

    // 私有初始化方法
    fn initialize_standard_game(&mut self) {
        StateInitializer::init_creep_wave(&mut self.ecs, &self.cw);
        StateInitializer::create_test_scene(&mut self.ecs);
    }

    fn initialize_campaign_game(&mut self, campaign_data: &CampaignData) {
        StateInitializer::init_campaign_data(&mut self.ecs, campaign_data);
        StateInitializer::init_creep_wave(&mut self.ecs, &self.cw);
        StateInitializer::create_campaign_scene(&mut self.ecs, campaign_data);
        
        // 發送初始化資料到 MQTT
        self.send_initial_game_state();
    }
    
    /// 發送初始遊戲狀態到 MQTT
    fn send_initial_game_state(&mut self) {
        use specs::Join;
        use serde_json::json;
        
        // 發送英雄資料
        {
            let entities = self.ecs.entities();
            let heroes = self.ecs.read_storage::<Hero>();
            let positions = self.ecs.read_storage::<Pos>();
            let properties = self.ecs.read_storage::<CProperty>();
            
            for (entity, hero, pos, prop) in (&entities, &heroes, &positions, &properties).join() {
                let hero_data = json!({
                    "entity_id": entity.id(),
                    "hero_id": hero.id,
                    "name": hero.name,
                    "title": hero.title,
                    "level": hero.level,
                    "position": {
                        "x": pos.0.x,
                        "y": pos.0.y
                    },
                    "hp": prop.hp,
                    "max_hp": prop.mhp,
                    "move_speed": prop.msd
                });
                
                if let Err(e) = self.mqtx.send(MqttMsg::new_s("td/all/res", "hero", "create", hero_data)) {
                    log::error!("無法發送英雄初始化資料: {}", e);
                }
                log::info!("已發送英雄 '{}' 初始化資料到 MQTT", hero.name);
            }
        }
        
        // 發送敵人單位資料
        {
            let entities = self.ecs.entities();
            let units = self.ecs.read_storage::<Unit>();
            let positions = self.ecs.read_storage::<Pos>();
            let properties = self.ecs.read_storage::<CProperty>();
            
            for (entity, unit, pos, prop) in (&entities, &units, &positions, &properties).join() {
                let unit_data = json!({
                    "entity_id": entity.id(),
                    "unit_id": unit.id,
                    "name": unit.name,
                    "unit_type": unit.unit_type,
                    "position": {
                        "x": pos.0.x,
                        "y": pos.0.y
                    },
                    "hp": prop.hp,
                    "max_hp": prop.mhp,
                    "move_speed": prop.msd
                });
                
                if let Err(e) = self.mqtx.send(MqttMsg::new_s("td/all/res", "unit", "create", unit_data)) {
                    log::error!("無法發送單位初始化資料: {}", e);
                }
                log::info!("已發送單位 '{}' 初始化資料到 MQTT", unit.name);
            }
        }
        
        // 發送小兵波資料
        {
            let creep_waves = self.ecs.read_resource::<Vec<CreepWave>>();
            let wave_data = json!({
                "total_waves": creep_waves.len(),
                "waves": creep_waves.iter().map(|wave| {
                    json!({
                        "start_time": wave.time,
                        "paths": wave.path_creeps.iter().map(|pc| {
                            json!({
                                "path": pc.path_name,
                                "creep_count": pc.creeps.len()
                            })
                        }).collect::<Vec<_>>()
                    })
                }).collect::<Vec<_>>()
            });
            
            if let Err(e) = self.mqtx.send(MqttMsg::new_s("td/all/res", "creep_wave", "init", wave_data)) {
                log::error!("無法發送小兵波初始化資料: {}", e);
            }
            log::info!("已發送 {} 個小兵波初始化資料到 MQTT", creep_waves.len());
        }
        
        // 發送戰役資訊
        if let Some(campaign) = &self.campaign {
            let campaign_info = json!({
                "campaign_id": campaign.mission.campaign.id,
                "campaign_name": campaign.mission.campaign.name,
                "hero_id": campaign.mission.campaign.hero_id,
                "stages": campaign.mission.stages.len(),
                "abilities": campaign.ability.abilities.len()
            });
            
            if let Err(e) = self.mqtx.send(MqttMsg::new_s("td/all/res", "campaign", "init", campaign_info)) {
                log::error!("無法發送戰役初始化資料: {}", e);
            }
            log::info!("已發送戰役 '{}' 初始化資料到 MQTT", campaign.mission.campaign.name);
        }
    }
}

/// 遊戲狀態配置
#[derive(Debug, Clone)]
pub struct StateConfig {
    /// 執行緒數量
    pub thread_count: Option<usize>,
    /// 日夜循環倍率
    pub day_cycle_factor: f64,
    /// 最大增量時間
    pub max_delta_time: f32,
    /// 是否啟用戰役模式
    pub campaign_mode: bool,
}

impl Default for StateConfig {
    fn default() -> Self {
        Self {
            thread_count: None,
            day_cycle_factor: 24.0,
            max_delta_time: 1.0,
            campaign_mode: false,
        }
    }
}