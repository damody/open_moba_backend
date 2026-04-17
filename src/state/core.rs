/// 遊戲狀態核心結構

use std::sync::Arc;
use rayon::ThreadPool;
use specs::{World, WorldExt};
use crossbeam_channel::{Receiver, Sender};
use failure::Error;
use core::time::Duration;

use crate::{comp::*, CreepWave};
use crate::ue4::import_map::CreepWaveData;
use crate::ue4::import_campaign::CampaignData;
use crate::transport::{OutboundMsg, InboundMsg};
#[cfg(any(feature = "grpc", feature = "kcp"))]
use crate::transport::{QueryRequest, Viewport, ViewportMsg};
#[cfg(any(feature = "grpc", feature = "kcp"))]
use std::collections::{HashMap, HashSet};
use std::collections::BTreeMap;

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
    mqtx: Sender<OutboundMsg>,
    /// 玩家資料接收通道
    mqrx: Receiver<InboundMsg>,
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
    /// 查詢請求接收通道（gRPC/KCP）
    #[cfg(any(feature = "grpc", feature = "kcp"))]
    query_rx: Receiver<QueryRequest>,
    /// Viewport 更新接收通道（來自 transport）
    #[cfg(any(feature = "grpc", feature = "kcp"))]
    viewport_rx: Receiver<ViewportMsg>,
    /// 每個已連線玩家目前的 viewport
    #[cfg(any(feature = "grpc", feature = "kcp"))]
    client_viewports: HashMap<String, Viewport>,
    /// 每個玩家最後一次已知可見的實體集合（分四類避免 entity id 重用衝突）
    #[cfg(any(feature = "grpc", feature = "kcp"))]
    client_visibility: HashMap<String, VisSet>,
    /// State-local tick counter, incremented every call to `tick()`.
    /// Used to throttle visibility diff (don't rely on ECS `Tick`, which isn't maintained).
    local_tick: u64,
    /// Value of `local_tick` when visibility diff last ran
    last_visibility_tick: u64,
}

/// Per-player visible entity sets, split by type so that specs `Entity::id()`
/// reuse across different storages doesn't collide inside a single `HashSet<u32>`.
#[cfg(any(feature = "grpc", feature = "kcp"))]
#[derive(Default, Debug)]
struct VisSet {
    heroes: HashSet<u32>,
    units: HashSet<u32>,
    creeps: HashSet<u32>,
    towers: HashSet<u32>,
}

#[cfg(any(feature = "grpc", feature = "kcp"))]
const VISIBILITY_DIFF_INTERVAL_TICKS: u64 = 6;

