#![allow(warnings)]
#![allow(unused)]

use log::{info, warn, error, trace, debug};
use crate::state::State;
use std::fs::File;
use std::io::{Write, BufReader, BufRead};
use failure::{err_msg, Error};
use chrono::{NaiveDateTime, Local};
mod ability_runtime;
mod aoi;
mod comp;
mod scripting;
mod tick;
mod ue4;
mod msg;
mod json_preprocessor;
mod item;
mod util;
#[cfg(feature = "mqtt")]
mod mqtt;
mod vision;
mod state;
mod transport;
#[cfg(feature = "kcp")]
mod lockstep;
pub mod config;
use crate::config::server_config::CONFIG;
use crate::json_preprocessor::JsonPreprocessor;
use uuid::Uuid;

use comp::*;
use std::{
    i32,fs,
    ops::{Deref, DerefMut},
    sync::{mpsc, Arc},
    time::{Instant, Duration},
    io,thread,
};
use std::time::SystemTime;
use specs::{
    prelude::Resource,
    Component, DispatcherBuilder, Entity, WorldExt,
};
use serde_json::{self, json};
use crate::ue4::import_map::CreepWaveData;
use crate::ue4::import_campaign::CampaignData;

const TPS: u64 = 30;

fn read_input() -> Option<String> {
    let mut buffer = String::new();

    match io::stdin().read_line(&mut buffer) {
        Ok(0) => None,  // EOF
        Ok(_) => Some(buffer.trim().to_string()),
        Err(_) => None,
    }
}

#[tokio::main]
async fn main() -> std::result::Result<(), Error> {
    log4rs::init_file("log4rs.yml", Default::default()).unwrap();

    // 載入戰役資料（由 game.toml 的 STORY 欄位決定關卡資料夾）
    let story_path = format!("Story/{}", CONFIG.STORY);
    let campaign_data = CampaignData::load_from_path(&story_path)
        .unwrap_or_else(|e| panic!("Failed to load campaign data from {}: {}", story_path, e));

    // 驗證戰役資料完整性
    if let Err(err) = campaign_data.validate() {
        log::error!("Campaign data validation failed: {}", err);
        return Err(err_msg(err));
    }

    log::info!("Campaign '{}' loaded successfully", campaign_data.mission.campaign.name);
    {
        let hid_str = &campaign_data.entity.heroes[0].id;
        let hid = omoba_template_ids::hero_by_name(hid_str).unwrap_or_default();
        log::info!(
            "Hero: {} - {}",
            omoba_template_ids::hero_display(hid),
            omoba_template_ids::hero_title(hid),
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
    let handle = transport::grpc_transport::start(
        server_addr.clone(),
        server_port.clone(),
    ).await?;

    #[cfg(feature = "kcp")]
    let handle = transport::kcp_transport::start(
        server_addr.clone(),
        server_port.clone(),
    ).await?;

    // === TEMP: P7 checkpoint dumper — revert after measurement ===
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
                    dbytes, dbytes / 5, dmsgs, dmsgs / 5, snap.total_bytes
                );
                // Per-event delta
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
                        t, a, db, dm
                    );
                }
            }
        });
    }
    // === END TEMP ===

    // Prevent enabling multiple transport features simultaneously
    #[cfg(all(feature = "mqtt", feature = "grpc"))]
    compile_error!("Cannot enable both 'mqtt' and 'grpc' features simultaneously");
    #[cfg(all(feature = "mqtt", feature = "kcp"))]
    compile_error!("Cannot enable both 'mqtt' and 'kcp' features simultaneously");
    #[cfg(all(feature = "grpc", feature = "kcp"))]
    compile_error!("Cannot enable both 'grpc' and 'kcp' features simultaneously");

    thread::sleep(Duration::from_millis(500));

    // 初始化 ECS
    // P5: pull the shared AOI grid Arc out before moving handle fields, so we
    // can plug it back into State after construction.
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

    // Phase 2 lockstep: spawn the 60Hz TickBroadcaster alongside the legacy
    // 30Hz simulation dispatcher. The broadcaster drains the InputBuffer per
    // tick and emits TickBatch (tag 0x11) + periodic StateHash (tag 0x12) via
    // the existing OutboundMsg channel; the kcp transport's broadcast thread
    // (Task 2.3) routes lockstep frames distinctly from GameEvent.
    //
    // InputBuffer / LockstepState are held in Arc<Mutex<...>> shared with the
    // kcp transport's JoinRequest / InputSubmit handlers (Task 2.3).
    #[cfg(feature = "kcp")]
    let (lockstep_state_handle, input_buffer_handle) = {
        use std::sync::{Arc, Mutex as StdMutex};
        use crate::lockstep::{InputBuffer, LockstepState, TickBroadcaster, TickBroadcasterConfig};

        let master_seed = state.ecs().read_resource::<crate::comp::MasterSeed>().0;
        let lockstep_state = Arc::new(StdMutex::new(LockstepState::new(master_seed)));
        let input_buffer = Arc::new(StdMutex::new(InputBuffer::new()));

        let broadcaster = TickBroadcaster::new(
            TickBroadcasterConfig::default(),
            input_buffer.clone(),
            lockstep_state.clone(),
            handle.tx.clone(),
        );
        tokio::spawn(broadcaster.run());
        log::info!(
            "Lockstep TickBroadcaster spawned at 60Hz (period {}us, state_hash every {} ticks)",
            TickBroadcasterConfig::default().tick_period_us,
            TickBroadcasterConfig::default().state_hash_interval,
        );
        (lockstep_state, input_buffer)
    };
    // Phase 2.3 will wire these into the kcp transport. For now they're just
    // kept alive so the broadcaster's shared state isn't dropped.
    #[cfg(feature = "kcp")]
    let _lockstep_handles = (lockstep_state_handle, input_buffer_handle);

    let mut clock = Clock::new(Duration::from_secs_f64(1.0 / TPS as f64));

    // Game speed multiplier (debug)。每 real frame 跑 N 個 sub-tick，sim 推進 N×dt。
    // 從 game.toml 讀 default，stdin 指令 `:speed N` 可 runtime 切換。clamp 1..=16。
    let mut speed_mult: u32 = CONFIG.SPEED_MULT.clamp(1, 16);
    log::info!("⏩ Game speed: {}× (use ':speed N' on stdin to change, range 1..=16)", speed_mult);

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
        // 跑 N 個 sub-tick；speed=1 時等同舊行為（單 tick 用 clock.dt()）
        let dt = clock.dt();
        for _ in 0..speed_mult {
            if let Err(e) = state.tick(dt) {
                log::error!("Tick error: {:?}", e);
            }
        }

        // Wait for the next tick.
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
