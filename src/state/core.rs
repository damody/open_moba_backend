use crate::config::server_config::CONFIG;
use core::time::Duration;
use crossbeam_channel::{Receiver, Sender};
use failure::Error;
use omoba_core::lockstep_timing::LockstepTiming;
use rayon::ThreadPool;
use specs::{Join, World, WorldExt};
/// 遊戲狀態核心結構
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use crate::scripting::{self, ScriptRegistry};
use crate::transport::{InboundMsg, OutboundMsg};
#[cfg(any(feature = "grpc", feature = "kcp"))]
use crate::transport::{QueryRequest, Viewport, ViewportMsg};
use crate::ue4::import_campaign::CampaignData;
use crate::ue4::import_map::CreepWaveData;
use crate::{comp::*, CreepWave};
use std::collections::BTreeMap;
#[cfg(any(feature = "grpc", feature = "kcp"))]
use std::collections::HashMap;

use super::{ResourceManager, StateInitializer, SystemDispatcher, TimeManager};

#[cfg(feature = "kcp")]
fn reliable_send_with_watchdog<T>(
    tx: &Sender<T>,
    value: T,
    timeout: Duration,
) -> Result<Duration, crossbeam_channel::SendTimeoutError<T>> {
    let started = Instant::now();
    tx.send_timeout(value, timeout)?;
    Ok(started.elapsed())
}

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
    #[cfg(feature = "kcp")]
    reliable_team_tx: Option<Sender<OutboundMsg>>,
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
    /// 每位玩家的差異快取：`entity_id→last_sent_quantized_hp`。心跳
    /// 僅在量化值與實際值不同的情況下重新發出 HP 條目
    /// 緩存了一份。修剪目前 AOI 中實體的每個刻度，以便
    /// 地圖不能無限增長。在“ViewportMsg::Remove”上清除。
    #[cfg(any(feature = "grpc", feature = "kcp"))]
    hb_last_hp_sent: HashMap<String, HashMap<u32, i32>>,
    /// 每個玩家強制發送時間戳：我們最後一次心跳的“game_time”
    /// 無論 diff 狀態如何，都會發出。用於驅動keepalive
    /// (`HEARTBEAT_FORCE_SEND_INTERVAL`) 因此客戶端仍然會收到 `tick`/
    /// 即使在空閒期間，「game_time」也可以進行時脈同步，HP 不會改變。
    #[cfg(any(feature = "grpc", feature = "kcp"))]
    hb_last_full_send: HashMap<String, f64>,
    /// 狀態本地刻度計數器，每次呼叫 `tick()` 時都會增加。
    /// 用於限制可見性差異（不要依賴 ECS `Tick`，它不被維護）。
    local_tick: u64,
    /// Runtime lockstep cadence from backend game.toml.
    lockstep_timing: LockstepTiming,
    /// Last observed `player_profile.json` modified time for live hero knowledge reloads.
    hero_knowledge_profile_modified: Option<SystemTime>,
    /// 載入的本機腳本 DLL（H1 — 進程生命週期，從不重新載入）。
    script_registry: ScriptRegistry,
    /// DEV-only Lua content hot reload poller; disabled unless env explicitly enables it.
    #[cfg(feature = "runtime-lua-content")]
    dev_lua_hot_reload: Option<super::dev_lua_hot_reload::DevLuaHotReload>,
    /// P5：共享 AOI 寬相網格。從相同的每個蜱蟲重建
    /// 預先收集的（id，pos）傳遞已經使用的心跳。運輸
    /// 廣播線程讀取它以進行“BroadcastPolicy::AoiEntity”查找。
    /// 對於非 kcp 構建，“無”（mqtt/grpc 不驅動 AOI Broadphase）。
    #[cfg(feature = "kcp")]
    aoi_grid: Option<std::sync::Arc<std::sync::Mutex<crate::aoi::AoiGrid>>>,
    /// 階段 3.4：可選的出站通道，發布新計算的結果
    /// 每個“STATE_HASH_INTERVAL_TICKS”調度程序滴答聲的 ECS 狀態雜湊。這
    /// `lockstep::TickBroadcaster` (120Hz) `try_recv` 獨立於此
    /// 狀態哈希間隔。在未啟用鎖定步驟的情況下運作時為“無”
    /// （mqtt/grpc 構建，或 kcp 構建，其中 main.rs 尚未連接它）。
    #[cfg(feature = "kcp")]
    state_hash_tx:
        Option<crossbeam_channel::Sender<crate::lockstep::tick_broadcaster::StateHashSample>>,
    /// 階段 5.3：用於觀察者重新加入的共享快照儲存。調度員寫道
    /// 每個“SNAPSHOT_INTERVAL_TICKS”滴答聲； KCP 傳輸的 0x16 SnapshotResp
    /// 處理程序讀取。當 main.rs 未連接 Arc 時為「無」（舊版/
    /// 非鎖步建置 - KCP 傳回落到空位元組）。
    #[cfg(feature = "kcp")]
    snapshot_store: Option<std::sync::Arc<std::sync::Mutex<crate::comp::SnapshotStore>>>,
    #[cfg(feature = "kcp")]
    team_bootstrap_store: Option<
        std::sync::Arc<
            std::sync::Mutex<
                std::collections::BTreeMap<u32, omoba_core::game_proto::TeamGameStart>,
            >,
        >,
    >,
    #[cfg(feature = "kcp")]
    observer_validation: Option<omoba_core::runtime::ObserverValidationWorker>,
    #[cfg(feature = "kcp")]
    authority_mismatch_rx:
        Option<crossbeam_channel::Receiver<omoba_core::runtime::ClientHashMismatch>>,
    #[cfg(feature = "kcp")]
    client_checkpoint_rx:
        Option<crossbeam_channel::Receiver<omoba_core::runtime::ClientCheckpointReport>>,
    #[cfg(feature = "kcp")]
    rebase_failure_rx:
        Option<crossbeam_channel::Receiver<omoba_core::runtime::RebaseFailureSignal>>,
    #[cfg(feature = "kcp")]
    secure_input_validation: Option<omoba_core::runtime::SharedSecureInputValidationSnapshot>,
    #[cfg(feature = "kcp")]
    selective_security_metrics:
        Option<std::sync::Arc<omoba_core::runtime::SelectiveSecurityMetrics>>,
    /// 階段 5.x 橋接器：與 `TickBroadcaster::host_input_tx` 配對的接收器。
    /// 每個廣播公司都會從「InputBuffer」消耗輸入一段時間
    /// `TickBatch` 也會沿著這個通道發送一個副本； `State::tick` 排水溝
    /// 並將輸入寫入“PendingPlayerInputs”，以便主機的
    /// `player_input_tick::Sys` 也能看到它們。主機與 broadcaster 現在同為
    /// 120Hz，但仍排空所有可用批次以便短暫 stall 後追上。
    #[cfg(feature = "kcp")]
    host_input_rx:
        Option<crossbeam_channel::Receiver<Vec<(u32, crate::lockstep::PlayerInput, u32)>>>,
}

// Superseded by omoba-core::runtime::production_guards, which validates the
// shared phase table rather than parsing duplicated source call sequences.
#[cfg(any())]
mod tower_ability_phase_order_tests {
    fn tick_body<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
        let start = source.rfind(start).expect("tick body start");
        let suffix = &source[start..];
        let end = suffix.find(end).expect("tick body end");
        &suffix[..end]
    }

    fn gameplay_phase_tokens(source: &str) -> Vec<&'static str> {
        let markers = [
            ("drain_pending_hero_command_clears", "hero_command_clears"),
            ("drain_pending_tower_spawns", "tower_spawns"),
            ("drain_pending_tower_sells", "tower_sells"),
            (
                "drain_pending_tower_target_priorities",
                "tower_target_priorities",
            ),
            ("drain_pending_item_uses", "item_uses"),
            ("drain_pending_ability_upgrades", "ability_upgrades"),
            ("drain_pending_ability_casts", "ability_casts"),
            ("drain_pending_moves", "moves"),
            ("process_outcomes", "outcomes"),
            ("drain_pending_tower_upgrades", "tower_upgrades"),
            ("drain_pending_tower_ability_casts", "tower_ability_casts"),
            ("tick_tower_abilities", "tower_ability_scheduler"),
            (
                "drain_pending_tower_ability_callbacks",
                "tower_ability_callbacks",
            ),
            ("run_script_dispatch", "script_dispatch"),
        ];
        source
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .flat_map(|line| {
                markers.iter().filter_map(move |(needle, token)| {
                    let matched = if *needle == "process_outcomes" {
                        line.contains(".process_outcomes(&mut self.ecs)")
                            || line.trim_start().starts_with("process_outcomes(world,")
                    } else {
                        line.contains(needle)
                    };
                    matched.then_some(*token)
                })
            })
            .collect()
    }

    fn authoritative_phase_source() -> &'static str {
        let source = include_str!("core.rs");
        let upgrades = source
            .find(concat!(
                "omoba_core::runtime::drain_pending_tower_",
                "upgrades(&mut self.ecs)"
            ))
            .expect("tower upgrade drain");
        let suffix = &source[upgrades..];
        let ordinary_ticks = suffix
            .find(concat!("scripting::run_script_", "dispatch("))
            .expect("ordinary unit script ticks");
        &suffix[..ordinary_ticks]
    }

    fn validate_contiguous_phase(phase: &str) -> Result<(), &'static str> {
        let casts = phase
            .find(concat!(
                "omoba_core::runtime::drain_pending_tower_ability_",
                "casts(&mut self.ecs)"
            ))
            .ok_or("tower ability cast drain missing")?;
        let scaled_dt = phase
            .find(concat!(
                "let scaled_dt = self.ecs.read_resource::<crate::comp::",
                "DeltaTime>().0;"
            ))
            .ok_or("scaled DeltaTime binding missing from phase")?;
        let scheduler = phase
            .find(concat!(
                "omoba_core::runtime::tick_tower_",
                "abilities(&mut self.ecs, scaled_dt)"
            ))
            .ok_or("tower ability scheduler missing")?;
        let callbacks = phase
            .find(concat!(
                "omoba_core::runtime::drain_pending_tower_ability_",
                "callbacks("
            ))
            .ok_or("tower ability callback drain missing")?;

        if !(casts < scaled_dt && scaled_dt < scheduler && scheduler < callbacks) {
            return Err("tower ability phase order changed");
        }
        if phase.matches("drain_pending_").count() != 3 {
            return Err("unexpected pending drain in phase");
        }
        if phase
            .matches(concat!("drain_pending_tower_ability_", "callbacks("))
            .count()
            != 1
        {
            return Err("callback drain must occur exactly once");
        }
        if phase.matches("tick_tower_abilities(").count() != 1 {
            return Err("scheduler must occur exactly once");
        }
        if phase.contains(concat!(".", "maintain()")) {
            return Err("maintenance boundary inside phase");
        }
        if phase.contains(concat!("process_", "outcomes(")) {
            return Err("outcome boundary inside phase");
        }
        if phase.contains(concat!("dispatcher.", "dispatch(")) {
            return Err("dispatcher boundary inside phase");
        }
        Ok(())
    }

    #[test]
    fn authoritative_runner_keeps_tower_ability_phase_order_and_scaled_delta() {
        assert_eq!(
            validate_contiguous_phase(authoritative_phase_source()),
            Ok(())
        );
    }

    #[test]
    fn authoritative_backend_and_local_replica_share_gameplay_phase_contract() {
        let backend = tick_body(
            include_str!("core.rs"),
            "pub fn tick(",
            "fn flush_runtime_events",
        );
        let replica_source = include_str!(concat!(
            "../../../omoba-core/src/runtime/native/",
            "simulation_driver.rs"
        ));
        let replica = tick_body(replica_source, "pub fn step(", "Ok(SimulationTickResult");

        let expected = vec![
            "hero_command_clears",
            "tower_spawns",
            "tower_sells",
            "tower_target_priorities",
            "item_uses",
            "ability_upgrades",
            "ability_casts",
            "moves",
            "outcomes",
            "tower_upgrades",
            "tower_ability_casts",
            "tower_ability_scheduler",
            "tower_ability_callbacks",
            "script_dispatch",
            "outcomes",
        ];
        assert_eq!(gameplay_phase_tokens(backend), expected);
        assert_eq!(gameplay_phase_tokens(replica), expected);
    }

    #[test]
    fn phase_validator_rejects_receiver_independent_maintenance_mutation() {
        let phase = authoritative_phase_source();
        let mutated = phase.replace(
            concat!(
                "omoba_core::runtime::drain_pending_tower_ability_",
                "casts(&mut self.ecs);"
            ),
            concat!(
                "omoba_core::runtime::drain_pending_tower_ability_",
                "casts(&mut self.ecs);\n        self.ecs.maintain();"
            ),
        );

        assert_ne!(mutated, phase, "mutation fixture must alter the phase");
        assert_eq!(
            validate_contiguous_phase(&mutated),
            Err("maintenance boundary inside phase")
        );
    }
}

