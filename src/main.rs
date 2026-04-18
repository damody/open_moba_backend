#![allow(warnings)]
#![allow(unused)]

use log::{info, warn, error, trace, debug};
use crate::state::State;
use std::fs::File;
use std::io::{Write, BufReader, BufRead};
use failure::{err_msg, Error};
use chrono::{NaiveDateTime, Local};
mod comp;
mod tick;
mod ue4;
mod msg;
mod json_preprocessor;
mod item;
#[cfg(feature = "mqtt")]
mod mqtt;
mod vision;
mod state;
mod transport;
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

const TPS: u64 = 10;

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

    // 載入戰役資料
    let campaign_data = CampaignData::load_from_path("Story/MVP_1")
        .expect("Failed to load campaign data from Story/MVP_1");

    // 驗證戰役資料完整性
    if let Err(err) = campaign_data.validate() {
        log::error!("Campaign data validation failed: {}", err);
        return Err(err_msg(err));
    }

    log::info!("Campaign '{}' loaded successfully", campaign_data.mission.campaign.name);
    log::info!("Hero: {} - {}", campaign_data.entity.heroes[0].name, campaign_data.entity.heroes[0].title);
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

    // Prevent enabling multiple transport features simultaneously
    #[cfg(all(feature = "mqtt", feature = "grpc"))]
    compile_error!("Cannot enable both 'mqtt' and 'grpc' features simultaneously");
    #[cfg(all(feature = "mqtt", feature = "kcp"))]
    compile_error!("Cannot enable both 'mqtt' and 'kcp' features simultaneously");
    #[cfg(all(feature = "grpc", feature = "kcp"))]
    compile_error!("Cannot enable both 'grpc' and 'kcp' features simultaneously");

    thread::sleep(Duration::from_millis(500));

    // 初始化 ECS
    let mut state = State::new_with_campaign(
        campaign_data,
        handle.tx.clone(),
        handle.rx,
        #[cfg(any(feature = "grpc", feature = "kcp"))]
        handle.query_rx,
        #[cfg(any(feature = "grpc", feature = "kcp"))]
        handle.viewport_rx,
    );
    let mut clock = Clock::new(Duration::from_secs_f64(1.0 / TPS as f64));

    // 啟動 MQTT 測試介面
    #[cfg(feature = "mqtt")]
    {
        use crate::mqtt::test_interface::MqttTestInterfaceManager;
        let test_interface = MqttTestInterfaceManager::new(handle.tx.clone());
        test_interface.start(server_addr.clone(), server_port.clone());
        log::info!("MQTT test interface started, listening on topic ability_test/command");
    }

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
            state.send_chat(msg)
        }
        if let Err(e) = state.tick(clock.dt()) {
            log::error!("Tick error: {:?}", e);
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
