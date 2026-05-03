/// 遊戲狀態核心結構

use std::sync::Arc;
use rayon::ThreadPool;
use specs::{World, WorldExt};
use crossbeam_channel::{Receiver, Sender};
use failure::Error;
use core::time::Duration;
use std::time::Instant;

use crate::{comp::*, CreepWave};
use crate::ue4::import_map::CreepWaveData;
use crate::ue4::import_campaign::CampaignData;
use crate::scripting::{self, ScriptRegistry};
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
    /// 上次 hero.stats 廣播的遊戲時間（UI 面板 buff 倒數用）
    last_hero_stats_time: f64,
    /// hero.stats 廣播間隔（秒）；每這麼久前端就更新一次含 buff 的 snapshot
    hero_stats_interval: f64,
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
    /// Per-player diff cache: `entity_id → last_sent_quantized_hp`. Heartbeat
    /// only re-emits HP entries where the quantized value differs from the
    /// cached one. Pruned each tick to the entities currently in AOI so the
    /// map can't grow unboundedly. Cleared on `ViewportMsg::Remove`.
    #[cfg(any(feature = "grpc", feature = "kcp"))]
    hb_last_hp_sent: HashMap<String, HashMap<u32, i32>>,
    /// Per-player force-send timestamp: `game_time` of the last heartbeat we
    /// emitted regardless of diff state. Used to drive the keepalive
    /// (`HEARTBEAT_FORCE_SEND_INTERVAL`) so clients still receive `tick`/
    /// `game_time` for clock sync even in idle periods with no HP change.
    #[cfg(any(feature = "grpc", feature = "kcp"))]
    hb_last_full_send: HashMap<String, f64>,
    /// State-local tick counter, incremented every call to `tick()`.
    /// Used to throttle visibility diff (don't rely on ECS `Tick`, which isn't maintained).
    local_tick: u64,
    /// Value of `local_tick` when visibility diff last ran
    last_visibility_tick: u64,
    /// Loaded native script DLLs (H1 — process-lifetime, never reloaded).
    script_registry: ScriptRegistry,
    /// P5: shared AOI broadphase grid. Rebuilt per tick from the same
    /// pre-gathered (id, pos) pass the heartbeat already uses. Transport
    /// broadcast thread reads it for `BroadcastPolicy::AoiEntity` lookups.
    /// `None` for non-kcp builds (mqtt/grpc don't drive AOI broadphase).
    #[cfg(feature = "kcp")]
    aoi_grid: Option<std::sync::Arc<std::sync::Mutex<crate::aoi::AoiGrid>>>,
    /// Phase 3.4: optional outbound channel that publishes a freshly computed
    /// ECS state hash every `STATE_HASH_INTERVAL_TICKS` dispatcher ticks. The
    /// `lockstep::TickBroadcaster` (60Hz) `try_recv`s on this on its own
    /// state-hash interval. `None` when running without lockstep enabled
    /// (mqtt/grpc builds, or kcp builds where main.rs hasn't wired it).
    #[cfg(feature = "kcp")]
    state_hash_tx: Option<crossbeam_channel::Sender<crate::lockstep::tick_broadcaster::StateHashSample>>,
    /// Phase 5.3: shared snapshot store for observer rejoin. Dispatcher writes
    /// every `SNAPSHOT_INTERVAL_TICKS` ticks; KCP transport's 0x16 SnapshotResp
    /// handler reads. `None` when main.rs hasn't wired the Arc (legacy /
    /// non-lockstep builds — KCP transport falls back to empty bytes).
    #[cfg(feature = "kcp")]
    snapshot_store: Option<std::sync::Arc<std::sync::Mutex<crate::comp::SnapshotStore>>>,
    /// Phase 5.x bridge: receiver paired with `TickBroadcaster::host_input_tx`.
    /// Each broadcaster tick that drains inputs from `InputBuffer` for a
    /// `TickBatch` also sends a copy down this channel; `State::tick` drains
    /// it and writes the inputs into `PendingPlayerInputs` so the host's
    /// `player_input_tick::Sys` sees them too. Without this, host runs at
    /// 30Hz dispatcher tick while broadcaster runs at 60Hz with its own
    /// counter — `drain_for_tick(my_local_tick)` never matches the keys
    /// inputs were stored under.
    #[cfg(feature = "kcp")]
    host_input_rx: Option<crossbeam_channel::Receiver<Vec<(u32, crate::lockstep::PlayerInput)>>>,
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

