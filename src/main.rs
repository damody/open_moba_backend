#![allow(warnings)]
#![allow(unused)]

use crate::state::State;
use chrono::{Local, NaiveDateTime};
use failure::{err_msg, Error};
use log::{debug, error, info, trace, warn};
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
mod ability_runtime;
mod aoi;
mod comp;
pub mod config;
mod item;
mod json_preprocessor;
#[cfg(feature = "kcp")]
mod lockstep;
#[cfg(feature = "mqtt")]
mod mqtt;
mod msg;
mod scripting;
mod state;
mod tick;
mod transport;
mod ue4;
mod util;
mod vision;
use crate::config::server_config::CONFIG;
use crate::json_preprocessor::JsonPreprocessor;
use uuid::Uuid;

use crate::ue4::import_campaign::CampaignData;
use crate::ue4::import_map::CreepWaveData;
use comp::*;
use omoba_core::lockstep_timing::{LOCKSTEP_DT_F64, LOCKSTEP_TPS_U64};
use serde_json::{self, json};
use specs::{prelude::Resource, Component, DispatcherBuilder, Entity, WorldExt};
use std::time::SystemTime;
use std::{
    fs, i32, io,
    ops::{Deref, DerefMut},
    sync::{mpsc, Arc},
    thread,
    time::{Duration, Instant},
};

const TPS: u64 = LOCKSTEP_TPS_U64;

fn read_input() -> Option<String> {
    let mut buffer = String::new();

    match io::stdin().read_line(&mut buffer) {
        Ok(0) => None, // EOF
        Ok(_) => Some(buffer.trim().to_string()),
        Err(_) => None,
    }
}

