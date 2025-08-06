use std::collections::BTreeMap;
use std::time::SystemTime;
use crossbeam_channel::{Receiver, Sender};
use serde::{Deserialize, Serialize};
use serde_json::json;
use failure::Error;
use specs::{World, WorldExt};

use crate::comp::*;
use crate::msg::MqttMsg;
use crate::PlayerData;

pub struct MqttHandler;

impl MqttHandler {
    pub fn handle_screen_request(ecs: &mut World, mqtx: &Sender<MqttMsg>, pd: PlayerData) -> Result<(), Error> {
        log::info!("處理畫面狀態請求 - 玩家: {}, 動作: {}", pd.name, pd.a);
        
        #[derive(Deserialize)]
        struct ScreenRequestData {
            player_name: String,
            request_type: String,
            center_x: f32,
            center_y: f32,
            width: f32,
            height: f32,
        }
        
        if let Ok(request_data) = serde_json::from_value::<ScreenRequestData>(pd.d.clone()) {
            match pd.a.as_str() {
                "get_screen_area" => {
                    let game_data = Self::get_screen_area_data(&pd.d)?;
                    let response_topic = format!("td/{}/screen_response", request_data.player_name);
                    
                    let mqtt_msg = MqttMsg {
                        topic: response_topic.clone(),
                        msg: game_data.to_string(),
                        time: SystemTime::now(),
                    };
                    
                    log::info!("📤 準備發送畫面資料到主題: {} - 消息內容: {}", response_topic, mqtt_msg.msg);
                    
                    match mqtx.try_send(mqtt_msg) {
                        Ok(_) => {
                            log::info!("✅ 成功將消息加入發送隊列 - 主題: {}", response_topic);
                        }
                        Err(e) => {
                            log::error!("❌ 發送消息到隊列失敗 - 主題: {}, 錯誤: {:?}", response_topic, e);
                            return Err(failure::err_msg(format!("Failed to send MQTT message: {:?}", e)));
                        }
                    }
                }
                _ => {
                    log::warn!("未知的畫面請求動作: {}", pd.a);
                }
            }
        } else {
            log::error!("無法解析畫面請求資料: {:?}", pd.d);
        }
        
        Ok(())
    }
    
    fn get_screen_area_data(_request_data: &serde_json::Value) -> Result<serde_json::Value, Error> {
        let response_data = json!({
            "t": "screen_response",
            "d": {
                "area": {
                    "min_x": 300.0,
                    "min_y": 200.0,
                    "max_x": 500.0,
                    "max_y": 400.0
                },
                "entities": [
                    {
                        "id": 1,
                        "entity_type": "enemy",
                        "position": [450.0, 350.0],
                        "health": [80.0, 100.0],
                        "name": "training_mage",
                        "owner": "AI"
                    },
                    {
                        "id": 2,
                        "entity_type": "summon",
                        "position": [380.0, 320.0],
                        "health": [50.0, 50.0],
                        "name": "saika_unit",
                        "owner": "TestPlayer"
                    }
                ],
                "players": [
                    {
                        "name": "TestPlayer",
                        "position": [400.0, 300.0],
                        "health": [100.0, 100.0],
                        "hero_type": "saika_magoichi",
                        "abilities": [
                            {"ability_id": "sniper_mode", "cooldown_remaining": 0.0, "is_available": true},
                            {"ability_id": "saika_reinforcements", "cooldown_remaining": 0.0, "is_available": true},
                            {"ability_id": "rain_iron_cannon", "cooldown_remaining": 0.0, "is_available": true},
                            {"ability_id": "three_stage_technique", "cooldown_remaining": 0.0, "is_available": true}
                        ],
                        "items": []
                    }
                ],
                "terrain": [
                    {"position": [300.0, 200.0], "terrain_type": "tree"},
                    {"position": [500.0, 400.0], "terrain_type": "water"},
                    {"position": [350.0, 250.0], "terrain_type": "rock"}
                ],
                "timestamp": std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis()
            }
        });
        
        Ok(response_data)
    }
    
    pub fn handle_tower(ecs: &mut World, mqtx: &Sender<MqttMsg>, pd: PlayerData) -> Result<(), Error> {
        match pd.a.as_str() {
            "R" => {
                mqtx.try_send(MqttMsg::new_s("td/all/res", "tower", "R", json!({"msg":"ok"})))?;
            }
            "C" => {
                #[derive(Serialize, Deserialize)]
                struct JData {
                    tid: i32,
                    x: f32,
                    y: f32,
                };
                let v: JData = serde_json::from_value(pd.d)?;
                let t = {
                    let mut pmap = ecs.get_mut::<BTreeMap<String, Player>>().unwrap();
                    if let Some(p) = pmap.get_mut(&pd.name) {
                        if let Some(t) = p.towers.get(v.tid as usize) {
                            Some(t.clone())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };
                let mut ocs = ecs.get_mut::<Vec<Outcome>>().unwrap();
                if let Some(t) = t {
                    ocs.push(Outcome::Tower { pos: vek::Vec2::new(v.x,v.y), td: TowerData { tpty: t.tpty, tatk: t.tatk } });
                    mqtx.try_send(MqttMsg::new_s("td/all/res", "tower", "C", json!({"msg":"ok"})))?;
                } else {
                    mqtx.try_send(MqttMsg::new_s("td/all/res", "tower", "C", json!({"msg":"fail"})))?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    
    pub fn handle_player(ecs: &mut World, mqtx: &Sender<MqttMsg>, pd: PlayerData) -> Result<(), Error> {
        let mut pmap = ecs.get_mut::<BTreeMap<String, Player>>().unwrap();
        match pd.a.as_str() {
            "C" => {
                let mut p = Player { name: pd.name.clone(), cost: 100., towers: vec![] };
                p.towers.push(TowerData { tpty: TProperty::new(10., 1, 100.), tatk: TAttack::new(3., 0.3, 300., 100.) });
                pmap.insert(pd.name.clone(), p);
                mqtx.try_send(MqttMsg::new_s("td/all/res", "player", "C", json!({"msg":"ok"})))?;
            }
            _ => {}
        }
        Ok(())
    }
    
    pub fn process_playerdatas(ecs: &mut World, mqtx: &Sender<MqttMsg>, mqrx: &Receiver<PlayerData>) -> Result<(), Error> {
        let n = mqrx.len();
        for _i in 0..n {
            let data = mqrx.try_recv();
            if let Ok(d) = data {
                log::info!("收到 PlayerData: t='{}', a='{}', name='{}'", d.t, d.a, d.name);
                log::debug!("完整 PlayerData: {:?}", d);
                match d.t.as_str() {
                    "tower" => {
                        Self::handle_tower(ecs, mqtx, d)?;
                    }
                    "player" => {
                        Self::handle_player(ecs, mqtx, d)?;
                    }
                    "screen_request" => {
                        Self::handle_screen_request(ecs, mqtx, d)?;
                    }
                    _ => {}
                }
            } else {
                log::warn!("json error");
            }
        }
        Ok(())
    }
}