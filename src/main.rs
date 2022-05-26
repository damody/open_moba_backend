#![allow(warnings)]
#![allow(unused)]

use log::{info, warn, error, trace, debug};
use std::fs::File;
use std::io::{Write, BufReader, BufRead};
use failure::{err_msg, Error};
use chrono::{NaiveDateTime, Local};
mod comp;
mod sync;
mod tick;
mod ue4;
mod msg;
use crate::msg::MqttMsg;
pub mod config;
use crate::config::server_config::CONFIG;

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
    Component, DispatcherBuilder, Entity as EcsEntity, WorldExt,
};
use serde_json::{self, json};
use crate::ue4::import_map::CreepWaveData;
use rumqttc::{Client, Connection, MqttOptions, QoS};
use crossbeam_channel::{bounded, select, tick, Receiver, Sender};

const TPS: u64 = 10;

fn create_mqtt_client(server_addr: String, server_port: String, client_id: String, sub: bool) -> Result<(Client, Connection), Error> {
    let mut mqtt_options = MqttOptions::new(client_id.as_str(), server_addr.as_str(), server_port.parse::<u16>()?);
    mqtt_options.set_keep_alive(Duration::from_secs(10));
    mqtt_options.set_request_channel_capacity(10000);
    mqtt_options.set_clean_session(true);
    let (mut mqtt_cli, mut connection) = Client::new(mqtt_options.clone(), 100);
    if sub {
        mqtt_cli.subscribe("td/+/send", QoS::AtMostOnce).unwrap();
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
    let map_json = fs::read_to_string("map.json")
        .expect("Something went wrong reading the file");
    // 初始化mqtt
    let server_addr = CONFIG.SERVER_IP.clone();
    let server_port = CONFIG.SERVER_PORT.clone();
    let client_id = CONFIG.CLIENT_ID.clone();
    let mqtt_url = "tcp://".to_owned() + &server_addr + ":" + &server_port;
    let (mqtx, rx): (Sender<MqttMsg>, Receiver<MqttMsg>) = bounded(10000);
    pub_mqtt_loop(server_addr.clone(), server_port.clone(), rx.clone(), client_id.clone());
    // 初始化怪物波次
    let creep_wave: CreepWaveData = serde_json::from_str(&map_json)?;
    // 初始化 ECS
    let mut state = State::new(creep_wave, mqtx);
    let mut clock = Clock::new(Duration::from_secs_f64(1.0 / TPS as f64));
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

fn pub_mqtt_loop(server_addr: String, server_port: String, rx1: Receiver<MqttMsg>, client_id: String) {
    thread::spawn(move || -> Result<(), Error> {
        let update = tick(Duration::from_millis(100));
        let mut msgs: Vec<MqttMsg> = vec![];
        loop {
            let (mut mqtt2, mut connection) = create_mqtt_client(server_addr.clone(), server_port.clone(), client_id.clone()+"_pub", false)?;
            let (btx, brx): (Sender<bool>, Receiver<bool>) = bounded(10);
            thread::spawn(move || {
                for (i, notification) in connection.iter().enumerate() {
                    thread::sleep(Duration::from_millis(1));
                    if let Ok(x) = notification {
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
                                        let msg_res = mqtt2.publish(d.topic.clone(), QoS::AtMostOnce, false, d.msg.clone());
                                        match msg_res {
                                            Ok(_) =>{},
                                            Err(x) => {
                                                warn!("??? {}", x);
                                            }
                                        }
                                    } else {
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
}