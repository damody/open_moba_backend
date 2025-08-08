/// 創建相關事件處理

use specs::{Entity, World, WorldExt, Builder};
use vek::Vec2;
use crate::comp::*;
use crate::msg::MqttMsg;
use crossbeam_channel::Sender;
use serde_json::json;
use log::{info, warn, error};

/// 創建事件處理器
pub struct CreationEventHandler;

impl CreationEventHandler {
    /// 處理小兵創建事件
    /// 在遊戲世界中創建小兵實體，並發送 MQTT 消息通知前端
    pub fn handle_creep_creation(
        world: &mut World,
        mqtx: &Sender<MqttMsg>,
        cd: CreepData,
    ) -> Vec<Outcome> {
        info!("創建小兵於位置: ({}, {})", cd.pos.x, cd.pos.y);
        
        // 序列化小兵資料為 JSON
        let mut cjs = json!(cd);
        
        // 創建小兵實體
        let entity = world.create_entity()
            .with(Pos(cd.pos))
            .with(cd.creep)
            .with(cd.cdata)
            .build();
        
        // 在 JSON 中添加實體 ID
        if let Some(obj) = cjs.as_object_mut() {
            obj.insert("id".to_owned(), json!(entity.id()));
        }
        
        // 發送 MQTT 消息通知前端
        if let Err(e) = mqtx.try_send(MqttMsg::new_s("td/all/res", "creep", "C", cjs)) {
            error!("發送小兵創建消息失敗: {}", e);
        }
        
        // 小兵創建成功，無需產生額外事件
        Vec::new()
    }

    /// 處理塔創建事件
    /// 在遊戲世界中創建塔實體，更新搜尋索引，並發送 MQTT 消息
    pub fn handle_tower_creation(
        world: &mut World,
        mqtx: &Sender<MqttMsg>,
        pos: Vec2<f32>,
        td: TowerData,
    ) -> Vec<Outcome> {
        info!("創建塔於位置: ({}, {})", pos.x, pos.y);
        
        // 序列化塔資料為 JSON
        let mut cjs = json!(td);
        
        // 創建塔實體
        let entity = world.create_entity()
            .with(Pos(pos))
            .with(Tower::new())
            .with(td.tpty)
            .with(td.tatk)
            .build();
        
        // 在 JSON 中添加實體 ID 和位置
        if let Some(obj) = cjs.as_object_mut() {
            obj.insert("id".to_owned(), json!(entity.id()));
            obj.insert("pos".to_owned(), json!(pos));
        }
        
        // 發送 MQTT 消息通知前端
        if let Err(e) = mqtx.try_send(MqttMsg::new_s("td/all/res", "tower", "C", cjs)) {
            error!("發送塔創建消息失敗: {}", e);
        }
        
        // 標記塔搜尋索引需要重新排序
        if let Some(mut searcher) = world.try_fetch_mut::<Searcher>() {
            searcher.tower.needsort = true;
            info!("標記塔搜尋索引需要重新排序");
        } else {
            warn!("無法獲取 Searcher 資源，跳過索引更新");
        }
        
        // 塔創建成功，無需產生額外事件
        Vec::new()
    }