/// 每個玩家至少強制發送一個（可能是空的）心跳，這樣
/// 客戶端仍然會收到“tick”/“game_time”心跳以進行時鐘同步和
/// 即使玩家的 AOI 中的 HP 值沒有變化，也能保持活躍度。空的
/// 在 prost+LZ4 之後，心跳壓縮到約 50 位元組 — 便宜的 keepalive。
#[cfg(any(feature = "grpc", feature = "kcp"))]
const HEARTBEAT_FORCE_SEND_INTERVAL: f64 = 5.0;

/// 階段 3.4：每 N 個調度程式週期發出一個狀態雜湊樣本。調度員
/// 以 lockstep cadence 運行，因此此值代表約 10 秒。
/// 廣播公司的間隔觸發（最多有一個陳舊時間）。
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
            // omoba-core installs its own transport::OutboundMsg resource; backend
            // uses omobab::transport::OutboundMsg, which is a distinct ECS type.
            ecs.insert(Vec::<Sender<OutboundMsg>>::new());
            let mut mqtx_vec = ecs.write_resource::<Vec<Sender<OutboundMsg>>>();
            mqtx_vec.push(mqtx.clone());
        }

        let lockstep_timing = CONFIG.lockstep_timing();
        let mut state = Self {
            ecs,
            cw: creep_wave_data,
            campaign: None,
            mqtx: mqtx.clone(),
            #[cfg(feature = "kcp")]
            reliable_team_tx: None,
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
            #[cfg(any(feature = "grpc", feature = "kcp"))]
            hb_last_hp_sent: HashMap::new(),
            #[cfg(any(feature = "grpc", feature = "kcp"))]
            hb_last_full_send: HashMap::new(),
            local_tick: 0,
            lockstep_timing,
            hero_knowledge_profile_modified: None,
            script_registry: ScriptRegistry::new(),
            #[cfg(feature = "runtime-lua-content")]
            dev_lua_hot_reload: None,
            #[cfg(feature = "kcp")]
            aoi_grid: None,
            #[cfg(feature = "kcp")]
            state_hash_tx: None,
            #[cfg(feature = "kcp")]
            snapshot_store: None,
            #[cfg(feature = "kcp")]
            team_bootstrap_store: None,
            #[cfg(feature = "kcp")]
            observer_validation: None,
            #[cfg(feature = "kcp")]
            authority_mismatch_rx: None,
            client_checkpoint_rx: None,
            #[cfg(feature = "kcp")]
            rebase_failure_rx: None,
            secure_input_validation: None,
            selective_security_metrics: None,
            #[cfg(feature = "kcp")]
            host_input_rx: None,
        };

        state.load_item_registry();
        state.load_scripts();
        state.initialize_standard_game();
        #[cfg(feature = "runtime-lua-content")]
        state.initialize_dev_lua_hot_reload();

        // 階段 5.2：遺留 0x02 心跳廣播切斷。鎖步刻度批次處理
        // (0x10) 透過每週期 state_hash 處理客戶端活躍度。

        state
    }

    /// 載入所有 native 腳本 DLL。目錄由環境變數 `OMB_SCRIPTS_DIR` 指定，
    /// 未設定時預設 `./scripts`（相對於執行目錄）。載入完就順便把塔 template
    /// 從腳本 `tower_metadata()` 收集到 `TowerTemplateRegistry` resource。
    ///
    /// 使用 `omoba-core::runtime` 的 shared bootstrap helpers，避免 backend
    /// 與 local replica 維護兩份 registry 初始化邏輯。
    fn load_scripts(&mut self) {
        let dir_str = std::env::var("OMB_SCRIPTS_DIR").unwrap_or_else(|_| "./scripts".to_string());
        let dir = std::path::Path::new(&dir_str);
        self.script_registry = crate::scripting::loader::load_scripts_dir(dir);
        omoba_core::runtime::populate_tower_template_registry(&mut self.ecs, &self.script_registry);
        omoba_core::runtime::populate_tower_upgrade_registry(&mut self.ecs);
        omoba_core::runtime::populate_ability_registry(&mut self.ecs, &self.script_registry);
    }

    fn load_item_registry(&mut self) {
        let item_reg = crate::item::load_registry_from_path("item-configs/items.json")
            .unwrap_or_else(|e| {
                log::warn!("裝備 Registry 載入失敗（{}），使用空 registry", e);
                crate::item::ItemRegistry::default()
            });
        self.ecs.insert(item_reg);
    }

    #[cfg(feature = "runtime-lua-content")]
    fn initialize_dev_lua_hot_reload(&mut self) {
        let manager = super::dev_lua_hot_reload::DevLuaHotReload::from_env();
        let status = manager
            .as_ref()
            .map(super::dev_lua_hot_reload::DevLuaHotReload::status)
            .unwrap_or_default();
        self.ecs.insert(status);
        self.dev_lua_hot_reload = manager;
    }

    #[cfg(feature = "runtime-lua-content")]
    fn poll_dev_lua_hot_reload(&mut self) {
        let event = {
            let Some(manager) = self.dev_lua_hot_reload.as_mut() else {
                return;
            };
            let event = manager.poll(self.local_tick);
            self.ecs.insert(manager.status());
            event
        };

        match event {
            Some(super::dev_lua_hot_reload::DevLuaHotReloadEvent::Candidate(info)) => {
                let result = self
                    .script_registry
                    .reload_runtime_lua_content_dev(&info.hash)
                    .and_then(|modules| {
                        omoba_template_ids::reload_runtime_lua_content_dev(Some(&info.hash))
                            .and_then(|committed| {
                                committed.ok_or_else(|| {
                                    "runtime Lua content became inactive during reload".to_string()
                                })
                            })
                            .map(|committed| (modules, committed))
                    });
                match result {
                    Ok((modules, committed)) => {
                        self.refresh_dev_lua_gameplay_content();
                        let pending = self
                            .dev_lua_hot_reload
                            .as_mut()
                            .expect("dev Lua hot reload manager")
                            .complete_reload(committed, self.local_tick);
                        log::info!(
                            "[dev-lua-hot-reload] reloaded {} script modules; scheduled generation={} hash={} apply_tick={}",
                            modules.len(),
                            pending.generation,
                            pending.hash,
                            pending.apply_tick
                        );
                    }
                    Err(err) => {
                        if let Some(manager) = self.dev_lua_hot_reload.as_mut() {
                            manager.fail_reload(err.clone());
                        }
                        log::warn!("[dev-lua-hot-reload] reload rejected: {}", err);
                    }
                }
            }
            Some(super::dev_lua_hot_reload::DevLuaHotReloadEvent::Scheduled(pending)) => {
                log::info!(
                    "[dev-lua-hot-reload] scheduled generation={} hash={} apply_tick={}",
                    pending.generation,
                    pending.hash,
                    pending.apply_tick
                );
            }
            Some(super::dev_lua_hot_reload::DevLuaHotReloadEvent::Failed(err)) => {
                log::warn!("[dev-lua-hot-reload] reload failed: {}", err);
            }
            None => {}
        }
        if let Some(manager) = self.dev_lua_hot_reload.as_ref() {
            self.ecs.insert(manager.status());
        }
    }

    #[cfg(feature = "runtime-lua-content")]
    fn refresh_dev_lua_gameplay_content(&mut self) {
        if let Some(campaign) = self.campaign.as_ref() {
            StateInitializer::refresh_creep_emiters(&mut self.ecs, &campaign.map);
        }
        omoba_core::runtime::populate_tower_template_registry(&mut self.ecs, &self.script_registry);
        omoba_core::runtime::populate_tower_upgrade_registry(&mut self.ecs);
        omoba_core::runtime::populate_ability_registry(&mut self.ecs, &self.script_registry);
        self.refresh_live_heroes_from_lua();
        self.refresh_live_creeps_from_lua();
        self.refresh_live_towers_from_lua();
        log::info!("[dev-lua-hot-reload] gameplay registries and live base stats refreshed");
    }

    #[cfg(feature = "runtime-lua-content")]
    fn refresh_live_heroes_from_lua(&mut self) {
        use crate::comp::{AttackSequencePhase, AttributeType, Hero, LevelGrowth, Vf32};
        use omoba_sim::Fixed64;
        let mut heroes = self.ecs.write_storage::<Hero>();
        let mut props = self.ecs.write_storage::<CProperty>();
        let mut attacks = self.ecs.write_storage::<TAttack>();
        let mut turns = self.ecs.write_storage::<TurnSpeed>();
        for (hero, prop, attack, turn) in (&mut heroes, &mut props, &mut attacks, &mut turns).join()
        {
            let Some(hero_id) = omoba_template_ids::hero_by_name(&hero.id) else {
                continue;
            };
            let Some(stats) = omoba_template_ids::active_hero_stats(hero_id) else {
                continue;
            };
            hero.name = omoba_template_ids::active_hero_display(hero_id).to_string();
            hero.title = omoba_template_ids::active_hero_title(hero_id).to_string();
            hero.strength = stats.strength;
            hero.agility = stats.agility;
            hero.intelligence = stats.intelligence;
            hero.primary_attribute = match stats.primary_attribute {
                1 => AttributeType::Agility,
                2 => AttributeType::Intelligence,
                _ => AttributeType::Strength,
            };
            hero.level_growth = LevelGrowth {
                strength_per_level: stats.level_growth.strength_per_level,
                agility_per_level: stats.level_growth.agility_per_level,
                intelligence_per_level: stats.level_growth.intelligence_per_level,
                damage_per_level: stats.level_growth.damage_per_level,
                hp_per_level: stats.level_growth.hp_per_level,
                mana_per_level: stats.level_growth.mana_per_level,
            };
            let new_abilities: Vec<String> = omoba_template_ids::active_hero_abilities(hero_id)
                .iter()
                .map(|id| id.as_str().to_string())
                .collect();
            for id in &new_abilities {
                hero.ability_levels.entry(id.clone()).or_insert(0);
            }
            hero.ability_levels
                .retain(|id, _| new_abilities.iter().any(|new_id| new_id == id));
            hero.abilities = new_abilities;

            let new_mhp = Fixed64::from_i32(500)
                + Fixed64::from_i32(hero.level) * hero.level_growth.hp_per_level;
            preserve_cproperty_hp_ratio(prop, new_mhp);
            prop.msd = stats.move_speed;
            prop.def_physic = Fixed64::from_i32(hero.strength) * Fixed64::from_raw(205);
            prop.def_magic = Fixed64::from_i32(hero.intelligence) * Fixed64::from_raw(154);
            attack.atk_physic = Vf32::new(
                Fixed64::from_i32(50)
                    + Fixed64::from_i32(hero.level) * hero.level_growth.damage_per_level,
            );
            attack.range = Vf32::new(stats.attack_range);
            attack.attack_phase = AttackSequencePhase::Idle;
            turn.0 = Fixed64::from_raw(
                (stats.turn_speed.to_f32_for_render().to_radians() * 1024.0) as i64,
            );
        }
    }

    #[cfg(feature = "runtime-lua-content")]
    fn refresh_live_creeps_from_lua(&mut self) {
        let emitters = self
            .ecs
            .read_resource::<BTreeMap<String, CreepEmiter>>()
            .clone();
        let mut creeps = self.ecs.write_storage::<Creep>();
        let mut props = self.ecs.write_storage::<CProperty>();
        let mut bounties = self.ecs.write_storage::<Bounty>();
        let mut turns = self.ecs.write_storage::<TurnSpeed>();
        for (creep, prop, bounty, turn) in
            (&mut creeps, &mut props, &mut bounties, &mut turns).join()
        {
            let Some(creep_id) = omoba_template_ids::creep_by_name(&creep.name) else {
                continue;
            };
            let Some(stats) = omoba_template_ids::active_creep_stats(creep_id) else {
                continue;
            };
            let display = omoba_template_ids::active_creep_display(creep_id);
            creep.label = (!display.is_empty()).then(|| display.to_string());
            preserve_cproperty_hp_ratio(prop, stats.hp);
            prop.msd = stats.move_speed;
            prop.def_physic = stats.armor;
            prop.def_magic = stats.magic_resistance;
            bounty.gold = stats.gold_reward;
            bounty.exp = stats.exp_reward;
            if let Some(emitter) = emitters.get(&creep.name) {
                turn.0 = omoba_sim::Fixed64::from_raw(
                    (emitter.turn_speed_deg.to_radians() * 1024.0) as i64,
                );
            }
        }
    }

    #[cfg(feature = "runtime-lua-content")]
    fn refresh_live_towers_from_lua(&mut self) {
        let registry = self.ecs.read_resource::<TowerTemplateRegistry>().clone();
        let tags = self.ecs.read_storage::<crate::scripting::ScriptUnitTag>();
        let mut towers = self.ecs.write_storage::<Tower>();
        let mut tprops = self.ecs.write_storage::<TProperty>();
        let mut cprops = self.ecs.write_storage::<CProperty>();
        let mut attacks = self.ecs.write_storage::<TAttack>();
        let mut visions = self.ecs.write_storage::<CircularVision>();
        let mut turns = self.ecs.write_storage::<TurnSpeed>();
        let mut radii = self.ecs.write_storage::<CollisionRadius>();
        let f32_to_fx = |v: f32| omoba_sim::Fixed64::from_raw((v * 1024.0) as i64);
        for (tag, _tower, tprop, cprop, attack, vision, turn, radius) in (
            &tags,
            &mut towers,
            &mut tprops,
            &mut cprops,
            &mut attacks,
            &mut visions,
            &mut turns,
            &mut radii,
        )
            .join()
        {
            let Some(tpl) = registry.get(&tag.unit_id) else {
                continue;
            };
            let new_hp = f32_to_fx(tpl.hp);
            let current_hp = scaled_hp(tprop.hp.v, tprop.hp.bv, new_hp);
            tprop.hp = Vf32 {
                bv: new_hp,
                v: current_hp,
            };
            preserve_cproperty_hp_ratio(cprop, new_hp);
            attack.atk_physic = Vf32::new(f32_to_fx(tpl.atk));
            attack.asd = Vf32::new(f32_to_fx(tpl.asd_interval));
            attack.range = Vf32::new(f32_to_fx(tpl.range));
            attack.bullet_speed = f32_to_fx(tpl.bullet_speed);
            vision.range = tpl.range + 100.0;
            turn.0 = f32_to_fx(tpl.turn_speed_deg.to_radians());
            radius.0 = f32_to_fx(tpl.footprint);
        }
        self.ecs.write_resource::<Searcher>().tower.mark_dirty();
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
            // omoba-core installs its own transport::OutboundMsg resource; backend
            // uses omobab::transport::OutboundMsg, which is a distinct ECS type.
            ecs.insert(Vec::<Sender<OutboundMsg>>::new());
            let mut mqtx_vec = ecs.write_resource::<Vec<Sender<OutboundMsg>>>();
            mqtx_vec.push(mqtx.clone());
        }

        let mut state = Self {
            ecs,
            cw: campaign_data.map.clone(),
            campaign: Some(campaign_data.clone()),
            mqtx: mqtx.clone(),
            #[cfg(feature = "kcp")]
            reliable_team_tx: None,
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
            #[cfg(any(feature = "grpc", feature = "kcp"))]
            hb_last_hp_sent: HashMap::new(),
            #[cfg(any(feature = "grpc", feature = "kcp"))]
            hb_last_full_send: HashMap::new(),
            local_tick: 0,
            lockstep_timing: CONFIG.lockstep_timing(),
            hero_knowledge_profile_modified: None,
            script_registry: ScriptRegistry::new(),
            #[cfg(feature = "runtime-lua-content")]
            dev_lua_hot_reload: None,
            #[cfg(feature = "kcp")]
            aoi_grid: None,
            #[cfg(feature = "kcp")]
            state_hash_tx: None,
            #[cfg(feature = "kcp")]
            snapshot_store: None,
            #[cfg(feature = "kcp")]
            team_bootstrap_store: None,
            #[cfg(feature = "kcp")]
            observer_validation: None,
            #[cfg(feature = "kcp")]
            authority_mismatch_rx: None,
            client_checkpoint_rx: None,
            #[cfg(feature = "kcp")]
            rebase_failure_rx: None,
            secure_input_validation: None,
            selective_security_metrics: None,
            #[cfg(feature = "kcp")]
            host_input_rx: None,
        };

        state.load_item_registry();
        // 先載 scripts，才能讓 initialize_campaign_game 內的 send_tower_templates 拿到 registry
        state.load_scripts();
        state.initialize_campaign_game(&campaign_data);
        #[cfg(feature = "runtime-lua-content")]
        state.initialize_dev_lua_hot_reload();

        // 階段 5.2：遺留 0x02 GameEvent 廣播剪輯。

        state
    }

    /// 遊戲主循環 tick
    pub fn tick(&mut self, dt: Duration) -> Result<(), Error> {
        self.local_tick = self.local_tick.wrapping_add(1);
        let dt_fixed_raw = self.lockstep_timing.fixed_raw_for_tick(self.local_tick);

        // 更新時間管理。暫停中仍會繼續收 lockstep input，但 gameplay time 不前進。
        let was_paused = self.ecs.read_resource::<crate::comp::GamePause>().is_paused;
        if was_paused {
            self.time_manager.pause_time(&mut self.ecs);
        } else {
            self.time_manager
                .update(&mut self.ecs, dt, Some(dt_fixed_raw))?;
        }

        #[cfg(feature = "runtime-lua-content")]
        self.poll_dev_lua_hot_reload();

        // 吸收 transport 傳進來的 viewport 更新
        #[cfg(any(feature = "grpc", feature = "kcp"))]
        self.drain_viewport_updates();

        // 階段 5.x 橋接器：將所有待處理的廣播機構排出的輸入拉入
        // PendingPlayerInputs 以便player_input_tick::Sys 可以路由 StartRound
        // （以及未來的命令）。排出此刻度中的所有可用批次
        // 如果主機短暫落後於 broadcaster，則可以趕上。
        #[cfg(feature = "kcp")]
        if let Some(rx) = self.host_input_rx.as_ref() {
            let mut accumulated: Vec<(u32, crate::lockstep::PlayerInput, u32)> = Vec::new();
            while let Ok(batch) = rx.try_recv() {
                accumulated.extend(batch);
            }
            if !accumulated.is_empty() {
                let accepted_projection = self.build_canonical_accepted_inputs(&accumulated);
                use crate::comp::PendingPlayerInputs;
                {
                    let mut pending = self.ecs.write_resource::<PendingPlayerInputs>();
                    pending.tick = self.local_tick as u32;
                    pending.inputs.clear();
                    for (player_id, input, _) in accumulated {
                        pending.inputs.push((player_id, input));
                    }
                }
                self.ecs
                    .write_resource::<omoba_core::runtime::TeamProjectionRuntime>()
                    .pending_accepted_inputs
                    .extend(accepted_projection);
            }
        }

        // All production runtimes consume the same shared ordering table.
        let mut run_systems_ns = 0u128;
        let mut process_outcomes_ns = 0u128;
        let mut script_dispatch_ns = 0u128;
        omoba_core::runtime::run_deterministic_gameplay_phases(
            &mut |phase| -> Result<(), Error> {
                use omoba_core::runtime::DeterministicGameplayPhase as P;
                match phase {
                    P::Dispatcher => {
                        let t = Instant::now();
                        self.system_dispatcher.run_systems(&self.ecs)?;
                        run_systems_ns += t.elapsed().as_nanos();
                    }
                    P::RuntimeEventBoundary => self.flush_runtime_events(),
                    P::HeroCommandClears => {
                        omoba_core::runtime::drain_pending_hero_command_clears(&mut self.ecs)
                    }
                    P::TowerSpawns => {
                        omoba_core::runtime::drain_pending_tower_spawns(&mut self.ecs)
                    }
                    P::TowerSells => omoba_core::runtime::drain_pending_tower_sells(&mut self.ecs),
                    P::TowerTargetPriorities => {
                        omoba_core::runtime::drain_pending_tower_target_priorities(&mut self.ecs)
                    }
                    P::ItemUses => omoba_core::runtime::drain_pending_item_uses(&mut self.ecs),
                    P::AbilityUpgrades => {
                        omoba_core::runtime::drain_pending_ability_upgrades(&mut self.ecs)
                    }
                    P::AbilityCasts => {
                        omoba_core::runtime::drain_pending_ability_casts(&mut self.ecs)
                    }
                    P::Moves => omoba_core::runtime::drain_pending_moves(&mut self.ecs),
                    P::PreScriptOutcomes | P::PostScriptOutcomes => {
                        let t = Instant::now();
                        self.resource_manager.process_outcomes(&mut self.ecs)?;
                        process_outcomes_ns += t.elapsed().as_nanos();
                    }
                    P::TowerUpgrades => {
                        omoba_core::runtime::drain_pending_tower_upgrades(&mut self.ecs)
                    }
                    P::TowerAbilityCasts => {
                        omoba_core::runtime::drain_pending_tower_ability_casts(&mut self.ecs)
                    }
                    P::TowerAbilityScheduler => {
                        let dt = self.ecs.read_resource::<crate::comp::DeltaTime>().0;
                        omoba_core::runtime::tick_tower_abilities(&mut self.ecs, dt);
                    }
                    P::TowerAbilityCallbacks => {
                        let global_seed = self.ecs.read_resource::<crate::comp::MasterSeed>().0;
                        omoba_core::runtime::drain_pending_tower_ability_callbacks(
                            &mut self.ecs,
                            &self.script_registry,
                            global_seed,
                        );
                    }
                    P::ScriptDispatch => {
                        let t = Instant::now();
                        let dt = self.ecs.read_resource::<crate::comp::DeltaTime>().0;
                        let global_seed = self.ecs.read_resource::<crate::comp::MasterSeed>().0;
                        scripting::run_script_dispatch(
                            &mut self.ecs,
                            &self.script_registry,
                            global_seed,
                            dt,
                        );
                        script_dispatch_ns += t.elapsed().as_nanos();
                    }
                    P::CreepWave => self.resource_manager.process_creep_waves(&mut self.ecs)?,
                }
                if matches!(phase, P::PreScriptOutcomes | P::PostScriptOutcomes) {
                    self.ecs.maintain();
                }
                Ok(())
            },
        )?;

        // Wave A：outcome 與 fact 已在同一 Specs tick 中完成並穩定 reduce。
        // 只有 barrier 完成後，Wave B 才能讀取 committed State[T+1]；各 team
        // visibility job 在 rayon 中彼此平行，且不再修改 gameplay state。
        let ordered_facts = self
            .ecs
            .read_resource::<omoba_core::runtime::ObservableFactBuffer>()
            .drain_ordered()
            .map_err(|error| {
                failure::err_msg(format!("observable fact reduce failed: {error:?}"))
            })?;
        let ordered_outcomes = {
            let mut buffer = self
                .ecs
                .write_resource::<omoba_core::runtime::OrderedRuntimeEventBuffer>();
            std::mem::take(&mut buffer.events)
        };
        let committed =
            omoba_core::runtime::commit_wave_a(self.local_tick, ordered_outcomes, ordered_facts)
                .map_err(|error| failure::err_msg(format!("Wave A commit failed: {error:?}")))?;
        {
            let mut batch = self
                .ecs
                .write_resource::<omoba_core::runtime::CommittedProjectionBatch>();
            batch.tick = committed.tick;
            batch.ordered_outcome_count = committed.ordered_outcomes.len();
            batch.facts = committed.ordered_facts;
            batch.barrier_reached = committed.barrier_reached;
        }
        self.system_dispatcher
            .run_post_commit_visibility(&mut self.ecs, self.local_tick, 1);
        #[cfg(feature = "kcp")]
        if let Some(worker) = &self.observer_validation {
            let mut coordinator = self
                .ecs
                .write_resource::<omoba_core::runtime::AuthorityRepairCoordinator>();
            while let Some(mismatch) = worker.try_recv_mismatch() {
                coordinator.report_observer_mismatch(mismatch);
            }
            while let Some(report) = worker.try_recv_checkpoint() {
                coordinator.report_observer_checkpoint(report);
            }
            for gap in worker.tap().take_coverage_gaps() {
                coordinator.report_coverage_gap(gap.team_id, gap.first_unverified_sequence);
            }
            for team_id in worker.unverified_worker_teams() {
                coordinator.report_coverage_gap(team_id, 0);
            }
        }
        #[cfg(feature = "kcp")]
        if let Some(rx) = &self.authority_mismatch_rx {
            let mut coordinator = self
                .ecs
                .write_resource::<omoba_core::runtime::AuthorityRepairCoordinator>();
            while let Ok(mismatch) = rx.try_recv() {
                coordinator.report_client_mismatch(mismatch);
            }
        }
        #[cfg(feature = "kcp")]
        if let Some(rx) = &self.client_checkpoint_rx {
            let mut coordinator = self
                .ecs
                .write_resource::<omoba_core::runtime::AuthorityRepairCoordinator>();
            while let Ok(report) = rx.try_recv() {
                coordinator.report_client_checkpoint(report);
            }
        }
        {
            let coordinator = self
                .ecs
                .read_resource::<omoba_core::runtime::AuthorityRepairCoordinator>();
            record_three_way_checkpoints(&coordinator);
        }
        #[cfg(feature = "kcp")]
        if let Some(rx) = &self.rebase_failure_rx {
            let mut coordinator = self
                .ecs
                .write_resource::<omoba_core::runtime::AuthorityRepairCoordinator>();
            while let Ok(failure) = rx.try_recv() {
                if failure.replay_coverage_gap {
                    coordinator.report_coverage_gap(
                        failure.team_id,
                        failure.last_safe_sequence.saturating_add(1),
                    );
                } else {
                    let _ = coordinator
                        .manifest_verification_failed(failure.team_id, failure.last_safe_sequence);
                }
            }
        }
        if crate::config::server_config::CONFIG.selective_generation_enabled() {
            omoba_core::runtime::run_team_projection_after_wave_b(&mut self.ecs, self.local_tick)
                .map_err(|error| failure::err_msg(format!("team projection failed: {error:?}")))?;
        } else {
            let mut projection = self
                .ecs
                .write_resource::<omoba_core::runtime::TeamProjectionRuntime>();
            projection.latest_frames.clear();
            projection.latest_rebases.clear();
        }
        record_canonical_timeline(self.local_tick, (&self.ecs.entities()).join().count());
        #[cfg(feature = "kcp")]
        if let Some(shared) = &self.secure_input_validation {
            let visibility = self
                .ecs
                .read_resource::<omoba_core::runtime::TeamVisibilityRuntime>();
            let projection = self
                .ecs
                .read_resource::<omoba_core::runtime::TeamProjectionRuntime>();
            *shared
                .lock()
                .expect("secure input validation snapshot mutex poisoned") =
                projection.build_input_validation_snapshot(&visibility);
        }
        #[cfg(feature = "kcp")]
        if let Some(metrics) = &self.selective_security_metrics {
            use std::sync::atomic::Ordering;
            let visibility = self
                .ecs
                .read_resource::<omoba_core::runtime::TeamVisibilityRuntime>();
            let projection = self
                .ecs
                .read_resource::<omoba_core::runtime::TeamProjectionRuntime>();
            metrics.visibility_transition_count.fetch_add(
                visibility
                    .last_transitions
                    .values()
                    .map(Vec::len)
                    .sum::<usize>() as u64,
                Ordering::Relaxed,
            );
            metrics.steady_state_padding_bytes.fetch_add(
                projection
                    .latest_frames
                    .values()
                    .map(|frame| frame.padding_len as u64)
                    .sum::<u64>(),
                Ordering::Relaxed,
            );
            metrics.encoded_frame_bytes.fetch_add(
                projection
                    .latest_frames
                    .values()
                    .map(|frame| frame.wire_bytes.len() as u64)
                    .sum::<u64>(),
                Ordering::Relaxed,
            );
            metrics.reveal_burst_bytes.fetch_add(
                projection
                    .latest_frames
                    .values()
                    .map(|frame| {
                        frame.frame.pre_step.as_ref().map_or(0, |pre| {
                            pre.transitions
                                .iter()
                                .filter(|transition| {
                                    matches!(
                                        transition.transition,
                                        Some(
                                            omoba_core::game_proto::transition::Transition::Reveal(
                                                _
                                            )
                                        )
                                    )
                                })
                                .map(prost::Message::encoded_len)
                                .sum::<usize>()
                        }) as u64
                    })
                    .sum::<u64>(),
                Ordering::Relaxed,
            );
            metrics
                .outbound_queue_depth
                .store(self.mqtx.len() as u64, Ordering::Relaxed);
            let coordinator = self
                .ecs
                .read_resource::<omoba_core::runtime::AuthorityRepairCoordinator>();
            metrics
                .authority_repair_count
                .store(coordinator.repair_count, Ordering::Relaxed);
            metrics
                .authority_rebase_count
                .store(coordinator.rebase_count, Ordering::Relaxed);
            if let Some(worker) = &self.observer_validation {
                let observer = &worker.tap().metrics;
                metrics.observer_audit_lag_ticks.store(
                    observer.audit_lag_ticks.load(Ordering::Relaxed),
                    Ordering::Relaxed,
                );
                metrics.coverage_gap_count.store(
                    observer.coverage_gap_count.load(Ordering::Relaxed),
                    Ordering::Relaxed,
                );
            }
        }
        if let Some(diagnostic) = self
            .ecs
            .read_resource::<omoba_core::runtime::TeamProjectionRuntime>()
            .safe_terminations
            .first()
            .cloned()
        {
            let opaque_team = omoba_core::runtime::opaque_match_team_id(
                &self
                    .ecs
                    .read_resource::<crate::comp::MasterSeed>()
                    .0
                    .to_be_bytes(),
                diagnostic.team_id,
            );
            return Err(failure::err_msg(format!(
                "secure match terminated opaque_team={} last_safe_sequence={} reason={} protocol_fallback_allowed={}",
                opaque_team,
                diagnostic.last_safe_sequence,
                diagnostic.reason_class,
                diagnostic.protocol_fallback_allowed,
            )));
        }
        #[cfg(feature = "kcp")]
        if let Some(store) = &self.team_bootstrap_store {
            let global_seed = self.ecs.read_resource::<crate::comp::MasterSeed>().0;
            let (bootstraps, observer_rebootstrap_teams) = {
                let mut projection = self
                    .ecs
                    .write_resource::<omoba_core::runtime::TeamProjectionRuntime>();
                let teams = projection
                    .latest_rebases
                    .keys()
                    .copied()
                    .collect::<Vec<_>>();
                let mut starts = projection.build_team_bootstraps(
                    u64::from(self.local_tick).saturating_add(1),
                    crate::config::server_config::CONFIG.STEP_FPS,
                    global_seed,
                );
                drop(projection);
                let blocked = self.ecs.read_resource::<crate::comp::BlockedRegions>();
                let encoded = omoba_core::runtime::encode_public_blocked_regions(&blocked);
                for start in starts.values_mut() {
                    start
                        .public_metadata
                        .push(omoba_core::game_proto::DeterministicMetadata {
                            namespace: omoba_core::runtime::PUBLIC_BLOCKED_REGIONS_NAMESPACE.into(),
                            key: omoba_core::runtime::PUBLIC_BLOCKED_REGIONS_KEY.into(),
                            schema_version: 1,
                            value: encoded.clone(),
                        });
                }
                (starts, teams)
            };
            let mut stored = store.lock().expect("team bootstrap store mutex poisoned");
            if let Some(worker) = &self.observer_validation {
                for (team_id, start) in &bootstraps {
                    // Initial bootstrap must be observed only after the KCP
                    // broadcaster has actually enqueued that exact
                    // TeamGameStart for a secure player session. Tapping it
                    // here as well races the session bootstrap and can reset
                    // the observer to a different start tick before frame 1.
                    // A projector-requested rebase is different: it replaces
                    // the already-running team view, so reset the observer to
                    // the equivalent freshly built filtered baseline before
                    // the queued rebase frames are consumed by the player.
                    if observer_rebootstrap_teams.contains(team_id) {
                        worker
                            .tap()
                            .try_bootstrap(Arc::from(prost::Message::encode_to_vec(start)));
                    }
                }
            }
            *stored = bootstraps;
        }
        #[cfg(feature = "kcp")]
        let mut rebase_frames = self
            .ecs
            .write_resource::<omoba_core::runtime::TeamProjectionRuntime>()
            .take_rate_limited_rebase_outbound();
        let team_frames: Vec<_> = self
            .ecs
            .read_resource::<omoba_core::runtime::TeamProjectionRuntime>()
            .latest_frames
            .iter()
            .map(|(team_id, padded)| {
                (
                    *team_id,
                    padded.frame.team_sequence,
                    padded.frame.replica_tick,
                    Arc::<[u8]>::from(padded.wire_bytes.clone()),
                )
            })
            .collect();
        for (team_id, sequence, replica_tick, encoded) in team_frames {
            #[cfg(feature = "kcp")]
            if let Some(index) = rebase_frames
                .iter()
                .position(|(team, _, _)| *team == team_id)
            {
                use prost::Message as _;
                let (_, chunks, manifest) = rebase_frames.remove(index);
                let tx = self.reliable_team_tx.as_ref().unwrap_or(&self.mqtx);
                for encoded in chunks {
                    if let Some(metrics) = &self.selective_security_metrics {
                        metrics
                            .rebase_burst_bytes
                            .fetch_add(encoded.len() as u64, std::sync::atomic::Ordering::Relaxed);
                    }
                    reliable_send_with_watchdog(
                        tx,
                        OutboundMsg::lockstep_frame(
                            crate::lockstep::LockstepFrame::TeamRebaseChunkV2 {
                                team_id,
                                encoded: Arc::from(encoded),
                            },
                        ),
                        Duration::from_secs(5),
                    )
                    .map_err(|_| failure::err_msg("rebase chunk outbound watchdog"))?;
                }
                if let Some(manifest) = manifest {
                    if let Some(metrics) = &self.selective_security_metrics {
                        metrics
                            .rebase_burst_bytes
                            .fetch_add(manifest.len() as u64, std::sync::atomic::Ordering::Relaxed);
                    }
                    reliable_send_with_watchdog(
                        tx,
                        OutboundMsg::lockstep_frame(
                            crate::lockstep::LockstepFrame::TeamRebaseManifestV2 {
                                team_id,
                                encoded: Arc::from(manifest),
                            },
                        ),
                        Duration::from_secs(5),
                    )
                    .map_err(|_| failure::err_msg("rebase manifest outbound watchdog"))?;
                }
            }
            record_team_frame_evidence(team_id, sequence, replica_tick, &encoded);
            let outbound =
                OutboundMsg::lockstep_frame(crate::lockstep::LockstepFrame::TeamTickFrameV2 {
                    team_id,
                    sequence,
                    replica_tick,
                    encoded,
                });
            #[cfg(feature = "kcp")]
            {
                use crossbeam_channel::SendTimeoutError;
                let tx = self.reliable_team_tx.as_ref().unwrap_or(&self.mqtx);
                let queue_was_full = tx.is_full();
                if queue_was_full {
                    log::warn!(
                        "secure team outbound queue blocked team={} sequence={}",
                        team_id,
                        sequence
                    );
                }
                match reliable_send_with_watchdog(tx, outbound, Duration::from_secs(5)) {
                    Ok(elapsed) => {
                        if queue_was_full {
                            log::info!(
                                "secure team outbound queue resumed team={} sequence={} blocked_us={}",
                                team_id,
                                sequence,
                                elapsed.as_micros()
                            );
                        }
                        if let Some(metrics) = &self.selective_security_metrics {
                            metrics.outbound_blocking_wait_ns.fetch_add(
                                elapsed.as_nanos().min(u128::from(u64::MAX)) as u64,
                                std::sync::atomic::Ordering::Relaxed,
                            );
                            if elapsed > self.lockstep_timing.dt_duration() {
                                metrics
                                    .outbound_deadline_miss_count
                                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            }
                        }
                    }
                    Err(SendTimeoutError::Timeout(_)) => {
                        if let Some(metrics) = &self.selective_security_metrics {
                            metrics
                                .outbound_watchdog_timeout_count
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        }
                        self.ecs
                            .write_resource::<omoba_core::runtime::TeamProjectionRuntime>()
                            .safe_terminations
                            .push(omoba_core::runtime::SafeTerminationDiagnostic {
                                team_id,
                                last_safe_sequence: sequence.saturating_sub(1),
                                reason_class: "outbound_queue_watchdog".into(),
                                safe_component_path: None,
                                protocol_fallback_allowed: false,
                            });
                        return Err(failure::err_msg(format!(
                            "secure team {team_id} outbound queue watchdog exceeded 5 seconds"
                        )));
                    }
                    Err(SendTimeoutError::Disconnected(_)) => {
                        return Err(failure::err_msg("secure team outbound queue disconnected"));
                    }
                }
            }
            #[cfg(not(feature = "kcp"))]
            self.mqtx
                .send(outbound)
                .map_err(|_| failure::err_msg("outbound queue disconnected"))?;
        }
        {
            use crate::comp::{TickPhase, TickProfile};
            let mut profile = self.ecs.write_resource::<TickProfile>();
            profile.record_phase(TickPhase::RunSystems, run_systems_ns);
            profile.record_phase(TickPhase::ScriptDispatch, script_dispatch_ns);
            profile.record_phase(TickPhase::ProcessOutcomes, process_outcomes_ns);
            profile.finish_tick_and_maybe_log();
        }

        // 處理玩家資料
        self.resource_manager
            .process_player_data(&mut self.ecs, &self.mqrx)?;

        self.poll_hero_knowledge_profile_reload();

        // 處理 MCP 查詢請求
        #[cfg(any(feature = "grpc", feature = "kcp"))]
        self.process_queries();

        // 階段 5.2：遺留 0x02 GameEvent 廣播剪輯。鎖步刻度批次處理
        // (0x10) 攜帶心跳/英雄統計數據/可見度差異等價物。

        // 維護 ECS
        self.ecs.maintain();

        // 階段 3.4：每隔一段時間發布一個確定性的 ECS 狀態哈希
        // STATE_HASH_INTERVAL_TICKS 調度程式滴答聲（120Hz cadence）。這
        // 120Hz 鎖步 TickBroadcaster 在其上提取最新樣本
        // 自己的狀態雜湊間隔（預設 10s @ 120Hz），因此是一個新鮮的樣本
        // 始終處於待處理狀態。當 state_hash_tx 為 None 時跳過（舊版/
        // 非鎖步建置）。
        #[cfg(feature = "kcp")]
        if self.local_tick % self.lockstep_timing.ticks_for_seconds_u64(10) == 0 {
            if let Some(tx) = &self.state_hash_tx {
                let hash = crate::lockstep::compute_state_hash(&self.ecs);
                // u32 包裝與原始 StateHash.tick 欄位相符。
                let tick_u32 = self.local_tick as u32;
                if let Err(e) = tx.send((tick_u32, hash)) {
                    log::warn!("State: failed to publish state hash: {e}");
                }
            }
        }

        // 階段 5.3：序列化新的世界快照以供觀察者重新加入
        // 每個 SNAPSHOT_INTERVAL_TICKS 排程器滴答（= 30 s @ 120Hz）。
        // 跳過刻度 0 — 第一個調度刻度可能會在所有刻度之前運行
        // populate_* 幫助程式已完成註冊表填充，所以請等待
        // 直到遊戲狀態至少一整刻已經穩定下來。
        // 寫入到 (1) SnapshotStore ECS 資源（始終 — 查詢
        // 路徑）和（2）可選的 `snapshot_store` Arc<Mutex<>> 時
        // 由 main.rs 連接（KCP 傳輸從中讀取）。
        #[cfg(feature = "kcp")]
        if self.local_tick > 0
            && self.local_tick % self.lockstep_timing.ticks_for_seconds_u64(30) == 0
        {
            let bytes = crate::lockstep::serialize_snapshot(&self.ecs);
            let tick_u32 = self.local_tick as u32;
            let byte_len = bytes.len();
            // 首先更新 ECS 資源（便宜 — 相同的調度程序執行緒）。
            {
                let mut store = self.ecs.write_resource::<crate::comp::SnapshotStore>();
                store.tick = tick_u32;
                store.bytes = bytes.clone();
            }
            // 當傳輸連線時，鏡像到共用 Arc<Mutex<>>。
            // `lock().unwrap()` 可以：傳輸端讀取器持有
            // 鎖定微秒（克隆+刪除）並且永遠不會出現恐慌
            // 正常運轉。這裡中毒的互斥體是無法恢復的。
            if let Some(shared) = &self.snapshot_store {
                let mut guard = shared.lock().expect("SnapshotStore mutex poisoned");
                guard.tick = tick_u32;
                guard.bytes = bytes;
            }
            log::info!("[snapshot] saved tick={} bytes={}", tick_u32, byte_len);
        }

        Ok(())
    }

    fn flush_runtime_events(&mut self) {
        let events = {
            let mut events = self
                .ecs
                .write_resource::<Vec<omoba_core::runtime::RuntimeEvent>>();
            std::mem::take(&mut *events)
        };
        for event in &events {
            // 偵測對局結束事件，發放 KP
            if event.topic == "td/all/res" && event.kind == "game" && event.action == "end" {
                log::info!("[hero_knowledge] 偵測到 game_end 事件，data={}", event.data);
                self.award_kp_on_game_end(&event.data);
            }
        }
        let ordered = crate::runtime_events::order_processed_runtime_events(
            self.local_tick,
            omoba_core::runtime::FactPhase::PostStep,
            events,
        );
        for msg in crate::runtime_events::ordered_runtime_events_to_outbound(ordered) {
            let _ = self.mqtx.try_send(msg);
        }
    }

    #[cfg(feature = "kcp")]
    fn build_canonical_accepted_inputs(
        &self,
        inputs: &[(u32, crate::lockstep::PlayerInput, u32)],
    ) -> Vec<omoba_core::runtime::CanonicalAcceptedInput> {
        use omoba_core::game_proto::player_input::Action;
        use prost::Message as _;
        let entities = self.ecs.entities();
        let heroes = self.ecs.read_storage::<crate::comp::Hero>();
        let owners = self.ecs.read_storage::<crate::comp::PlayerOwner>();
        inputs
            .iter()
            .filter_map(|(player_id, input, acceptance_correlation)| {
                let team_id = *crate::config::server_config::CONFIG
                    .AUTHENTICATED_TEAM_BINDINGS
                    .get(player_id)?;
                let actor = (&entities, &heroes, &owners)
                    .join()
                    .find(|(_, _, owner)| owner.player_id == *player_id)
                    .map(|(entity, _, _)| omoba_core::runtime::canonical_entity_id(entity))?;
                let mut sanitized = input.clone();
                let (action_kind, target_index) = match sanitized.action.as_mut()? {
                    Action::NoOp(_) => (1, None),
                    Action::MoveTo(_) => (2, None),
                    Action::AttackTarget(value) => {
                        let id = value.target_id;
                        value.target_id = 0;
                        (3, Some(id))
                    }
                    Action::CastAbility(value) => {
                        let id = value.target_entity.take();
                        (4, id)
                    }
                    Action::TowerPlace(_) => (5, None),
                    Action::TowerUpgrade(value) => {
                        let id = value.tower_entity_id;
                        value.tower_entity_id = 0;
                        (6, Some(id))
                    }
                    Action::TowerSell(value) => {
                        let id = value.tower_entity_id;
                        value.tower_entity_id = 0;
                        (7, Some(id))
                    }
                    Action::ItemUse(value) => {
                        let id = value.target_entity.take();
                        (8, id)
                    }
                    Action::StartRound(_) => (9, None),
                    Action::UpgradeAbility(_) => (10, None),
                    Action::AttackMove(_) => (11, None),
                    Action::SetTowerTargetPriority(value) => {
                        let id = value.tower_entity_id;
                        value.tower_entity_id = 0;
                        (12, Some(id))
                    }
                    Action::TogglePause(_) => (13, None),
                    Action::ToggleGameSpeed(_) => (14, None),
                    Action::DebugSpawnCreep(_) => (15, None),
                    Action::TowerAbilityCast(value) => {
                        let id = value.tower_entity_id;
                        value.tower_entity_id = 0;
                        (16, Some(id))
                    }
                };
                let target_canonical_id = target_index.and_then(|id| {
                    let entity = entities.entity(id);
                    entities
                        .is_alive(entity)
                        .then(|| omoba_core::runtime::canonical_entity_id(entity))
                });
                if target_index.is_some() && target_canonical_id.is_none() {
                    return None;
                }
                Some(
                    omoba_core::runtime::CanonicalAcceptedInput::from_authoritative_acceptance(
                        team_id,
                        *player_id,
                        u64::from(*acceptance_correlation),
                        action_kind,
                        actor,
                        target_canonical_id,
                        sanitized.encode_to_vec(),
                    ),
                )
            })
            .collect()
    }

    fn award_kp_on_game_end(&self, data: &serde_json::Value) {
        use crate::config::server_config::read_hero_knowledge_setting;
        use crate::knowledge::kp_reward::{award_kp_for_game_end, KpRewardConfig};

        let gk_cfg = read_hero_knowledge_setting();
        if !gk_cfg.enabled {
            return;
        }

        let is_victory = data
            .get("winner")
            .or_else(|| data.get("result"))
            .and_then(|v| v.as_str())
            .map(|s| s == "player" || s == "victory" || s == "win")
            .unwrap_or(false);

        let omb_dir = std::path::PathBuf::from(".");
        let config = KpRewardConfig {
            base_kp_reward: gk_cfg.base_kp_reward,
            win_kp_bonus: gk_cfg.win_kp_bonus,
        };
        award_kp_for_game_end(&omb_dir, config, is_victory);
    }

    /// 從傳輸層排出視窗更新。調用每個蜱蟲。
    #[cfg(any(feature = "grpc", feature = "kcp"))]
    fn drain_viewport_updates(&mut self) {
        while let Ok(msg) = self.viewport_rx.try_recv() {
            match msg {
                ViewportMsg::Set {
                    player_name,
                    viewport,
                } => {
                    log::info!(
                        "📥 [State] ViewportMsg::Set player='{}' padded=({}, {})",
                        player_name,
                        viewport.padded_hw,
                        viewport.padded_hh
                    );
                    self.client_viewports.insert(player_name, viewport);
                }
                ViewportMsg::Remove { player_name } => {
                    log::info!("📥 [State] ViewportMsg::Remove player='{}'", player_name);
                    self.client_viewports.remove(&player_name);
                    // 刪除玩家的心跳差異緩存，以便未來
                    // 重新連接從頭開始（完整快照
                    // 重新加入後的第一個刻度 - 每個“prev”都是“None”
                    // 實體 → 全部包括在內）。
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
                "inspect_player_view" => {
                    query::query_inspect_player_view(&self.ecs, &req.player_name)
                }
                "list_abilities" => query::query_list_abilities(&self.ecs),
                "get_ability_detail" => {
                    query::query_get_ability_detail(&self.ecs, &req.player_name)
                }
                other => crate::transport::QueryResponse {
                    success: false,
                    error: format!("Unknown query_type: {}", other),
                    data_json: Vec::new(),
                },
            };
            let _ = req.response_tx.send(response);
        }
    }

    /// P5：插入 KCP 傳輸中的共用「AoiGrid」。國家將
    /// 使用相同的（id，pos）預先收集重建網格每個心跳滴答
    /// 建立心跳快照。之後可以安全撥打一次
    /// 獲得“TransportHandle”。
    #[cfg(feature = "kcp")]
    pub fn attach_aoi_grid(&mut self, grid: std::sync::Arc<std::sync::Mutex<crate::aoi::AoiGrid>>) {
        self.aoi_grid = Some(grid);
    }

    /// 階段 3.4：註冊調度程式 → 廣播程式狀態雜湊通道。
    /// 建立 State 和 the 之後從 `main.rs` 調用
    /// `TickBroadcaster` 的接收器。如果從未調用過，則哈希發布是
    /// 無操作，廣播公司退回其占位符。
    #[cfg(feature = "kcp")]
    pub fn set_state_hash_tx(
        &mut self,
        tx: crossbeam_channel::Sender<crate::lockstep::tick_broadcaster::StateHashSample>,
    ) {
        self.state_hash_tx = Some(tx);
    }

    /// 階段 5.3：註冊共享快照儲存。調度員勾選
    /// 循環會將其週期性的“serialize_snapshot”輸出鏡像到此
    /// `Arc<Mutex<>>` 因此 KCP 傳輸的 0x16 SnapshotResp 處理程序
    /// （在 tokio 任務中運行 - 沒有直接的 World 訪問）可以服務真實的
    /// 位元組.如果從未調用，快照仍會更新 ECS 資源
    /// （可查詢）但傳輸看到空字節。
    #[cfg(feature = "kcp")]
    pub fn attach_snapshot_store(
        &mut self,
        store: std::sync::Arc<std::sync::Mutex<crate::comp::SnapshotStore>>,
    ) {
        self.snapshot_store = Some(store);
    }

    #[cfg(feature = "kcp")]
    pub fn attach_team_bootstrap_store(
        &mut self,
        store: std::sync::Arc<
            std::sync::Mutex<
                std::collections::BTreeMap<u32, omoba_core::game_proto::TeamGameStart>,
            >,
        >,
    ) {
        self.team_bootstrap_store = Some(store);
    }

    #[cfg(feature = "kcp")]
    pub fn attach_observer_validation(
        &mut self,
        worker: omoba_core::runtime::ObserverValidationWorker,
    ) {
        self.observer_validation = Some(worker);
    }

    #[cfg(feature = "kcp")]
    pub fn attach_reliable_team_sender(&mut self, tx: Sender<OutboundMsg>) {
        self.reliable_team_tx = Some(tx);
    }

    #[cfg(feature = "kcp")]
    pub fn attach_authority_mismatch_rx(
        &mut self,
        rx: crossbeam_channel::Receiver<omoba_core::runtime::ClientHashMismatch>,
    ) {
        self.authority_mismatch_rx = Some(rx);
    }

    #[cfg(feature = "kcp")]
    pub fn attach_client_checkpoint_rx(
        &mut self,
        rx: crossbeam_channel::Receiver<omoba_core::runtime::ClientCheckpointReport>,
    ) {
        self.client_checkpoint_rx = Some(rx);
    }

    #[cfg(feature = "kcp")]
    pub fn attach_rebase_failure_rx(
        &mut self,
        rx: crossbeam_channel::Receiver<omoba_core::runtime::RebaseFailureSignal>,
    ) {
        self.rebase_failure_rx = Some(rx);
    }

    #[cfg(feature = "kcp")]
    pub fn attach_secure_input_security(
        &mut self,
        validation: omoba_core::runtime::SharedSecureInputValidationSnapshot,
        metrics: std::sync::Arc<omoba_core::runtime::SelectiveSecurityMetrics>,
    ) {
        self.secure_input_validation = Some(validation);
        self.selective_security_metrics = Some(metrics);
    }

    /// 階段 5.x 橋接器：註冊與配對的主機輸入接收器
    /// `TickBroadcaster::with_host_input_tx`。每個 `tick()` 都會耗盡待處理的內容
    /// 每個刻度輸入 vecs 並將它們寫入 ECS `PendingPlayerInputs`
    /// 資源，然後將 `player_input_tick::Sys` 路由到遊戲端
    /// 處理程序（StartRound 翻轉 CurrentCreepWave.is_running 等）。
    #[cfg(feature = "kcp")]
    pub fn attach_host_input_rx(
        &mut self,
        rx: crossbeam_channel::Receiver<Vec<(u32, crate::lockstep::PlayerInput, u32)>>,
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
        self.resource_manager
            .handle_tower_request(&mut self.ecs, pd)
    }

    /// 處理玩家相關請求
    pub fn handle_player(&mut self, pd: InboundMsg) -> Result<(), Error> {
        self.resource_manager
            .handle_player_request(&mut self.ecs, pd)
    }

    /// 處理畫面請求
    pub fn handle_screen_request(&mut self, pd: InboundMsg) -> Result<(), Error> {
        self.resource_manager
            .handle_screen_request(&mut self.ecs, pd)
    }

    // 私有初始化方法
    fn initialize_standard_game(&mut self) {
        StateInitializer::init_creep_wave(&mut self.ecs, &self.cw);
        StateInitializer::create_test_scene(&mut self.ecs);
        // 動態實體建完後再填 Region blockers（Searcher 索引一次性完成）
        StateInitializer::populate_region_blockers(&mut self.ecs);
        // 階段 5.2：遺留 0x02 GameEvent 廣播剪輯。塔模板
        // 仍在 — 前端 TD placement UI 需要 cost、placement radius、label。
        self.send_tower_templates();
    }

    fn initialize_campaign_game(&mut self, campaign_data: &CampaignData) {
        StateInitializer::init_campaign_data(&mut self.ecs, campaign_data);
        StateInitializer::init_creep_wave(&mut self.ecs, &self.cw);
        StateInitializer::create_campaign_scene(&mut self.ecs, campaign_data);
        StateInitializer::populate_region_blockers(&mut self.ecs);

        // 英雄知識加成初始化（塔已就緒後套入）
        self.apply_hero_knowledge_bonuses();

        // Phase 5.2: legacy 0x02 GameEvent broadcast cut. tower_templates 保留。
        self.send_tower_templates();
    }

    /// 載入 player_profile.json + knowledge_tree.json，將已解鎖節點的加成
    /// 填入 ECS 的 `KnowledgeBonusResource`，供塔生成時套用。
    fn apply_hero_knowledge_bonuses(&mut self) {
        use crate::config::server_config::read_hero_knowledge_setting;
        use crate::knowledge::{build_bonus_map, load_knowledge_tree, load_profile};
        use omoba_core::comp::KnowledgeBonusResource;

        let gk_cfg = read_hero_knowledge_setting();
        if !gk_cfg.enabled {
            {
                let mut resource = self.ecs.write_resource::<KnowledgeBonusResource>();
                resource.enabled = false;
                resource.bonuses_by_category.clear();
                resource.unlocked_nodes.clear();
            }
            self.reapply_hero_knowledge_buffs_to_live_entities();
            self.hero_knowledge_profile_modified = Self::hero_knowledge_profile_mtime();
            return;
        }

        // 讀 lua_data root（OMB_LUA_CONTENT_ROOT 或 fallback ../scripts/lua_data）
        let lua_root = std::env::var("OMB_LUA_CONTENT_ROOT")
            .unwrap_or_else(|_| "../scripts/lua_data".to_string());
        let lua_root = std::path::PathBuf::from(&lua_root);

        // omb 執行目錄（game.toml 同層）作為 profile 存放位置
        let omb_dir = std::path::PathBuf::from(".");

        let tree = load_knowledge_tree(&lua_root);
        let profile = load_profile(&omb_dir);

        let bonus_map = build_bonus_map(&tree, &profile.unlocked_nodes);

        let mut resource = self.ecs.write_resource::<KnowledgeBonusResource>();
        resource.enabled = profile.enabled;
        resource.bonuses_by_category = bonus_map;
        resource.unlocked_nodes = profile.unlocked_nodes.clone();

        log::info!(
            "[hero_knowledge] 初始化完成：{} 個解鎖節點，{} 個 category 有加成，enabled={}",
            profile.unlocked_nodes.len(),
            resource.bonuses_by_category.len(),
            profile.enabled,
        );
        drop(resource);

        self.reapply_hero_knowledge_buffs_to_live_entities();
        self.hero_knowledge_profile_modified = Self::hero_knowledge_profile_mtime();
    }

    fn hero_knowledge_profile_mtime() -> Option<SystemTime> {
        std::fs::metadata("player_profile.json")
            .and_then(|metadata| metadata.modified())
            .ok()
    }

    fn poll_hero_knowledge_profile_reload(&mut self) {
        let current = Self::hero_knowledge_profile_mtime();
        if current == self.hero_knowledge_profile_modified {
            return;
        }

        log::info!("[hero_knowledge] player_profile.json changed; reloading live knowledge buffs");
        self.apply_hero_knowledge_bonuses();
    }

    fn reapply_hero_knowledge_buffs_to_live_entities(&mut self) {
        let applications: Vec<(specs::Entity, Vec<(String, String)>)> = {
            use omoba_core::comp::KnowledgeBonusResource;

            let gk = self.ecs.read_resource::<KnowledgeBonusResource>();
            let entities = self.ecs.entities();
            let heroes = self.ecs.read_storage::<Hero>();
            let towers = self.ecs.read_storage::<Tower>();
            let script_tags = self.ecs.read_storage::<crate::scripting::ScriptUnitTag>();
            let mut applications = Vec::new();

            for (entity, _) in (&entities, &heroes).join() {
                let buffs = if gk.enabled {
                    gk.bonuses_for("hero").to_vec()
                } else {
                    Vec::new()
                };
                applications.push((entity, buffs));
            }

            for (entity, _) in (&entities, &towers).join() {
                let buffs = script_tags
                    .get(entity)
                    .map(|tag| {
                        let category =
                            omoba_core::runtime::hero_knowledge_category_for_unit_id(&tag.unit_id);
                        if gk.enabled && !category.is_empty() {
                            gk.bonuses_for(category)
                                .iter()
                                .chain(gk.global_bonuses().iter())
                                .cloned()
                                .collect()
                        } else {
                            Vec::new()
                        }
                    })
                    .unwrap_or_default();
                applications.push((entity, buffs));
            }

            applications
        };

        if applications.is_empty() {
            return;
        }

        let mut buff_store = self
            .ecs
            .write_resource::<omoba_core::runtime::ability_runtime::BuffStore>();
        let mut applied_count = 0usize;
        for (entity, buffs) in &applications {
            let old_gk_buffs: Vec<String> = buff_store
                .iter_for(*entity)
                .map(|(buff_id, _)| buff_id)
                .filter(|buff_id| buff_id.starts_with("gk_"))
                .map(str::to_string)
                .collect();
            for buff_id in old_gk_buffs {
                buff_store.remove(*entity, &buff_id);
            }

            for (buff_id, payload_str) in buffs {
                let payload: serde_json::Value = serde_json::from_str(payload_str)
                    .unwrap_or_else(|_| serde_json::Value::Object(Default::default()));
                buff_store.add(
                    *entity,
                    buff_id,
                    omoba_sim::Fixed64::from_raw(i64::MAX),
                    payload,
                );
                applied_count += 1;
            }
        }

        log::info!(
            "[hero_knowledge] reapplied {} knowledge buffs across {} live heroes/towers",
            applied_count,
            applications.len(),
        );
    }

    /// 收集 script registry 內每支塔腳本的 tower_metadata，合併 host TowerTemplate
    /// 的 cost/placement radius/label，廣播 `game/tower_templates` 給前端。
    fn send_tower_templates(&self) {
        use serde_json::json;
        let reg = self
            .ecs
            .read_resource::<crate::comp::tower_registry::TowerTemplateRegistry>();
        let mut templates: Vec<serde_json::Value> = Vec::new();
        // 依 DLL units() 註冊順序 broadcast（Q2 作者意圖優先）
        for tpl in reg.iter_ordered() {
            templates.push(json!({
                "kind": tpl.unit_id,
                "label": tpl.label,
                "cost": tpl.cost,
                "footprint": tpl.footprint,
                "placement_radius": tpl.placement_radius,
                "atk": tpl.atk,
                "asd_interval": tpl.asd_interval,
                "range": tpl.range,
                "bullet_speed": tpl.bullet_speed,
                "splash_radius": tpl.splash_radius,
                "hit_radius": tpl.hit_radius,
                "slow_factor": tpl.slow_factor,
                "slow_duration": tpl.slow_duration,
                "render": {
                    "render_mode": tpl.render.render_mode,
                    "base": tpl.render.base,
                    "barrel": tpl.render.barrel,
                    "visual_size": tpl.render.visual_size,
                    "barrel_frames": tpl.render.barrel_frames,
                    "body_frames": tpl.render.body_frames,
                    "barrel_animation": {
                        "fps": tpl.render.barrel_animation.fps,
                        "loop": tpl.render.barrel_animation.loop_animation,
                        "fire_fps": tpl.render.barrel_animation.fire_fps,
                        "fire_once": tpl.render.barrel_animation.fire_once,
                    },
                    "body_animation": {
                        "fps": tpl.render.body_animation.fps,
                        "loop": tpl.render.body_animation.loop_animation,
                        "fire_fps": tpl.render.body_animation.fire_fps,
                        "fire_once": tpl.render.body_animation.fire_once,
                    },
                    "rotation_mode": tpl.render.rotation_mode,
                    "barrel_layout": tpl.render.barrel_layout,
                    "barrel_variants": tpl.render.barrel_variants.iter().map(|v| json!({
                        "min_path": v.min_path,
                        "min_level": v.min_level,
                        "count": v.count,
                        "image": v.image,
                        "frames": v.frames,
                    })).collect::<Vec<_>>(),
                    "barrel_offset": { "x": tpl.render.barrel_offset.x, "y": tpl.render.barrel_offset.y },
                    "barrel_pivot": { "x": tpl.render.barrel_pivot.x, "y": tpl.render.barrel_pivot.y },
                    "muzzle_offset": { "x": tpl.render.muzzle_offset.x, "y": tpl.render.muzzle_offset.y },
                    "default_angle_deg": tpl.render.default_angle_deg,
                    "recoil": {
                        "mode": tpl.render.recoil.mode,
                        "distance": tpl.render.recoil.distance,
                        "scale": tpl.render.recoil.scale,
                        "duration_ms": tpl.render.recoil.duration_ms,
                        "return_ms": tpl.render.recoil.return_ms,
                    },
                },
                "attack_timing": {
                    "windup": tpl.attack_timing.windup,
                    "backswing": tpl.attack_timing.backswing,
                },
            }));
        }
        let n = templates.len();
        let payload = json!({ "templates": templates });
        let _ = self.mqtx.send(OutboundMsg::new_s(
            "td/all/res",
            "game",
            "tower_templates",
            payload,
        ));
        log::info!("已發送 {} 個 tower template 給前端", n);
    }
}

