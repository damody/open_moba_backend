use crossbeam_channel::{bounded, select, tick, Receiver, Sender};
use failure::Error;
use log::*;
use regex::Regex;
use rumqttc::{Client, Connection, MqttOptions, QoS};
use std::thread;
use std::time::{Duration, SystemTime};
use uuid::Uuid;

use super::types::{InboundMsg, OutboundMsg, TransportHandle};

fn generate_client_id() -> String {
    let s = format!("td_{}", Uuid::new_v4().as_simple());
    (&s[..3]).to_string()
}

fn create_mqtt_client(
    server_addr: &str,
    server_port: &str,
    client_id: String,
    sub: bool,
) -> Result<(Client, Connection), Error> {
    let mut mqtt_options = MqttOptions::new(
        client_id.as_str(),
        server_addr,
        server_port.parse::<u16>()?,
    );
    mqtt_options.set_keep_alive(Duration::from_secs(10));
    mqtt_options.set_request_channel_capacity(10000);
    mqtt_options.set_clean_session(true);
    let (mut mqtt_cli, connection) = Client::new(mqtt_options, 100000);
    if sub {
        mqtt_cli.subscribe("td/+/send", QoS::AtMostOnce).unwrap();
        info!("Backend subscribed to MQTT topic: td/+/send");
    }
    Ok((mqtt_cli, connection))
}

/// Start the MQTT transport layer.
///
/// Returns a `TransportHandle` whose `tx` feeds outbound messages to MQTT
/// and whose `rx` yields inbound player data from MQTT subscriptions.
pub fn start(
    server_addr: String,
    server_port: String,
    client_id: String,
) -> Result<TransportHandle, Error> {
    let (out_tx, out_rx): (Sender<OutboundMsg>, Receiver<OutboundMsg>) = bounded(10000);
    let (in_tx, in_rx): (Sender<InboundMsg>, Receiver<InboundMsg>) = bounded(10000);

    thread::spawn(move || -> Result<(), Error> {
        let update = tick(Duration::from_millis(100));
        let mut msgs: Vec<OutboundMsg> = vec![];
        loop {
            let in_tx = in_tx.clone();
            let (mut mqtt2, mut connection) =
                create_mqtt_client(&server_addr, &server_port, generate_client_id(), true)?;
            let (btx, brx): (Sender<bool>, Receiver<bool>) = bounded(10);
            thread::spawn(move || {
                let _rex_td = Regex::new(r"td/([\w\S]+)/send").unwrap();
                for (_i, notification) in connection.iter().enumerate() {
                    thread::sleep(Duration::from_millis(1));
                    if let Ok(x) = notification {
                        debug!("MQTT Event received: {:?}", x);
                        if let rumqttc::Event::Incoming(x) = x {
                            if let rumqttc::v4::Packet::Publish(x) = x {
                                let handle = || -> Result<(), Error> {
                                    let payload = x.payload;
                                    let msg = match std::str::from_utf8(&payload[..]) {
                                        Ok(msg) => msg,
                                        Err(err) => {
                                            return Err(failure::err_msg(format!(
                                                "Failed to decode publish message {:?}",
                                                err
                                            )));
                                        }
                                    };
                                    let topic_name = x.topic.as_str();
                                    info!(
                                        "Backend received MQTT message - Topic: {}, Payload: {}",
                                        topic_name, msg
                                    );

                                    // Parse as InboundMsg (same JSON shape as old PlayerData)
                                    let vo: serde_json::Result<InboundMsg> =
                                        serde_json::from_str(msg);
                                    if let Ok(v) = vo {
                                        info!(
                                            "Parsed InboundMsg - name: {}, t: {}, a: {}",
                                            v.name, v.t, v.a
                                        );
                                        in_tx.try_send(v).ok();
                                    } else {
                                        warn!(
                                            "Json Parser error for topic: {} payload: {}",
                                            topic_name, msg
                                        );
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
                    recv(update) -> _d => {
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
                    recv(brx) -> _d => {
                        break;
                    },
                    recv(out_rx) -> d => {
                        let handle = || -> Result<(), Error>
                        {
                            if let Ok(d) = d {
                                debug!("[DEBUG] Received outbound msg - topic: {} - len: {}", d.topic, d.msg.len());
                                let diff = d.time.duration_since(SystemTime::now());
                                let mut difftime = 0;
                                match diff {
                                    Ok(n) => { difftime = n.as_micros(); },
                                    Err(_) => {},
                                }
                                if d.topic.len() > 2 {
                                    if difftime == 0 {
                                        trace!("Publishing MQTT message to topic: {} - len: {}", d.topic, d.msg.len());
                                        let msg_res = mqtt2.publish(d.topic.clone(), QoS::AtMostOnce, false, d.msg.clone());
                                        match msg_res {
                                            Ok(_) => {
                                                trace!("MQTT publish success - topic: {}", d.topic);
                                            },
                                            Err(x) => {
                                                warn!("MQTT publish failed - topic: {}, error: {:?}", d.topic, x);
                                                msgs.push(d);
                                            }
                                        }
                                    } else {
                                        trace!("Delayed MQTT message - topic: {}", d.topic);
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
    });

    Ok(TransportHandle {
        tx: out_tx,
        rx: in_rx,
    })
}
