/// 遊戲狀態核心結構

use std::sync::Arc;
use rayon::ThreadPool;
use specs::World;
use crossbeam_channel::{Receiver, Sender};
use failure::Error;
use core::time::Duration;

use crate::{comp::*, msg::MqttMsg};
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
}

impl State {
    /// 創建新的遊戲狀態（標準模式）
    pub fn new(
        creep_wave_data: CreepWaveData,
        mqtx: Sender<MqttMsg>,
        mqrx: Receiver<PlayerData>,
    ) -> Self {
        let thread_pool = StateInitializer::create_thread_pool();
        let ecs = StateInitializer::setup_standard_ecs_world(&thread_pool);
        
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
        };
        
        state.initialize_standard_game();
        state
    }

    /// 創建新的遊戲狀態（戰役模式）
    pub fn new_with_campaign(
        campaign_data: CampaignData,
        mqtx: Sender<MqttMsg>,
        mqrx: Receiver<PlayerData>,
    ) -> Self {
        let thread_pool = StateInitializer::create_thread_pool();
        let ecs = StateInitializer::setup_campaign_ecs_world(&thread_pool);
        
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
        };
        
        state.initialize_campaign_game(&campaign_data);
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
        
        // 維護 ECS
        self.ecs.maintain();
        
        Ok(())
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