#[cfg(feature = "kcp")]
fn record_team_frame_evidence(team_id: u32, sequence: u64, replica_tick: u64, encoded: &[u8]) {
    use prost::Message;
    use sha2::Digest;
    use std::io::Write;
    let Ok(root) = std::env::var("OMOBA_FOG_EVIDENCE_DIR") else {
        return;
    };
    let dir = std::path::Path::new(&root)
        .join("server")
        .join(format!("team-{team_id}"));
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("raw-application.capture"))
    {
        let _ = file.write_all(&(encoded.len() as u32).to_be_bytes());
        let _ = file.write_all(encoded);
    }
    if let Ok(frame) = omoba_core::game_proto::TeamTickFrame::decode(encoded) {
        let expected_hash = frame
            .post_step
            .as_ref()
            .and_then(|post| post.hash_checkpoint.as_ref())
            .map(|checkpoint| hex::encode(&checkpoint.canonical_team_hash));
        let transitions = frame.pre_step.as_ref().map(|pre| pre.transitions.iter().map(|transition| serde_json::json!({
            "kind": match transition.transition { Some(omoba_core::game_proto::transition::Transition::Reveal(_)) => "Reveal", Some(omoba_core::game_proto::transition::Transition::Hide(_)) => "Hide", Some(omoba_core::game_proto::transition::Transition::Forget(_)) => "Forget", Some(omoba_core::game_proto::transition::Transition::Replace(_)) => "Replace", None => "None" }
        })).collect::<Vec<_>>()).unwrap_or_default();
        let safe = serde_json::json!({
            "team_id": team_id, "team_sequence": sequence, "replica_tick": replica_tick,
            "view_epoch": frame.view_epoch.map(|value| value.value),
            "transition_count": frame.pre_step.map_or(0, |value| value.transitions.len()),
            "accepted_input_count": frame.step.as_ref().map_or(0, |value| value.accepted_inputs.len()),
            "external_effect_count": frame.step.as_ref().map_or(0, |value| value.external_effects.len()),
            "encoded_sha256": format!("{:x}", sha2::Sha256::digest(encoded)),
            "expected_pre_repair_hash": expected_hash,
            "transitions": transitions,
        });
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("decoded-timeline.jsonl"))
        {
            let _ = serde_json::to_writer(&mut file, &safe);
            let _ = file.write_all(b"\n");
        }
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("expected-timeline.jsonl"))
        {
            let _ = serde_json::to_writer(&mut file, &safe);
            let _ = file.write_all(b"\n");
        }
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(
            std::path::Path::new(&root)
                .join("server")
                .join("disclosure-matrix.jsonl"),
        ) {
            let row = serde_json::json!({"team_id":team_id,"team_sequence":sequence,"replica_tick":replica_tick,"transitions":safe["transitions"]});
            let _ = serde_json::to_writer(&mut file, &row);
            let _ = file.write_all(b"\n");
        }
    }
}