impl State {
    /// 創建新的遊戲狀態（標準模式）
    pub fn new(
        creep_wave_data: CreepWaveData,
        mqtx: Sender<OutboundMsg>,
        mqrx: Receiver<InboundMsg>,
        #[cfg(any(feature = "grpc", feature = "kcp"))] query_rx: Receiver<QueryRequest>,
        #[cfg(any(feature = "grpc", feature = "kcp"))] viewport_rx: Receiver<ViewportMsg>,
    ) -> Self {
        let thread_pool = StateInitializer::create_thread_pool();
        let mut ecs = StateInitializer::setup_standard_ecs_world(&thread_pool);

        // 設置 MQTT 發送器
        {
            let mut mqtx_vec = ecs.write_resource::<Vec<Sender<OutboundMsg>>>();
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
            heartbeat_interval: 2.0,
            #[cfg(any(feature = "grpc", feature = "kcp"))]
            query_rx,
            #[cfg(any(feature = "grpc", feature = "kcp"))]
            viewport_rx,
            #[cfg(any(feature = "grpc", feature = "kcp"))]
            client_viewports: HashMap::new(),
            #[cfg(any(feature = "grpc", feature = "kcp"))]
            client_visibility: HashMap::new(),
            local_tick: 0,
            last_visibility_tick: 0,
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
        mqtx: Sender<OutboundMsg>,
        mqrx: Receiver<InboundMsg>,
        #[cfg(any(feature = "grpc", feature = "kcp"))] query_rx: Receiver<QueryRequest>,
        #[cfg(any(feature = "grpc", feature = "kcp"))] viewport_rx: Receiver<ViewportMsg>,
    ) -> Self {
        let thread_pool = StateInitializer::create_thread_pool();
        let mut ecs = StateInitializer::setup_campaign_ecs_world(&thread_pool);

        // 設置 MQTT 發送器
        {
            let mut mqtx_vec = ecs.write_resource::<Vec<Sender<OutboundMsg>>>();
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
            heartbeat_interval: 2.0,
            #[cfg(any(feature = "grpc", feature = "kcp"))]
            query_rx,
            #[cfg(any(feature = "grpc", feature = "kcp"))]
            viewport_rx,
            #[cfg(any(feature = "grpc", feature = "kcp"))]
            client_viewports: HashMap::new(),
            #[cfg(any(feature = "grpc", feature = "kcp"))]
            client_visibility: HashMap::new(),
            local_tick: 0,
            last_visibility_tick: 0,
        };

        state.initialize_campaign_game(&campaign_data);

        // 立即發送初始心跳，讓前端知道後端已啟動
        state.send_heartbeat();
        log::info!("📡 初始心跳已發送，後端準備就緒");

        state
    }

    /// 遊戲主循環 tick
    pub fn tick(&mut self, dt: Duration) -> Result<(), Error> {
        self.local_tick = self.local_tick.wrapping_add(1);

        // 更新時間管理
        self.time_manager.update(&mut self.ecs, dt)?;

        // 吸收 transport 傳進來的 viewport 更新
        #[cfg(any(feature = "grpc", feature = "kcp"))]
        self.drain_viewport_updates();

        // 運行遊戲系統
        self.system_dispatcher.run_systems(&self.ecs)?;

        // 處理小兵波
        self.resource_manager.process_creep_waves(&mut self.ecs)?;

        // 處理遊戲結果
        self.resource_manager.process_outcomes(&mut self.ecs)?;

        // 處理玩家資料
        self.resource_manager.process_player_data(&mut self.ecs, &self.mqrx)?;

        // 處理 MCP 查詢請求
        #[cfg(any(feature = "grpc", feature = "kcp"))]
        self.process_queries();

        // 發送心跳（每 2 秒一次，只有 counter）
        self.send_heartbeat_if_needed();

        // 依視野對每個 session 送 C/D diff。必須在 ecs.maintain() 前，
        // 這樣本 tick 死亡的實體還在 storage 裡，diff 才能正確判斷「離開」。
        #[cfg(any(feature = "grpc", feature = "kcp"))]
        self.compute_and_send_visibility_diffs();

        // 維護 ECS
        self.ecs.maintain();

        Ok(())
    }

    /// Drain viewport updates from transport layer. Called each tick.
    #[cfg(any(feature = "grpc", feature = "kcp"))]
    fn drain_viewport_updates(&mut self) {
        while let Ok(msg) = self.viewport_rx.try_recv() {
            match msg {
                ViewportMsg::Set { player_name, viewport } => {
                    log::info!("📥 [State] ViewportMsg::Set player='{}' padded=({}, {})",
                        player_name, viewport.padded_hw, viewport.padded_hh);
                    self.client_viewports.insert(player_name, viewport);
                }
                ViewportMsg::Remove { player_name } => {
                    log::info!("📥 [State] ViewportMsg::Remove player='{}'", player_name);
                    self.client_viewports.remove(&player_name);
                    self.client_visibility.remove(&player_name);
                }
            }
        }
    }

    /// Per-session visibility diff: send `C` for entities that just entered
    /// each player's viewport and `D` for entities that just left.
    ///
    /// The first diff for a freshly-subscribed client naturally produces a full
    /// snapshot (old set is empty → everything currently visible is "entered").
    /// Players that haven't sent a viewport yet are skipped entirely (anti-cheat).
    #[cfg(any(feature = "grpc", feature = "kcp"))]
    fn compute_and_send_visibility_diffs(&mut self) {
        use specs::Join;

        // Throttle using local_tick (ECS Tick resource is not maintained)
        let tick = self.local_tick;
        if tick.wrapping_sub(self.last_visibility_tick) < VISIBILITY_DIFF_INTERVAL_TICKS {
            return;
        }
        self.last_visibility_tick = tick;

        if self.client_viewports.is_empty() {
            log::trace!("👁 [diff tick={}] skipped: no client viewports", tick);
            return;
        }

        let entities = self.ecs.entities();
        let heroes = self.ecs.read_storage::<Hero>();
        let units = self.ecs.read_storage::<Unit>();
        let creeps = self.ecs.read_storage::<Creep>();
        let towers = self.ecs.read_storage::<Tower>();
        let positions = self.ecs.read_storage::<Pos>();
        let properties = self.ecs.read_storage::<CProperty>();
        let paths = self.ecs.try_fetch::<BTreeMap<String, Path>>();

        // Pre-collect the world's broadcastable entities once.
        #[derive(Copy, Clone, Debug)]
        enum Kind { Hero, Unit, Creep, Tower }
        let mut world: Vec<(specs::Entity, f32, f32, Kind)> = Vec::new();
        for (e, _, p) in (&entities, &heroes, &positions).join()  { world.push((e, p.0.x, p.0.y, Kind::Hero)); }
        for (e, _, p) in (&entities, &units, &positions).join()   { world.push((e, p.0.x, p.0.y, Kind::Unit)); }
        for (e, _, p) in (&entities, &creeps, &positions).join()  { world.push((e, p.0.x, p.0.y, Kind::Creep)); }
        for (e, _, p) in (&entities, &towers, &positions).join()  { world.push((e, p.0.x, p.0.y, Kind::Tower)); }

        log::info!("👁 [diff tick={}] world={} entities, players={}",
            tick, world.len(), self.client_viewports.len());

        // Iterate players into a staging buffer, then write back (avoid mut-while-iter).
        let mut updates: Vec<(String, VisSet)> = Vec::with_capacity(self.client_viewports.len());

        for (player_name, vp) in &self.client_viewports {
            let default = VisSet::default();
            let old = self.client_visibility.get(player_name).unwrap_or(&default);
            let mut new_set = VisSet::default();
            let topic = format!("td/{}/res", player_name);
            let mut entered_count = 0u32;
            let mut exited_count = 0u32;

            // Compute newly-visible and emit C events
            for &(e, x, y, kind) in &world {
                let in_vp = vp.contains(x, y);
                log::trace!("  entity={} kind={:?} pos=({}, {}) in_vp={}", e.id(), kind, x, y, in_vp);
                if !in_vp { continue; }
                let id = e.id();
                let (new_target, old_target) = match kind {
                    Kind::Hero  => (&mut new_set.heroes,  &old.heroes),
                    Kind::Unit  => (&mut new_set.units,   &old.units),
                    Kind::Creep => (&mut new_set.creeps,  &old.creeps),
                    Kind::Tower => (&mut new_set.towers,  &old.towers),
                };
                new_target.insert(id);
                if old_target.contains(&id) { continue; }
                entered_count += 1;

                // Entered viewport → emit C with correct payload for kind
                let prop = properties.get(e);
                let (type_tag, payload) = match kind {
                    Kind::Hero => {
                        let Some(h) = heroes.get(e) else { continue };
                        let Some(p) = positions.get(e) else { continue };
                        ("hero", build_hero_payload(e, h, p, prop))
                    }
                    Kind::Unit => {
                        let Some(u) = units.get(e) else { continue };
                        let Some(p) = positions.get(e) else { continue };
                        ("unit", build_unit_payload(e, u, p, prop))
                    }
                    Kind::Creep => {
                        let Some(c) = creeps.get(e) else { continue };
                        let Some(p) = positions.get(e) else { continue };
                        ("creep", build_creep_payload(e, c, p, prop, paths.as_deref()))
                    }
                    Kind::Tower => {
                        let Some(t) = towers.get(e) else { continue };
                        let Some(p) = positions.get(e) else { continue };
                        ("tower", build_tower_payload(e, t, p, prop))
                    }
                };
                let _ = self.mqtx.send(OutboundMsg::new_s_at(
                    &topic, type_tag, "create", payload, x, y,
                ));
            }

            // Emit D for entities that left the viewport (old - new) per-kind.
            // Use `new_s` (no position) so the transport viewport filter doesn't
            // drop a D for an entity that happens to be outside the viewport now.
            for &id in old.heroes.difference(&new_set.heroes) {
                exited_count += 1;
                let _ = self.mqtx.send(OutboundMsg::new_s(
                    &topic, "hero", "D",
                    serde_json::json!({ "id": id, "entity_id": id }),
                ));
            }
            for &id in old.units.difference(&new_set.units) {
                exited_count += 1;
                let _ = self.mqtx.send(OutboundMsg::new_s(
                    &topic, "unit", "D",
                    serde_json::json!({ "id": id, "entity_id": id }),
                ));
            }
            for &id in old.creeps.difference(&new_set.creeps) {
                exited_count += 1;
                let _ = self.mqtx.send(OutboundMsg::new_s(
                    &topic, "creep", "D",
                    serde_json::json!({ "id": id, "entity_id": id }),
                ));
            }
            for &id in old.towers.difference(&new_set.towers) {
                exited_count += 1;
                let _ = self.mqtx.send(OutboundMsg::new_s(
                    &topic, "tower", "D",
                    serde_json::json!({ "id": id, "entity_id": id }),
                ));
            }

            if entered_count > 0 || exited_count > 0 {
                log::info!("👁 [diff] player='{}' topic='{}' vp=(cx={}, cy={}, phw={}, phh={}) entered={} exited={} visible={}/{}/{}/{}",
                    player_name, topic, vp.cx, vp.cy, vp.padded_hw, vp.padded_hh,
                    entered_count, exited_count,
                    new_set.heroes.len(), new_set.units.len(), new_set.creeps.len(), new_set.towers.len());
            }

            updates.push((player_name.clone(), new_set));
        }

        // Commit visibility state
        for (name, set) in updates {
            self.client_visibility.insert(name, set);
        }
    }

    /// 處理來自 MCP server 的查詢請求
    #[cfg(any(feature = "grpc", feature = "kcp"))]
    fn process_queries(&self) {
        use super::query;
        while let Ok(req) = self.query_rx.try_recv() {
            let response = match req.query_type.as_str() {
                "list_players" => query::query_list_players(&self.ecs),
                "inspect_player_view" => query::query_inspect_player_view(&self.ecs, &req.player_name),
                other => crate::transport::QueryResponse {
                    success: false,
                    error: format!("Unknown query_type: {}", other),
                    data_json: Vec::new(),
                },
            };
            let _ = req.response_tx.send(response);
        }
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
        let properties = self.ecs.read_storage::<CProperty>();
        let towers = self.ecs.read_storage::<Tower>();

        let hero_count = (&entities, &heroes).join().count();
        let unit_count = (&entities, &units).join().count();
        let creep_count = (&entities, &creeps).join().count();
        let entity_count = hero_count + unit_count + creep_count;

        // 取得當前 tick 數
        let tick = self.ecs.read_resource::<Tick>().0;

        // 所有帶 HP 的實體的 authoritative 快照，讓前端每 2 秒校正預測值。
        let mut hp_snapshot: Vec<serde_json::Value> = Vec::new();
        for (e, _, p) in (&entities, &heroes, &properties).join() {
            hp_snapshot.push(json!({ "id": e.id(), "hp": p.hp, "max_hp": p.mhp }));
        }
        for (e, _, p) in (&entities, &units, &properties).join() {
            hp_snapshot.push(json!({ "id": e.id(), "hp": p.hp, "max_hp": p.mhp }));
        }
        for (e, _, p) in (&entities, &creeps, &properties).join() {
            hp_snapshot.push(json!({ "id": e.id(), "hp": p.hp, "max_hp": p.mhp }));
        }
        for (e, _, p) in (&entities, &towers, &properties).join() {
            hp_snapshot.push(json!({ "id": e.id(), "hp": p.hp, "max_hp": p.mhp }));
        }

        let heartbeat_data = json!({
            "tick": tick,
            "game_time": self.time_manager.get_time(),
            "entity_count": entity_count,
            "hero_count": hero_count,
            "unit_count": unit_count,
            "creep_count": creep_count,
            "render_delay_ms": crate::config::server_config::CONFIG.RENDER_DELAY_MS,
            "hp_snapshot": hp_snapshot,
        });

        if let Err(e) = self.mqtx.send(OutboundMsg::new_s("td/all/res", "heartbeat", "tick", heartbeat_data)) {
            log::error!("無法發送心跳訊息: {}", e);
        } else {
            log::trace!("💓 心跳已發送 - tick: {}, entities: {}", tick, entity_count);
        }

        // 實體 create/delete 事件改由 compute_and_send_visibility_diffs 依視野產生，
        // heartbeat 只保留 counter/liveness
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
    pub fn handle_tower(&mut self, pd: InboundMsg) -> Result<(), Error> {
        self.resource_manager.handle_tower_request(&mut self.ecs, pd)
    }

    /// 處理玩家相關請求
    pub fn handle_player(&mut self, pd: InboundMsg) -> Result<(), Error> {
        self.resource_manager.handle_player_request(&mut self.ecs, pd)
    }

    /// 處理畫面請求
    pub fn handle_screen_request(&mut self, pd: InboundMsg) -> Result<(), Error> {
        self.resource_manager.handle_screen_request(&mut self.ecs, pd)
    }

    // 私有初始化方法
    fn initialize_standard_game(&mut self) {
        StateInitializer::init_creep_wave(&mut self.ecs, &self.cw);
        StateInitializer::create_test_scene(&mut self.ecs);
        self.send_initial_game_state();
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

            for (entity, hero, pos) in (&entities, &heroes, &positions).join() {
                let payload = build_hero_payload(entity, hero, pos, properties.get(entity));
                if let Err(e) = self.mqtx.send(OutboundMsg::new_s_at(
                    "td/all/res", "hero", "create", payload, pos.0.x, pos.0.y,
                )) {
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

            for (entity, unit, pos) in (&entities, &units, &positions).join() {
                let payload = build_unit_payload(entity, unit, pos, properties.get(entity));
                if let Err(e) = self.mqtx.send(OutboundMsg::new_s_at(
                    "td/all/res", "unit", "create", payload, pos.0.x, pos.0.y,
                )) {
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
            
            if let Err(e) = self.mqtx.send(OutboundMsg::new_s("td/all/res", "creep_wave", "init", wave_data)) {
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

            if let Err(e) = self.mqtx.send(OutboundMsg::new_s("td/all/res", "campaign", "init", campaign_info)) {
                log::error!("無法發送戰役初始化資料: {}", e);
            }
            log::info!("已發送戰役 '{}' 初始化資料到 MQTT", campaign.mission.campaign.name);
        }
    }
}

// ---- Entity payload builders (shared by initial-state and visibility-diff) ----

fn build_hero_payload(
    entity: specs::Entity,
    hero: &Hero,
    pos: &Pos,
    prop: Option<&CProperty>,
) -> serde_json::Value {
    let (hp, mhp, msd) = prop
        .map(|p| (p.hp, p.mhp, p.msd))
        .unwrap_or((100.0, 100.0, 0.0));
    serde_json::json!({
        "entity_id": entity.id(),
        "hero_id": hero.id,
        "name": hero.name,
        "title": hero.title,
        "level": hero.level,
        "position": { "x": pos.0.x, "y": pos.0.y },
        "hp": hp,
        "max_hp": mhp,
        "move_speed": msd,
    })
}

fn build_unit_payload(
    entity: specs::Entity,
    unit: &Unit,
    pos: &Pos,
    prop: Option<&CProperty>,
) -> serde_json::Value {
    let (hp, mhp, msd) = prop
        .map(|p| (p.hp, p.mhp, p.msd))
        .unwrap_or((unit.current_hp as f32, unit.max_hp as f32, unit.move_speed));
    serde_json::json!({
        "entity_id": entity.id(),
        "unit_id": unit.id,
        "name": unit.name,
        "unit_type": unit.unit_type,
        "position": { "x": pos.0.x, "y": pos.0.y },
        "hp": hp,
        "max_hp": mhp,
        "move_speed": msd,
    })
}

fn build_creep_payload(
    entity: specs::Entity,
    creep: &Creep,
    pos: &Pos,
    prop: Option<&CProperty>,
    paths: Option<&BTreeMap<String, Path>>,
) -> serde_json::Value {
    let (hp, mhp, msd) = prop
        .map(|p| (p.hp, p.mhp, p.msd))
        .unwrap_or((0.0, 0.0, 0.0));
    let display_name = creep.label.clone().unwrap_or_else(|| creep.name.clone());
    // 輸出從 creep 當前 checkpoint 起到終點的剩餘 waypoints，供前端 debug 畫線
    let path_points: Vec<serde_json::Value> = paths
        .and_then(|m| m.get(&creep.path))
        .map(|p| {
            p.check_points
                .iter()
                .skip(creep.pidx)
                .map(|cp| serde_json::json!({ "x": cp.pos.x, "y": cp.pos.y }))
                .collect()
        })
        .unwrap_or_default();
    serde_json::json!({
        "entity_id": entity.id(),
        "id": entity.id(),
        "name": display_name,
        "position": { "x": pos.0.x, "y": pos.0.y },
        "hp": hp,
        "max_hp": mhp,
        "move_speed": msd,
        "path_name": creep.path,
        "path_points": path_points,
    })
}

fn build_tower_payload(
    entity: specs::Entity,
    _tower: &Tower,
    pos: &Pos,
    prop: Option<&CProperty>,
) -> serde_json::Value {
    let (hp, mhp) = prop.map(|p| (p.hp, p.mhp)).unwrap_or((100.0, 100.0));
    serde_json::json!({
        "entity_id": entity.id(),
        "id": entity.id(),
        "position": { "x": pos.0.x, "y": pos.0.y },
        "hp": hp,
        "max_hp": mhp,
    })
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