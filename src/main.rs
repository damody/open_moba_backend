#![allow(warnings)]
#![allow(unused)]

#[macro_use]
extern crate specs_derive;

use log::{info, warn, error, trace, debug};
use std::fs::File;
use std::io::{Write, BufReader, BufRead};
use failure::{err_msg, Error};
use chrono::{NaiveDateTime, Local};
mod comp;
mod tick;
mod ue4;
mod msg;
mod json_preprocessor;
mod mqtt;
mod vision;
use crate::msg::MqttMsg;
pub mod config;
use crate::config::server_config::CONFIG;
use crate::json_preprocessor::JsonPreprocessor;
use uuid::Uuid;
use regex::Regex;
use crate::msg::PlayerData;

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
    shred::{Fetch, FetchMut},
    storage::{MaskedStorage as EcsMaskedStorage, Storage as EcsStorage},
    Component, DispatcherBuilder, Entity, WorldExt,
};
use serde_json::{self, json};
use crate::ue4::import_map::CreepWaveData;
use crate::ue4::import_campaign::CampaignData;
use rumqttc::{Client, Connection, MqttOptions, QoS};
use crossbeam_channel::{bounded, select, tick, Receiver, Sender};
use crate::mqtt::test_interface::MqttTestInterfaceManager;

const TPS: u64 = 10;

fn create_mqtt_client(server_addr: String, server_port: String, client_id: String, sub: bool) -> Result<(Client, Connection), Error> {
    let mut mqtt_options = MqttOptions::new(client_id.as_str(), server_addr.as_str(), server_port.parse::<u16>()?);
    mqtt_options.set_keep_alive(Duration::from_secs(10));
    mqtt_options.set_request_channel_capacity(10000);
    mqtt_options.set_clean_session(true);
    let (mut mqtt_cli, mut connection) = Client::new(mqtt_options.clone(), 100000);
    if sub {
        mqtt_cli.subscribe("td/+/send", QoS::AtMostOnce).unwrap();
        info!("🔔 Backend subscribed to MQTT topic: td/+/send");
    }
    Ok((mqtt_cli, connection))
}

fn read_input() -> String {
    let mut buffer = String::new();

    io::stdin()
        .read_line(&mut buffer)
        .expect("Failed to read input");

    buffer.trim().to_string()
}