fn record_canonical_timeline(tick: u64, entity_count: usize) {
    use std::io::Write;
    let Ok(root) = std::env::var("OMOBA_FOG_EVIDENCE_DIR") else {
        return;
    };
    let path = std::path::Path::new(&root)
        .join("server")
        .join("canonical-timeline.jsonl");
    let Some(parent) = path.parent() else {
        return;
    };
    let _ = std::fs::create_dir_all(parent);
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = serde_json::to_writer(
            &mut file,
            &serde_json::json!({"authoritative_tick":tick,"canonical_entity_count":entity_count}),
        );
        let _ = file.write_all(b"\n");
    }
}

fn record_three_way_checkpoints(coordinator: &omoba_core::runtime::AuthorityRepairCoordinator) {
    let Ok(root) = std::env::var("OMOBA_FOG_EVIDENCE_DIR") else {
        return;
    };
    let rows: Vec<_> = coordinator.three_way_checkpoints.iter().map(|(key, value)| serde_json::json!({
        "team_id":key.team_id,"replica_tick":key.replica_tick,"team_sequence":key.team_sequence,"authority_revision":key.authority_revision,
        "expected":value.expected_hash.map(hex::encode),
        "observer_pre_repair":value.observer_pre_repair_hash.map(hex::encode),
        "observer_post_repair":value.observer_post_repair_hash.map(hex::encode),
        "external_runtime_pre_repair":value.client_pre_repair_hash.map(hex::encode),
        "external_runtime_post_repair":value.client_post_repair_hash.map(hex::encode),
        "pre_repair_parity":value.pre_repair_parity,
        "post_repair_parity":value.parity,
        "observer_frame_hash":value.observer_frame_hash.map(hex::encode),
        "external_runtime_frame_hash":value.client_frame_hash.map(hex::encode),
        "verdict":format!("{:?}",value.verdict()).to_uppercase()
    })).collect();
    let path = std::path::Path::new(&root)
        .join("server")
        .join("three-way-checkpoints.json");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, serde_json::to_vec_pretty(&rows).unwrap_or_default());
}

