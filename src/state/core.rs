use core::time::Duration;
use crossbeam_channel::{Receiver, Sender};
use failure::Error;
use omoba_core::lockstep_timing::{
    lockstep_dt_fixed_raw_for_tick, LOCKSTEP_TEN_SECONDS_TICKS_U64,
    LOCKSTEP_THIRTY_SECONDS_TICKS_U64, LOCKSTEP_TPS_U64,
};
use rayon::ThreadPool;
use specs::{Join, World, WorldExt};
/// 遊戲狀態核心結構
use std::sync::Arc;
use std::time::Instant;

use crate::scripting::{self, ScriptRegistry};
use crate::transport::{InboundMsg, OutboundMsg};
#[cfg(any(feature = "grpc", feature = "kcp"))]
use crate::transport::{QueryRequest, Viewport, ViewportMsg};
use crate::ue4::import_campaign::CampaignData;
use crate::ue4::import_map::CreepWaveData;
use crate::{comp::*, CreepWave};
use std::collections::BTreeMap;
#[cfg(any(feature = "grpc", feature = "kcp"))]
use std::collections::{HashMap, HashSet};

use super::{ResourceManager, StateInitializer, SystemDispatcher, TimeManager};

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
    /// 上次執行可見度差異時「local_tick」的值
    last_visibility_tick: u64,
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
    /// 階段 5.x 橋接器：與 `TickBroadcaster::host_input_tx` 配對的接收器。
    /// 每個廣播公司都會從「InputBuffer」消耗輸入一段時間
    /// `TickBatch` 也會沿著這個通道發送一個副本； `State::tick` 排水溝
    /// 並將輸入寫入“PendingPlayerInputs”，以便主機的
    /// `player_input_tick::Sys` 也能看到它們。主機與 broadcaster 現在同為
    /// 120Hz，但仍排空所有可用批次以便短暫 stall 後追上。
    #[cfg(feature = "kcp")]
    host_input_rx: Option<crossbeam_channel::Receiver<Vec<(u32, crate::lockstep::PlayerInput)>>>,
}

/// 每個玩家可見的實體集，按類型劃分，以便規範“Entity::id()”
/// 跨不同儲存的重複使用不會在單一「HashSet<u32>」內發生衝突。
#[cfg(any(feature = "grpc", feature = "kcp"))]
#[derive(Default, Debug)]
struct VisSet {
    heroes: HashSet<u32>,
    units: HashSet<u32>,
    creeps: HashSet<u32>,
    towers: HashSet<u32>,
}

#[cfg(any(feature = "grpc", feature = "kcp"))]
const VISIBILITY_DIFF_INTERVAL_TICKS: u64 = LOCKSTEP_TPS_U64 / 5;

/// 每個玩家至少強制發送一個（可能是空的）心跳，這樣
/// 客戶端仍然會收到“tick”/“game_time”心跳以進行時鐘同步和
/// 即使玩家的 AOI 中的 HP 值沒有變化，也能保持活躍度。空的
/// 在 prost+LZ4 之後，心跳壓縮到約 50 位元組 — 便宜的 keepalive。
#[cfg(any(feature = "grpc", feature = "kcp"))]
const HEARTBEAT_FORCE_SEND_INTERVAL: f64 = 5.0;

/// 階段 3.4：每 N 個調度程式週期發出一個狀態雜湊樣本。調度員
/// 以 lockstep cadence 運行，因此此值代表約 10 秒。
/// 廣播公司的間隔觸發（最多有一個陳舊時間）。
#[cfg(feature = "kcp")]
const STATE_HASH_INTERVAL_TICKS: u64 = LOCKSTEP_TEN_SECONDS_TICKS_U64;