    /// 處理彈道創建事件
    /// 創建投射物實體，用於視覺效果顯示和傷害處理
    pub fn handle_projectile_creation(
        world: &mut World,
        mqtx: &Sender<MqttMsg>,
        pos: Vec2<f32>,
        source: Entity,
        target: Entity,
        damage_phys: Option<f32>,
        damage_magi: Option<f32>,
        damage_real: Option<f32>,
    ) -> Vec<Outcome> {
        info!("創建彈道從實體 {} 到實體 {} 於位置 ({}, {})", 
              source.id(), target.id(), pos.x, pos.y);
        
        // 獲取來源和目標的位置資訊
        let (source_pos, target_pos) = {
            let positions = world.read_storage::<Pos>();
            
            let source_pos = match positions.get(source) {
                Some(pos) => pos.0,
                None => {
                    warn!("無法找到來源實體 {} 的位置，使用預設位置", source.id());
                    pos
                }
            };
            
            let target_pos = match positions.get(target) {
                Some(pos) => pos.0,
                None => {
                    warn!("無法找到目標實體 {} 的位置，使用預設位置", target.id());
                    pos
                }
            };
            
            (source_pos, target_pos)
        }; // positions 在這裡被釋放
        
        // 從來源實體獲取攻擊屬性來計算傷害值
        let (phys_damage, magi_damage, real_damage) = {
            let attacks = world.read_storage::<TAttack>();
            if let Some(attack) = attacks.get(source) {
                (
                    damage_phys.unwrap_or(attack.atk_physic.v),
                    damage_magi.unwrap_or(0.0),
                    damage_real.unwrap_or(0.0)
                )
            } else {
                // 如果沒有攻擊組件，使用傳入的數值或預設值
                (
                    damage_phys.unwrap_or(25.0),
                    damage_magi.unwrap_or(0.0),
                    damage_real.unwrap_or(0.0)
                )
            }
        };

        // 創建投射物實體（用於視覺效果和傷害處理）
        let projectile_entity = world.create_entity()
            .with(Pos(source_pos))
            .with(Projectile {
                time_left: 2.0,     // 彈道存活時間
                owner: source,      // 擁有者
                target: Some(target), // 目標實體
                tpos: target_pos,   // 目標位置
                radius: 5.0,        // 碰撞半徑
                msd: 500.0,         // 移動速度
                damage_phys: phys_damage, // 物理傷害
                damage_magi: magi_damage, // 魔法傷害
                damage_real: real_damage, // 真實傷害
            })
            .build();
        
        // 發送 MQTT 消息顯示彈道視覺效果
        let projectile_data = json!({
            "id": projectile_entity.id(),
            "source_id": source.id(),
            "target_id": target.id(),
            "start_pos": {
                "x": source_pos.x,
                "y": source_pos.y
            },
            "end_pos": {
                "x": target_pos.x,
                "y": target_pos.y
            }
        });
        
        if let Err(e) = mqtx.try_send(MqttMsg::new_s("td/all/res", "projectile", "C", projectile_data)) {
            error!("發送彈道創建消息失敗: {}", e);
        }
        
        // 彈道創建成功，無需產生額外事件
        Vec::new()
    }

    /// 處理單位生成事件
    /// 根據單位類型和陣營創建相應實體
    pub fn handle_unit_spawn(
        world: &mut World,
        mqtx: &Sender<MqttMsg>,
        pos: Vec2<f32>,
        unit: Unit,
        faction: Faction,
        duration: Option<f32>,
    ) -> Vec<Outcome> {
        info!("生成單位於位置 ({}, {})，陣營: {:?}", pos.x, pos.y, faction);
        
        let faction_clone = faction.clone(); // 克隆供後續使用
        
        let mut entity_builder = world.create_entity()
            .with(Pos(pos))
            .with(unit)
            .with(faction);
        
        // 如果有持續時間，添加臨時單位組件
        if let Some(duration) = duration {
            // 這裡需要一個 TemporaryUnit 組件來處理有時間限制的單位
            info!("單位將在 {:.1} 秒後消失", duration);
            // entity_builder = entity_builder.with(TemporaryUnit { remaining_time: duration });
        }
        
        let entity = entity_builder.build();
        
        // 發送單位創建消息
        let unit_data = json!({
            "id": entity.id(),
            "pos": {
                "x": pos.x,
                "y": pos.y
            },
            "faction": faction_clone,
            "duration": duration
        });
        
        if let Err(e) = mqtx.try_send(MqttMsg::new_s("td/all/res", "unit", "C", unit_data)) {
            error!("發送單位創建消息失敗: {}", e);
        }
        
        Vec::new()
    }
}