#[cfg(all(test, feature = "kcp"))]
mod reliable_outbound_tests {
    use super::*;
    use crossbeam_channel::{bounded, SendTimeoutError};

    #[test]
    fn temporary_backpressure_blocks_then_preserves_sequence() {
        let (tx, rx) = bounded(1);
        tx.send(1u64).unwrap();
        let drain = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            assert_eq!(rx.recv().unwrap(), 1);
            assert_eq!(rx.recv().unwrap(), 2);
        });
        let waited = reliable_send_with_watchdog(&tx, 2, Duration::from_secs(1)).unwrap();
        assert!(waited >= Duration::from_millis(10));
        drop(tx);
        drain.join().unwrap();
    }

    #[test]
    fn watchdog_returns_unsent_frame_instead_of_dropping_it() {
        let (tx, _rx) = bounded(1);
        tx.send(1u64).unwrap();
        let result = reliable_send_with_watchdog(&tx, 2, Duration::from_millis(10));
        assert!(matches!(result, Err(SendTimeoutError::Timeout(2))));
    }
}

#[cfg(feature = "runtime-lua-content")]
fn preserve_cproperty_hp_ratio(prop: &mut CProperty, new_mhp: omoba_sim::Fixed64) {
    let new_hp = scaled_hp(prop.hp, prop.mhp, new_mhp);
    prop.mhp = new_mhp;
    prop.hp = new_hp;
}