/// 階段 5.3：每 N 個調度程序週期序列化一個新的世界快照。
/// 調度程式以 lockstep cadence 運行，因此此值代表約 30 秒 — 觀察者重新加入最多獲得一個
/// 快照擷取和引導之間有 30 秒的間隔。跳過 `tick=0`
/// （讓世界在第一次捕獲之前完成 init）。
#[cfg(feature = "kcp")]
const SNAPSHOT_INTERVAL_TICKS: u64 = LOCKSTEP_THIRTY_SECONDS_TICKS_U64;

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
            #[cfg(feature = "runtime-lua-content")]
            dev_lua_hot_reload: None,
            #[cfg(feature = "kcp")]
            aoi_grid: None,
            #[cfg(feature = "kcp")]
            state_hash_tx: None,
            #[cfg(feature = "kcp")]
            snapshot_store: None,
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
    /// 階段 3.2：將 populate_* 助手提取到
    /// `狀態::初始化::{populate_tower_template_registry,
    /// populate_tower_upgrade_registry、populate_ability_registry}`，讓
    /// local replica bootstrap 可以重複使用相同引導程式碼。
    fn load_scripts(&mut self) {
        let dir_str = std::env::var("OMB_SCRIPTS_DIR").unwrap_or_else(|_| "./scripts".to_string());
        let dir = std::path::Path::new(&dir_str);
        self.script_registry = crate::scripting::loader::load_scripts_dir(dir);
        super::initialization::populate_tower_template_registry(
            &mut self.ecs,
            &self.script_registry,
        );
        super::initialization::populate_tower_upgrade_registry(&mut self.ecs);
        super::initialization::populate_ability_registry(&mut self.ecs, &self.script_registry);
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
        super::initialization::populate_tower_template_registry(
            &mut self.ecs,
            &self.script_registry,
        );
        super::initialization::populate_tower_upgrade_registry(&mut self.ecs);
        super::initialization::populate_ability_registry(&mut self.ecs, &self.script_registry);
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
            #[cfg(feature = "runtime-lua-content")]
            dev_lua_hot_reload: None,
            #[cfg(feature = "kcp")]
            aoi_grid: None,
            #[cfg(feature = "kcp")]
            state_hash_tx: None,
            #[cfg(feature = "kcp")]
            snapshot_store: None,
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
        let dt_fixed_raw = lockstep_dt_fixed_raw_for_tick(self.local_tick);

        // 更新時間管理
        self.time_manager
            .update(&mut self.ecs, dt, Some(dt_fixed_raw))?;

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

        self.flush_runtime_events();

        // 階段 2.1：耗盡 `PendingTowerSpawnQueue` 填充
        // 上述調度期間的`player_input_tick::Sys`。需要 `&mut World`
        // （TowerTemplateRegistry 尋找 + 實體建立 + ScriptEvent::Spawn
        // Push) 是「System」的規格無法借用。local replica 在自己的
        // dispatcher 運行後使用相同 boundary drain。
        crate::comp::GameProcessor::drain_pending_tower_spawns(&mut self.ecs);

        // 階段 2.2：排出`PendingTowerSellQueue`（TowerSell 鎖步輸入）
        // — 相同的「&mut World」要求（金幣+BuffStore清除+
        // 實體刪除）。local replica 使用相同 boundary。
        crate::comp::GameProcessor::drain_pending_tower_sells(&mut self.ecs);

        // 階段 2.3：排空`PendingTowerUpgradeQueue`（TowerUpgrade 鎖步
        // 輸入） - 需要 `&mut World` （TowerUpgradeRegistry 讀取，驗證
        // 透過 tower_upgrade_rules，扣除 Gold，寫入 Tower.upgrade_levels +
        // Upgrade_flags，將 StatMod 推入 BuffStore）。local replica 使用相同 boundary。
        crate::comp::GameProcessor::drain_pending_tower_upgrades(&mut self.ecs);

        // 階段 2.4：排出 `PendingItemUseQueue` （ItemUse 鎖步輸入） —
        // 需要`&mut World`（ItemRegistry讀取，寫入Inventory冷卻時間，
        // 為專案效果編寫 CProperty）。副本反映了這一點
        // sim_runner。
        crate::comp::GameProcessor::drain_pending_item_uses(&mut self.ecs);

        // AbilityUpgrade：消耗 skill point 並在 script dispatch 前排入 SkillLearn。
        // Replica 端在 sim_runner 中鏡像同一流程。
        crate::comp::GameProcessor::drain_pending_ability_upgrades(&mut self.ecs);

        // AbilityCast：在 script dispatch 前排入 SkillCast。放在 upgrades 後 drain，
        // 讓同 tick 的 Shift+key 學習後再 key cast 可以成功。
        crate::comp::GameProcessor::drain_pending_ability_casts(&mut self.ecs);

        // MoveTo (右鍵移動): drain `PendingMoveQueue` — writes `MoveTarget`
        // 玩家英雄實體上的組件。副本反映了這一點
        // sim_runner。
        crate::comp::GameProcessor::drain_pending_moves(&mut self.ecs);

        // 腳本 dispatch 階段（E1 — 序列、獨佔 World）
        // 放在並行系統之後、其他序列處理之前，確保腳本能看到本 tick 的
        // 完整戰鬥結果，也能修改狀態讓下游處理看見。
        let t_dispatch = Instant::now();
        let dt_fx = omoba_template_ids::Fixed64::from_raw(dt_fixed_raw);
        scripting::run_script_dispatch(
            &mut self.ecs,
            &self.script_registry,
            self.local_tick,
            dt_fx,
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
        self.resource_manager
            .process_player_data(&mut self.ecs, &self.mqrx)?;

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
        if self.local_tick % STATE_HASH_INTERVAL_TICKS == 0 {
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
        if self.local_tick > 0 && self.local_tick % SNAPSHOT_INTERVAL_TICKS == 0 {
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
        for msg in crate::runtime_events::runtime_events_to_outbound(events) {
            let _ = self.mqtx.try_send(msg);
        }
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
                    self.client_visibility.remove(&player_name);
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

    /// 階段 5.x 橋接器：註冊與配對的主機輸入接收器
    /// `TickBroadcaster::with_host_input_tx`。每個 `tick()` 都會耗盡待處理的內容
    /// 每個刻度輸入 vecs 並將它們寫入 ECS `PendingPlayerInputs`
    /// 資源，然後將 `player_input_tick::Sys` 路由到遊戲端
    /// 處理程序（StartRound 翻轉 CurrentCreepWave.is_running 等）。
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

        // Phase 5.2: legacy 0x02 GameEvent broadcast cut. tower_templates 保留。
        self.send_tower_templates();
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