/// Force-send a (possibly empty) heartbeat at least this often per player so
/// clients still receive a `tick`/`game_time` heartbeat for clock sync and
/// liveness even when no HP value has changed in the player's AOI. Empty
/// heartbeats compress to ~50 bytes after prost+LZ4 — cheap keepalive.
#[cfg(any(feature = "grpc", feature = "kcp"))]
const HEARTBEAT_FORCE_SEND_INTERVAL: f64 = 5.0;

/// Phase 3.4: emit one state-hash sample every N dispatcher ticks. Dispatcher
/// runs at 30Hz so 300 = 10s — broadcaster's `state_hash_interval` default
/// is 600 (10s @ 60Hz), so the channel always has a fresh sample by the time
/// the broadcaster's interval fires (with at most one tick of staleness).
#[cfg(feature = "kcp")]
const STATE_HASH_INTERVAL_TICKS: u64 = 300;

/// Phase 5.3: serialize a fresh world snapshot every N dispatcher ticks.
/// Dispatcher runs at 30Hz so 900 = 30s — observer rejoin gets at most a
/// 30 s gap between snapshot capture and bootstrap. Skipped on `tick=0`
/// (let the world finish init before the first capture).
#[cfg(feature = "kcp")]
const SNAPSHOT_INTERVAL_TICKS: u64 = 900;

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
            heartbeat_interval: 0.5,
            last_hero_stats_time: 0.0,
            hero_stats_interval: 0.3,
            #[cfg(any(feature = "grpc", feature = "kcp"))]
            query_rx,
            #[cfg(any(feature = "grpc", feature = "kcp"))]
            viewport_rx,
            #[cfg(any(feature = "grpc", feature = "kcp"))]
            client_viewports: HashMap::new(),
            #[cfg(any(feature = "grpc", feature = "kcp"))]
            client_visibility: HashMap::new(),
            #[cfg(any(feature = "grpc", feature = "kcp"))]
            hb_last_hp_sent: HashMap::new(),
            #[cfg(any(feature = "grpc", feature = "kcp"))]
            hb_last_full_send: HashMap::new(),
            local_tick: 0,
            last_visibility_tick: 0,
            script_registry: ScriptRegistry::new(),
            #[cfg(feature = "kcp")]
            aoi_grid: None,
            #[cfg(feature = "kcp")]
            state_hash_tx: None,
            #[cfg(feature = "kcp")]
            snapshot_store: None,
            #[cfg(feature = "kcp")]
            host_input_rx: None,
        };

        state.initialize_standard_game();
        state.load_scripts();

        // Phase 5.2: legacy 0x02 heartbeat broadcast cut. Lockstep TickBatch
        // (0x10) handles client liveness via per-tick state_hash.

        state
    }

    /// 載入所有 native 腳本 DLL。目錄由環境變數 `OMB_SCRIPTS_DIR` 指定，
    /// 未設定時預設 `./scripts`（相對於執行目錄）。載入完就順便把塔 template
    /// 從腳本 `tower_metadata()` 收集到 `TowerTemplateRegistry` resource。
    ///
    /// Phase 3.2: extracted populate_* helpers into
    /// `state::initialization::{populate_tower_template_registry,
    /// populate_tower_upgrade_registry, populate_ability_registry}` so
    /// the omfx sim_runner can reuse the same bootstrap code.
    fn load_scripts(&mut self) {
        let dir_str = std::env::var("OMB_SCRIPTS_DIR").unwrap_or_else(|_| "./scripts".to_string());
        let dir = std::path::Path::new(&dir_str);
        self.script_registry = crate::scripting::loader::load_scripts_dir(dir);
        super::initialization::populate_tower_template_registry(&mut self.ecs, &self.script_registry);
        super::initialization::populate_tower_upgrade_registry(&mut self.ecs);
        super::initialization::populate_ability_registry(&mut self.ecs, &self.script_registry);
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
            heartbeat_interval: 0.5,
            last_hero_stats_time: 0.0,
            hero_stats_interval: 0.3,
            #[cfg(any(feature = "grpc", feature = "kcp"))]
            query_rx,
            #[cfg(any(feature = "grpc", feature = "kcp"))]
            viewport_rx,
            #[cfg(any(feature = "grpc", feature = "kcp"))]
            client_viewports: HashMap::new(),
            #[cfg(any(feature = "grpc", feature = "kcp"))]
            client_visibility: HashMap::new(),
            #[cfg(any(feature = "grpc", feature = "kcp"))]
            hb_last_hp_sent: HashMap::new(),
            #[cfg(any(feature = "grpc", feature = "kcp"))]
            hb_last_full_send: HashMap::new(),
            local_tick: 0,
            last_visibility_tick: 0,
            script_registry: ScriptRegistry::new(),
            #[cfg(feature = "kcp")]
            aoi_grid: None,
            #[cfg(feature = "kcp")]
            state_hash_tx: None,
            #[cfg(feature = "kcp")]
            snapshot_store: None,
            #[cfg(feature = "kcp")]
            host_input_rx: None,
        };

        // 先載 scripts，才能讓 initialize_campaign_game 內的 send_tower_templates 拿到 registry
        state.load_scripts();
        state.initialize_campaign_game(&campaign_data);

        // Phase 5.2: legacy 0x02 GameEvent broadcast cut.

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

        // Phase 5.x bridge: pull all pending broadcaster-drained inputs into
        // PendingPlayerInputs so player_input_tick::Sys can route StartRound
        // (and future commands). Drains all available batches in this tick to
        // catch up if the host runs at lower TPS than the 60Hz broadcaster.
        #[cfg(feature = "kcp")]
        if let Some(rx) = self.host_input_rx.as_ref() {
            let mut accumulated: Vec<(u32, crate::lockstep::PlayerInput)> = Vec::new();
            while let Ok(batch) = rx.try_recv() {
                accumulated.extend(batch);
            }
            if !accumulated.is_empty() {
                use crate::comp::PendingPlayerInputs;
                let mut pending = self.ecs.write_resource::<PendingPlayerInputs>();
                pending.tick = self.local_tick as u32;
                pending.by_player.clear();
                for (player_id, input) in accumulated {
                    pending.by_player.insert(player_id, input);
                }
            }
        }

        // 運行遊戲系統
        let t_run = Instant::now();
        self.system_dispatcher.run_systems(&self.ecs)?;
        let run_systems_ns = t_run.elapsed().as_nanos();

        // Phase 2.1: drain `PendingTowerSpawnQueue` filled by
        // `player_input_tick::Sys` during the dispatch above. Needs `&mut World`
        // (TowerTemplateRegistry lookup + entity create + ScriptEvent::Spawn
        // push) which a specs `System` can't borrow. Replica (omfx sim_runner)
        // mirrors this call after its own dispatcher run.
        crate::comp::GameProcessor::drain_pending_tower_spawns(&mut self.ecs);

        // Phase 2.2: drain `PendingTowerSellQueue` (TowerSell lockstep input)
        // — same `&mut World` requirement (Gold credit + BuffStore clear +
        // entity delete). Replica mirrors this in sim_runner.
        crate::comp::GameProcessor::drain_pending_tower_sells(&mut self.ecs);

        // Phase 2.3: drain `PendingTowerUpgradeQueue` (TowerUpgrade lockstep
        // input) — needs `&mut World` (TowerUpgradeRegistry read, validate
        // via tower_upgrade_rules, deduct Gold, write Tower.upgrade_levels +
        // upgrade_flags, push StatMod into BuffStore). Replica mirrors this
        // in sim_runner.
        crate::comp::GameProcessor::drain_pending_tower_upgrades(&mut self.ecs);

        // Phase 2.4: drain `PendingItemUseQueue` (ItemUse lockstep input) —
        // needs `&mut World` (ItemRegistry read, write Inventory cooldown,
        // write CProperty for item effects). Replica mirrors this in
        // sim_runner.
        crate::comp::GameProcessor::drain_pending_item_uses(&mut self.ecs);

        // 腳本 dispatch 階段（E1 — 序列、獨佔 World）
        // 放在並行系統之後、其他序列處理之前，確保腳本能看到本 tick 的
        // 完整戰鬥結果，也能修改狀態讓下游處理看見。
        let t_dispatch = Instant::now();
        let dt_fx = omoba_template_ids::Fixed64::from_raw((dt.as_secs_f32() * 1024.0) as i64);
        scripting::run_script_dispatch(
            &mut self.ecs,
            &self.script_registry,
            self.local_tick,
            dt_fx,
            self.mqtx.clone(),
        );
        let script_dispatch_ns = t_dispatch.elapsed().as_nanos();

        // 處理小兵波
        self.resource_manager.process_creep_waves(&mut self.ecs)?;

        // 處理遊戲結果
        let t_outcomes = Instant::now();
        self.resource_manager.process_outcomes(&mut self.ecs)?;
        let process_outcomes_ns = t_outcomes.elapsed().as_nanos();

        {
            use crate::comp::{TickPhase, TickProfile};
            let mut profile = self.ecs.write_resource::<TickProfile>();
            profile.record_phase(TickPhase::RunSystems, run_systems_ns);
            profile.record_phase(TickPhase::ScriptDispatch, script_dispatch_ns);
            profile.record_phase(TickPhase::ProcessOutcomes, process_outcomes_ns);
            profile.finish_tick_and_maybe_log();
        }

        // 處理玩家資料
        self.resource_manager.process_player_data(&mut self.ecs, &self.mqrx)?;

        // 處理 MCP 查詢請求
        #[cfg(any(feature = "grpc", feature = "kcp"))]
        self.process_queries();

        // Phase 5.2: legacy 0x02 GameEvent broadcast cut. Lockstep TickBatch
        // (0x10) carries heartbeat / hero stats / visibility diff equivalents.

        // 維護 ECS
        self.ecs.maintain();

        // Phase 3.4: publish a deterministic ECS state hash every
        // STATE_HASH_INTERVAL_TICKS dispatcher ticks (30Hz cadence). The
        // 60Hz lockstep TickBroadcaster pulls the latest sample on its
        // own state-hash interval (default 10s @ 60Hz), so a fresh sample
        // is always pending. Skipped when state_hash_tx is None (legacy /
        // non-lockstep builds).
        #[cfg(feature = "kcp")]
        if self.local_tick % STATE_HASH_INTERVAL_TICKS == 0 {
            if let Some(tx) = &self.state_hash_tx {
                let hash = crate::lockstep::compute_state_hash(&self.ecs);
                // u32 wrap matches the proto StateHash.tick field.
                let tick_u32 = self.local_tick as u32;
                if let Err(e) = tx.send((tick_u32, hash)) {
                    log::warn!("State: failed to publish state hash: {e}");
                }
            }
        }

        // Phase 5.3: serialize a fresh world snapshot for observer rejoin
        // every SNAPSHOT_INTERVAL_TICKS dispatcher ticks (= 30 s @ 30 Hz).
        // Skip tick 0 — the very first dispatch tick may run before all
        // populate_* helpers have finished filling the registry, so wait
        // until at least one full tick of game state has settled.
        // Writes go to (1) the SnapshotStore ECS resource (always — query
        // path) and (2) the optional `snapshot_store` Arc<Mutex<>> when
        // wired by main.rs (the KCP transport reads from this).
        #[cfg(feature = "kcp")]
        if self.local_tick > 0 && self.local_tick % SNAPSHOT_INTERVAL_TICKS == 0 {
            let bytes = crate::lockstep::serialize_snapshot(&self.ecs);
            let tick_u32 = self.local_tick as u32;
            let byte_len = bytes.len();
            // Update the ECS resource first (cheap — same dispatcher thread).
            {
                let mut store = self.ecs.write_resource::<crate::comp::SnapshotStore>();
                store.tick = tick_u32;
                store.bytes = bytes.clone();
            }
            // Mirror to the shared Arc<Mutex<>> when transport is wired.
            // `lock().unwrap()` is OK: the transport-side reader holds the
            // lock for microseconds (clone + drop) and never panics under
            // normal operation. A poisoned mutex here is unrecoverable.
            if let Some(shared) = &self.snapshot_store {
                let mut guard = shared.lock().expect("SnapshotStore mutex poisoned");
                guard.tick = tick_u32;
                guard.bytes = bytes;
            }
            log::info!("[snapshot] saved tick={} bytes={}", tick_u32, byte_len);
        }

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
                    // Drop the player's heartbeat diff cache so a future
                    // reconnect starts from a clean slate (full snapshot on
                    // the first tick after rejoin — `prev` is None for every
                    // entity → all included).
                    self.hb_last_hp_sent.remove(&player_name);
                    self.hb_last_full_send.remove(&player_name);
                }
            }
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
                "list_abilities" => query::query_list_abilities(&self.ecs),
                "get_ability_detail" => query::query_get_ability_detail(&self.ecs, &req.player_name),
                other => crate::transport::QueryResponse {
                    success: false,
                    error: format!("Unknown query_type: {}", other),
                    data_json: Vec::new(),
                },
            };
            let _ = req.response_tx.send(response);
        }
    }

    /// P5: plug in the shared `AoiGrid` from the KCP transport. State will
    /// rebuild the grid each heartbeat tick using the same (id, pos) pre-gather
    /// that builds the heartbeat snapshot. Safe to call once after
    /// `TransportHandle` is obtained.
    #[cfg(feature = "kcp")]
    pub fn attach_aoi_grid(&mut self, grid: std::sync::Arc<std::sync::Mutex<crate::aoi::AoiGrid>>) {
        self.aoi_grid = Some(grid);
    }

    /// Phase 3.4: register the dispatcher → broadcaster state-hash channel.
    /// Called from `main.rs` after creating both the State and the
    /// `TickBroadcaster`'s receiver. If never called, hash publishing is a
    /// no-op and the broadcaster falls back to its placeholder.
    #[cfg(feature = "kcp")]
    pub fn set_state_hash_tx(
        &mut self,
        tx: crossbeam_channel::Sender<crate::lockstep::tick_broadcaster::StateHashSample>,
    ) {
        self.state_hash_tx = Some(tx);
    }

    /// Phase 5.3: register the shared snapshot store. The dispatcher tick
    /// loop will mirror its periodic `serialize_snapshot` output into this
    /// `Arc<Mutex<>>` so the KCP transport's 0x16 SnapshotResp handler
    /// (running in a tokio task — no direct World access) can serve real
    /// bytes. If never called, snapshots still update the ECS resource
    /// (queryable) but the transport sees empty bytes.
    #[cfg(feature = "kcp")]
    pub fn attach_snapshot_store(
        &mut self,
        store: std::sync::Arc<std::sync::Mutex<crate::comp::SnapshotStore>>,
    ) {
        self.snapshot_store = Some(store);
    }

    /// Phase 5.x bridge: register the host input receiver paired with
    /// `TickBroadcaster::with_host_input_tx`. Each `tick()` drains pending
    /// per-tick input vecs and writes them into the ECS `PendingPlayerInputs`
    /// resource, which `player_input_tick::Sys` then routes to game-side
    /// handlers (StartRound flips CurrentCreepWave.is_running, etc.).
    #[cfg(feature = "kcp")]
    pub fn attach_host_input_rx(
        &mut self,
        rx: crossbeam_channel::Receiver<Vec<(u32, crate::lockstep::PlayerInput)>>,
    ) {
        self.host_input_rx = Some(rx);
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
        // 動態實體建完後再填 Region blockers（Searcher 索引一次性完成）
        StateInitializer::populate_region_blockers(&mut self.ecs);
        // Phase 5.2: legacy 0x02 GameEvent broadcast cut. tower_templates
        // 仍在 — 前端 TD placement UI 需要 cost/footprint/label。
        self.send_tower_templates();
    }

    fn initialize_campaign_game(&mut self, campaign_data: &CampaignData) {
        StateInitializer::init_campaign_data(&mut self.ecs, campaign_data);
        StateInitializer::init_creep_wave(&mut self.ecs, &self.cw);
        StateInitializer::create_campaign_scene(&mut self.ecs, campaign_data);
        StateInitializer::populate_region_blockers(&mut self.ecs);

        // Phase 5.2: legacy 0x02 GameEvent broadcast cut. tower_templates 保留。
        self.send_tower_templates();
    }
    

    /// 收集 script registry 內每支塔腳本的 tower_metadata，合併 host TowerTemplate
    /// 的 cost/footprint/label，廣播 `game/tower_templates` 給前端。
    fn send_tower_templates(&self) {
        use serde_json::json;
        let reg = self.ecs.read_resource::<crate::comp::tower_registry::TowerTemplateRegistry>();
        let mut templates: Vec<serde_json::Value> = Vec::new();
        // 依 DLL units() 註冊順序 broadcast（Q2 作者意圖優先）
        for tpl in reg.iter_ordered() {
            templates.push(json!({
                "kind": tpl.unit_id,
                "label": tpl.label,
                "cost": tpl.cost,
                "footprint": tpl.footprint,
                "atk": tpl.atk,
                "asd_interval": tpl.asd_interval,
                "range": tpl.range,
                "bullet_speed": tpl.bullet_speed,
                "splash_radius": tpl.splash_radius,
                "hit_radius": tpl.hit_radius,
                "slow_factor": tpl.slow_factor,
                "slow_duration": tpl.slow_duration,
            }));
        }
        let n = templates.len();
        let payload = json!({ "templates": templates });
        let _ = self.mqtx.send(OutboundMsg::new_s(
            "td/all/res", "game", "tower_templates", payload,
        ));
        log::info!("已發送 {} 個 tower template 給前端", n);
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