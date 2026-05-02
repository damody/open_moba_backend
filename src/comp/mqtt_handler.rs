use std::collections::BTreeMap;
use std::time::SystemTime;
use crossbeam_channel::{Receiver, Sender};
use serde::{Deserialize, Serialize};
use serde_json::json;
use failure::Error;
use specs::{World, WorldExt};

use crate::comp::*;
use crate::transport::{OutboundMsg, InboundMsg};
use omoba_template_ids::{HERO_SAIKA_MAGOICHI, hero_abilities};

pub struct MqttHandler;

impl MqttHandler {
    pub fn handle_screen_request(ecs: &mut World, mqtx: &Sender<OutboundMsg>, pd: InboundMsg) -> Result<(), Error> {
        log::info!("🔍 [DEBUG] 開始處理畫面狀態請求 - 玩家: {}, 動作: {}, 完整數據: {:?}", pd.name, pd.a, pd.d);
        
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
            log::info!("🔍 [DEBUG] 成功解析請求數據 - 玩家: {}, 請求類型: {}", request_data.player_name, request_data.request_type);
            match pd.a.as_str() {
                "get_screen_area" => {
                    log::info!("🔍 [DEBUG] 開始處理 get_screen_area 請求");
                    let game_data = Self::get_screen_area_data(&pd.d)?;
                    let response_topic = format!("td/{}/screen_response", request_data.player_name);
                    
                    let mqtt_msg = OutboundMsg {
                        topic: response_topic.clone(),
                        msg: game_data.to_string(),
                        time: SystemTime::now(),
                        entity_pos: None,
                        #[cfg(feature = "kcp")]
                        typed: None,
                        #[cfg(any(feature = "grpc", feature = "kcp"))]
                        policy: Some(crate::transport::BroadcastPolicy::PlayerOnly(request_data.player_name.clone())),
                    };
                    
                    log::info!("📤 [DEBUG] 準備發送畫面資料到主題: {} - 消息內容長度: {} - 發送隊列容量: {}", response_topic, mqtt_msg.msg.len(), mqtx.len());
                    
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
                        "hero_type": HERO_SAIKA_MAGOICHI.as_str(),
                        "abilities": hero_abilities(HERO_SAIKA_MAGOICHI)
                            .iter()
                            .map(|aid| json!({
                                "ability_id": aid.as_str(),
                                "cooldown_remaining": 0.0,
                                "is_available": true,
                            }))
                            .collect::<Vec<_>>(),
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
    
    pub fn handle_tower(ecs: &mut World, mqtx: &Sender<OutboundMsg>, pd: InboundMsg) -> Result<(), Error> {
        match pd.a.as_str() {
            "R" => {
                mqtx.try_send(OutboundMsg::new_s("td/all/res", "tower", "R", json!({"msg":"ok"})))?;
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
                    mqtx.try_send(OutboundMsg::new_s("td/all/res", "tower", "C", json!({"msg":"ok"})))?;
                } else {
                    mqtx.try_send(OutboundMsg::new_s("td/all/res", "tower", "C", json!({"msg":"fail"})))?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    
    pub fn handle_player(ecs: &mut World, mqtx: &Sender<OutboundMsg>, pd: InboundMsg) -> Result<(), Error> {
        let mut pmap = ecs.get_mut::<BTreeMap<String, Player>>().unwrap();
        match pd.a.as_str() {
            "C" => {
                let mut p = Player { name: pd.name.clone(), cost: 100., towers: vec![] };
                p.towers.push(TowerData { tpty: TProperty::new(10., 1, 100.), tatk: TAttack::new(3., 0.3, 300., 100.) });
                pmap.insert(pd.name.clone(), p);
                mqtx.try_send(OutboundMsg::new_s("td/all/res", "player", "C", json!({"msg":"ok"})))?;
            }
            _ => {}
        }
        Ok(())
    }
    
    pub fn process_playerdatas(ecs: &mut World, mqtx: &Sender<OutboundMsg>, mqrx: &Receiver<InboundMsg>) -> Result<(), Error> {
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
                    "skill" => {
                        Self::handle_skill(ecs, mqtx, d)?;
                    }
                    _ => {}
                }
            } else {
                log::warn!("json error");
            }
        }
        Ok(())
    }

    /// 玩家施放技能：把 InboundMsg 轉成 `ScriptEvent::SkillCast` 放入 queue，
    /// 下一次 `run_script_dispatch` 會呼叫對應 `AbilityScript::execute`。
    ///
    /// Payload 格式：
    /// ```json
    /// { "t": "skill", "a": "cast", "name": "player1",
    ///   "d": { "ability_id": "sniper_mode",
    ///          "target_entity": 42,          // 可選
    ///          "target_position": [x, y] } } // 可選
    /// ```
    pub fn handle_skill(ecs: &mut World, _mqtx: &Sender<OutboundMsg>, pd: InboundMsg) -> Result<(), Error> {
        use crate::scripting::event::{ScriptEvent, ScriptEventQueue, SkillTarget};
        use specs::Join;

        if pd.a.as_str() != "cast" {
            return Ok(());
        }

        let ability_id = pd
            .d
            .get("ability_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if ability_id.is_empty() {
            log::warn!("[skill/cast] missing ability_id for player '{}'", pd.name);
            return Ok(());
        }

        let target_entity_id = pd
            .d
            .get("target_entity")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32);
        let target_pos = pd.d.get("target_position").and_then(|v| {
            let arr = v.as_array()?;
            if arr.len() != 2 {
                return None;
            }
            let x = arr[0].as_f64()? as f32;
            let y = arr[1].as_f64()? as f32;
            Some((x, y))
        });

        // 玩家對應的 hero entity：match by hero.name == pd.name
        let caster = {
            let entities = ecs.entities();
            let heroes = ecs.read_storage::<Hero>();
            (&entities, &heroes)
                .join()
                .find(|(_, h)| h.name == pd.name)
                .map(|(e, _)| e)
        };
        let Some(caster) = caster else {
            log::warn!("[skill/cast] no hero entity for player '{}'", pd.name);
            return Ok(());
        };

        // target_entity id 轉 specs::Entity（掃 entities 找 matching id）
        let target = match (target_entity_id, target_pos) {
            (Some(id), _) => {
                let entities = ecs.entities();
                let found = entities.join().find(|e| e.id() == id);
                match found {
                    Some(e) => SkillTarget::Entity(e),
                    None => SkillTarget::None,
                }
            }
            (None, Some((x, y))) => SkillTarget::Point(x, y),
            (None, None) => SkillTarget::None,
        };

        let mut queue = ecs.write_resource::<ScriptEventQueue>();
        queue.push(ScriptEvent::SkillCast {
            caster,
            skill_id: ability_id,
            target,
        });
        Ok(())
    }
}