#[tokio::main]
async fn main() -> std::result::Result<(), Error> {
    log4rs::init_file("log4rs.yml", Default::default()).unwrap();

    if omoba_template_ids::ensure_runtime_lua_content().map_err(err_msg)? {
        log::info!("Runtime Lua content mode enabled");
    }

    // 載入戰役資料（由 game.toml 的 STORY 欄位決定 generated story id）。
    let campaign_data =
        crate::ue4::import_campaign::load_generated(&CONFIG.STORY).unwrap_or_else(|e| {
            panic!(
                "Failed to load generated campaign '{}': {}",
                CONFIG.STORY, e
            )
        });

    // 驗證戰役資料完整性
    if let Err(err) = campaign_data.validate() {
        log::error!("Campaign data validation failed: {}", err);
        return Err(err_msg(err));
    }

    log::info!(
        "Campaign '{}' loaded successfully",
        campaign_data.mission.campaign.name
    );
    {
        let hid_str = &campaign_data.entity.heroes[0].id;
        let hid = omoba_template_ids::hero_by_name(hid_str).unwrap_or_default();
        log::info!(
            "Hero: {} - {}",
            omoba_template_ids::active_hero_display(hid),
            omoba_template_ids::active_hero_title(hid),
        );
    }
    log::info!("Total stages: {}", campaign_data.mission.stages.len());
    log::info!("Total abilities: {}", campaign_data.ability.abilities.len());

    // 初始化 transport
    let server_addr = CONFIG.SERVER_IP.clone();
    let server_port = CONFIG.SERVER_PORT.clone();
    let client_id = CONFIG.CLIENT_ID.clone();

    #[cfg(feature = "mqtt")]
    let handle = transport::mqtt_transport::start(
        server_addr.clone(),
        server_port.clone(),
        client_id.clone(),
    )?;

    #[cfg(feature = "grpc")]
    let handle = transport::grpc_transport::start(server_addr.clone(), server_port.clone()).await?;

    // 步驟 2 鎖定步驟：預先建立共用狀態，以便我們可以將其傳遞給
    // kcp 傳輸（任務 2.3 — 處理 0x10/0x13/0x15）和
    // TickBroadcaster（在下面生成）。 MasterSeed::default() 傳回相同的結果
    // ECS 資源在 state::initialization 中初始化的值，因此
    // 兩個程式碼路徑看到相同的種子。
    //
    // 階段 5.3 新增了第三個 Arc<Mutex<>> — SnapshotStore — 由以下人員編寫
    // 調度程式每 30 秒循環一次並由 kcp 傳輸讀取
    // 0x16 SnapshotResp 處理程序。
    #[cfg(feature = "kcp")]
    let (lockstep_state_handle, input_buffer_handle, snapshot_store_handle) = {
        use crate::lockstep::{InputBuffer, LockstepState};
        use std::sync::{Arc, Mutex as StdMutex};
        let master_seed = crate::comp::MasterSeed::default().0;
        let lockstep_state = Arc::new(StdMutex::new(LockstepState::new(master_seed)));
        let input_buffer = Arc::new(StdMutex::new(InputBuffer::new()));
        let snapshot_store = Arc::new(StdMutex::new(crate::comp::SnapshotStore::default()));
        (lockstep_state, input_buffer, snapshot_store)
    };

    #[cfg(feature = "kcp")]
    let handle = transport::kcp_transport::start(
        server_addr.clone(),
        server_port.clone(),
        input_buffer_handle.clone(),
        lockstep_state_handle.clone(),
        snapshot_store_handle.clone(),
    )
    .await?;

    // === TEMP：P7 檢查點轉儲器 — 測量後恢復 ===
    // 顯示 per-window delta（不是累積！）— 修正前一版誤導。
    #[cfg(feature = "kcp")]
    {
        use std::collections::HashMap;
        let counter = handle.counter.clone();
        std::thread::spawn(move || {
            let mut last_total_bytes: u64 = 0;
            let mut last_total_msgs: u64 = 0;
            let mut last_per: HashMap<(String, String), (u64, u64)> = HashMap::new();
            loop {
                std::thread::sleep(std::time::Duration::from_secs(5));
                let snap = counter.snapshot();
                let dbytes = snap.total_bytes - last_total_bytes;
                let dmsgs = snap.total_msgs - last_total_msgs;
                last_total_bytes = snap.total_bytes;
                last_total_msgs = snap.total_msgs;
                log::info!(
                    "[kcp-p7 Δ5s] bytes={} ({}B/s)  msgs={} ({}m/s)  cum_total={}B",
                    dbytes,
                    dbytes / 5,
                    dmsgs,
                    dmsgs / 5,
                    snap.total_bytes
                );
                // 每個事件的增量
                let mut deltas: Vec<((String, String), (u64, u64))> = Vec::new();
                for (k, v) in snap.per_event.iter() {
                    let prev = last_per.get(k).copied().unwrap_or((0, 0));
                    let db = v.0 - prev.0;
                    let dm = v.1 - prev.1;
                    if db > 0 || dm > 0 {
                        deltas.push((k.clone(), (db, dm)));
                    }
                    last_per.insert(k.clone(), *v);
                }
                deltas.sort_by_key(|(_, v)| std::cmp::Reverse(v.0));
                for ((t, a), (db, dm)) in deltas.into_iter().take(12) {
                    log::info!(
                        "[kcp-p7 Δ5s]   {:>14}.{:<10}  +bytes={:>8}  +msgs={:>6}",
                        t,
                        a,
                        db,
                        dm
                    );
                }
            }
        });
    }
    // === 結束溫度 ===

    // 防止同時啟用多個傳輸功能
    #[cfg(all(feature = "mqtt", feature = "grpc"))]
    compile_error!("Cannot enable both 'mqtt' and 'grpc' features simultaneously");
    #[cfg(all(feature = "mqtt", feature = "kcp"))]
    compile_error!("Cannot enable both 'mqtt' and 'kcp' features simultaneously");
    #[cfg(all(feature = "grpc", feature = "kcp"))]
    compile_error!("Cannot enable both 'grpc' and 'kcp' features simultaneously");

    thread::sleep(Duration::from_millis(500));

    // 初始化 ECS
    // P5：在移動手柄字段之前拉出共享 AOI 網格弧，所以我們
    // 施工完成後可以插回狀態。
    #[cfg(feature = "kcp")]
    let aoi_grid = handle.aoi.clone();
    let mut state = State::new_with_campaign(
        campaign_data,
        handle.tx.clone(),
        handle.rx,
        #[cfg(any(feature = "grpc", feature = "kcp"))]
        handle.query_rx,
        #[cfg(any(feature = "grpc", feature = "kcp"))]
        handle.viewport_rx,
    );
    #[cfg(feature = "kcp")]
    state.attach_aoi_grid(aoi_grid);
    // 階段 5.3：對共享 SnapshotStore 進行執行緒化，以便調度程式循環
    // 將其週期性快照位元組鏡像到相同的 Arc kcp 傳輸中
    // 從提供 0x16 SnapshotResp 時讀取。
    #[cfg(feature = "kcp")]
    state.attach_snapshot_store(snapshot_store_handle.clone());
    // 階段 5.x 橋接器：與 TickBroadcaster 的 host_input_tx 配對的接收器
    // （連線如下）。 State::tick Drains 每個tick 排出的輸入批次，並且
    // 將它們鏡像到調度程式的 PendingPlayerInputs 中。
    #[cfg(feature = "kcp")]
    let host_input_tx = {
        let (host_input_tx, host_input_rx) = crossbeam_channel::unbounded();
        state.attach_host_input_rx(host_input_rx);
        host_input_tx
    };

    // 階段 2 鎖定步：產生 120Hz TickBroadcaster，與 authoritative dispatcher
    // 使用相同 cadence。廣播者每消耗一次InputBuffer
    // 勾選並透過以下方式發出 TickBatch（標籤 0x11）+ 週期性 StateHash（標籤 0x12）
    // 現有的 OutboundMsg 通道； kcp 傳輸的廣播線程
    // 與 GameEvent 不同的是，路由鎖定步幀。
    //
    // InputBuffer / LockstepState 是在上面建立的（在transport.start之前）
    // 並與 kcp 傳輸的 JoinRequest / InputSubmit 處理程序共用。
    //
    // 階段 3.4：同時建立排程器 → 廣播程式 `state_hash`
    // 通道和電線兩端。發送方已在國家註冊
    // （每個 STATE_HASH_INTERVAL_TICKS 從 `tick()` 呼叫）；接收端
    // 透過“with_state_hash_rx”附加到廣播公司。
    #[cfg(feature = "kcp")]
    {
        use crate::lockstep::{TickBroadcaster, TickBroadcasterConfig};
        let (state_hash_tx, state_hash_rx) = crossbeam_channel::unbounded();
        state.set_state_hash_tx(state_hash_tx);
        let broadcaster = TickBroadcaster::new(
            TickBroadcasterConfig::default(),
            input_buffer_handle.clone(),
            lockstep_state_handle.clone(),
            handle.tx.clone(),
        )
        .with_state_hash_rx(state_hash_rx)
        .with_host_input_tx(host_input_tx.clone());
        tokio::spawn(broadcaster.run());
        log::info!(
            "Lockstep TickBroadcaster spawned at {}Hz (period {}us, state_hash every {} ticks)",
            TPS,
            TickBroadcasterConfig::default().tick_period_us,
            TickBroadcasterConfig::default().state_hash_interval,
        );
    }
    // 保持句柄處於活動狀態，這樣共享狀態就不會被丟棄。
    #[cfg(feature = "kcp")]
    let _lockstep_handles = (
        lockstep_state_handle,
        input_buffer_handle,
        snapshot_store_handle,
    );

    let fixed_dt = Duration::from_secs_f64(LOCKSTEP_DT_F64);
    let mut clock = Clock::new(fixed_dt);

    // Game speed multiplier (debug)。每 real frame 跑 N 個 sub-tick，sim 推進 N×dt。
    // 從 game.toml 讀 default，stdin 指令 `:speed N` 可 runtime 切換。clamp 1..=16。
    let mut speed_mult: u32 = CONFIG.SPEED_MULT.clamp(1, 16);
    log::info!(
        "⏩ Game speed: {}× (use ':speed N' on stdin to change, range 1..=16)",
        speed_mult
    );

    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        loop {
            match read_input() {
                Some(msg) if !msg.is_empty() => {
                    if tx.send(msg).is_err() {
                        break;
                    }
                }
                Some(_) => {} // empty line, continue
                None => {
                    log::info!("stdin closed (EOF), stopping input reader");
                    break;
                }
            }
        }
    });
    loop {
        for msg in rx.try_iter().take(10) {
            // 攔截 `:speed N` 指令，不轉發到 chat
            if let Some(rest) = msg.strip_prefix(":speed") {
                let arg = rest.trim();
                match arg.parse::<u32>() {
                    Ok(n) if (1..=16).contains(&n) => {
                        speed_mult = n;
                        log::info!("⏩ Game speed → {}×", speed_mult);
                    }
                    _ => log::warn!("⏩ Invalid ':speed' arg {:?}, expected integer 1..=16", arg),
                }
                continue;
            }
            state.send_chat(msg)
        }
        // 跑 N 個 sub-tick；speed=1 時每迴圈推進一個固定 120Hz sim tick。
        let dt = fixed_dt;
        for _ in 0..speed_mult {
            if let Err(e) = state.tick(dt) {
                log::error!("Tick error: {:?}", e);
            }
        }

        // 等待下一個滴答聲。
        clock.tick();
    }
    Ok(())
}

pub trait DateTimeNow {
    fn now() -> NaiveDateTime;
}

impl DateTimeNow for NaiveDateTime {
    fn now() -> NaiveDateTime {
        let dt = Local::now();
        dt.naive_local()
    }
}