#[cfg(feature = "runtime-lua-content")]
fn scaled_hp(
    old_hp: omoba_sim::Fixed64,
    old_mhp: omoba_sim::Fixed64,
    new_mhp: omoba_sim::Fixed64,
) -> omoba_sim::Fixed64 {
    if old_mhp.raw() <= 0 {
        return new_mhp;
    }
    let raw = (old_hp.raw() as i128 * new_mhp.raw() as i128 / old_mhp.raw() as i128)
        .clamp(0, new_mhp.raw() as i128) as i64;
    omoba_sim::Fixed64::from_raw(raw)
}

#[cfg(all(test, feature = "runtime-lua-content"))]
mod dev_lua_hot_reload_tests {
    use super::*;

    #[test]
    fn scaled_hp_preserves_ratio_and_clamps() {
        assert_eq!(
            scaled_hp(
                omoba_sim::Fixed64::from_i32(25),
                omoba_sim::Fixed64::from_i32(100),
                omoba_sim::Fixed64::from_i32(200),
            ),
            omoba_sim::Fixed64::from_i32(50)
        );
        assert_eq!(
            scaled_hp(
                omoba_sim::Fixed64::from_i32(150),
                omoba_sim::Fixed64::from_i32(100),
                omoba_sim::Fixed64::from_i32(200),
            ),
            omoba_sim::Fixed64::from_i32(200)
        );
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