#[async_std::main]
async fn main() -> std::result::Result<(), Error> {
    log4rs::init_file("log4rs.yml", Default::default()).unwrap();
    
    // 載入戰役資料
    let campaign_data = CampaignData::load_from_path("Story/B01_1")
        .expect("Failed to load campaign data from Story/B01_1");
    
    // 驗證戰役資料完整性
    if let Err(err) = campaign_data.validate() {
        log::error!("Campaign data validation failed: {}", err);
        return Err(err_msg(err));
    }
    
    log::info!("Campaign '{}' loaded successfully", campaign_data.mission.campaign.name);
    log::info!("Hero: {} - {}", campaign_data.entity.heroes[0].name, campaign_data.entity.heroes[0].title);
    log::info!("Total stages: {}", campaign_data.mission.stages.len());
    log::info!("Total abilities: {}", campaign_data.ability.abilities.len());
    
    // 初始化mqtt
    let server_addr = CONFIG.SERVER_IP.clone();
    let server_port = CONFIG.SERVER_PORT.clone();
    let client_id = CONFIG.CLIENT_ID.clone();
    let mqtt_url = "tcp://".to_owned() + &server_addr + ":" + &server_port;
    log::info!("{}", mqtt_url);
    let (mqtx, rx): (Sender<MqttMsg>, Receiver<MqttMsg>) = bounded(10000);
    let mqrx = pub_mqtt_loop(server_addr.clone(), server_port.clone(), rx.clone(), client_id.clone()).await?;
    thread::sleep(Duration::from_millis(500));
    // 初始化 ECS
    let mut state = State::new_with_campaign(campaign_data, mqtx.clone(), mqrx);
    let mut clock = Clock::new(Duration::from_secs_f64(1.0 / TPS as f64));
    
    // 啟動 MQTT 測試介面
    let test_interface = MqttTestInterfaceManager::new(mqtx.clone());
    test_interface.start(server_addr.clone(), server_port.clone());
    log::info!("MQTT 測試介面已啟動，監聽主題 ability_test/command");
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        loop {
            let msg = read_input();
            tx.send(msg).unwrap();
        }
    });
    loop {
        for msg in rx.try_iter() {
            state.send_chat(msg)
        }
        state.tick(clock.dt());
        
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

fn generate_client_id() -> String {
    let s = format!("td_{}", Uuid::new_v4().as_simple());
    (&s[..3]).to_string()
}

async fn pub_mqtt_loop(server_addr: String, server_port: String, rx1: Receiver<MqttMsg>, client_id: String) -> Result<Receiver<PlayerData>, Error> {
    let (tx, rx): (Sender<PlayerData>, Receiver<PlayerData>) = bounded(10000);
    let server_addr = server_addr.clone();
    let server_port = server_port.clone();
    let rx1 = rx1.clone();
    thread::spawn(move || -> Result<(), Error> {
        let update = tick(Duration::from_millis(100));
        let mut msgs: Vec<MqttMsg> = vec![];
        loop {
            let tx = tx.clone();
            let (mut mqtt2, mut connection) = create_mqtt_client(server_addr.clone(), server_port.clone(), generate_client_id(), true)?;
            let (btx, brx): (Sender<bool>, Receiver<bool>) = bounded(10);
            thread::spawn(move || {
                let rex_td = Regex::new(r"td/([\w\S]+)/send").unwrap();
                for (i, notification) in connection.iter().enumerate() {
                    thread::sleep(Duration::from_millis(1));
                    if let Ok(x) = notification {
                        // Log all MQTT events for debugging
                        debug!("📡 MQTT Event received: {:?}", x);
                        if let rumqttc::Event::Incoming(x) = x {
                            if let rumqttc::v4::Packet::Publish(x) = x {
                                let handle = || -> Result<(), Error> {
                                    let payload = x.payload;
                                    let msg = match std::str::from_utf8(&payload[..]) {
                                        Ok(msg) => msg,
                                        Err(err) => {
                                            return Err(failure::err_msg(format!("Failed to decode publish message {:?}", err)));
                                            //continue;
                                        }
                                    };
                                    let topic_name = x.topic.as_str();
                                    
                                    // 詳細記錄接收到的MQTT消息
                                    info!("📨 Backend received MQTT message - Topic: {}, Payload: {}", topic_name, msg);
                                    
                                    let vo: serde_json::Result<PlayerData> = serde_json::from_str(&msg);
                                    if let Ok(v) = vo {
                                        info!("✅ Successfully parsed PlayerData - name: {}, t: {}, a: {}", v.name, v.t, v.a);
                                        tx.try_send(v);
                                    } else {
                                        warn!("❌ Json Parser error for topic: {} payload: {}", topic_name, msg);
                                    };
                                    Ok(())
                                };
                                if let Err(msg) = handle() {
                                    println!("{:?}", msg);
                                    continue;
                                }
                            }
                        }
                    } else {
                        btx.try_send(true).unwrap();
                        break;
                    }
                }
            });
            loop {
                select! {
                    recv(update) -> d => {
                        let mut i: usize = 0;
                        loop {
                            if msgs.len() <= i {
                                break;
                            }
                            let diff = msgs[i].time.duration_since(SystemTime::now());
                            let mut difftime = 0;
                            match diff {
                                Ok(n) => { difftime = n.as_micros(); },
                                Err(_) => {},
                            }
                            if difftime == 0 {
                                let msg_res = mqtt2.publish(msgs[i].topic.clone(), QoS::AtMostOnce, false, msgs[i].msg.clone());
                                match msg_res {
                                    Ok(_) =>{},
                                    Err(x) => {
                                        warn!("??? {}", x);
                                    }
                                }
                                msgs.remove(i);
                            } else {
                                i += 1;
                            }
                        }
                    },
                    recv(brx) -> d => {
                        break;
                    },
                    recv(rx1) -> d => {
                        let handle = || -> Result<(), Error>
                        {
                            if let Ok(d) = d {
                                let diff = d.time.duration_since(SystemTime::now());
                                let mut difftime = 0;
                                match diff {
                                    Ok(n) => { difftime = n.as_micros(); },
                                    Err(_) => {},
                                }
                                if d.topic.len() > 2 {
                                    if difftime == 0 {
                                        info!("🚀 正在發布 MQTT 消息到主題: {} - 內容: {}", d.topic, d.msg);
                                        let msg_res = mqtt2.publish(d.topic.clone(), QoS::AtMostOnce, false, d.msg.clone());
                                        match msg_res {
                                            Ok(_) => {
                                                info!("✅ MQTT 消息發布成功 - 主題: {}", d.topic);
                                            },
                                            Err(x) => {
                                                warn!("❌ MQTT 消息發布失敗 - 主題: {}, 錯誤: {:?}", d.topic, x);
                                                msgs.push(d);
                                            }
                                        }
                                    } else {
                                        info!("⏰ 延遲發送 MQTT 消息 - 主題: {}", d.topic);
                                        msgs.push(d);
                                    }
                                }
                            }
                            Ok(())
                        };
                        if let Err(msg) = handle() {
                            warn!("mqtt {:?}", msg);
                            break;
                        }
                    }
                }
            }
            thread::sleep(Duration::from_millis(100));
        }
        Ok(())
    });
    Ok(rx)
}