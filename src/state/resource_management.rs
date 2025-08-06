/// 資源管理器 - 負責處理遊戲資源和玩家請求

use specs::{World, Entity, WorldExt};
use crossbeam_channel::{Receiver, Sender};
use failure::Error;

use crate::comp::*;
use crate::msg::MqttMsg;
use crate::msg::PlayerData;
use crate::Outcome;

/// 資源管理器
pub struct ResourceManager {
    /// MQTT 發送通道
    mqtx: Sender<MqttMsg>,
}

impl ResourceManager {
    /// 創建新的資源管理器
    pub fn new(mqtx: Sender<MqttMsg>) -> Self {
        Self { mqtx }
    }

    /// 處理小兵波生成
    pub fn process_creep_waves(&self, _world: &mut World) -> Result<(), Error> {
        // 實現小兵波處理邏輯
        // 暫時為空實現
        Ok(())
    }

    /// 處理遊戲結果事件
    pub fn process_outcomes(&self, world: &mut World) -> Result<(), Error> {
        // 暫時簡化事件處理，避免循環依賴
        // 取得所有待處理的結果
        let outcomes = {
            let mut outcome_vec = world.write_resource::<Vec<Outcome>>();
            if outcome_vec.is_empty() {
                return Ok(());
            }
            std::mem::take(&mut *outcome_vec)
        };

        // 暫時簡化處理邏輯，後續可以整合事件分派器
        log::info!("處理 {} 個遊戲結果事件", outcomes.len());
        
        // 將待處理的結果清空（暫時實現）
        // 實際應該根據結果類型進行相應處理

        Ok(())
    }

    /// 處理玩家資料
    pub fn process_player_data(&self, world: &mut World, mqrx: &Receiver<PlayerData>) -> Result<(), Error> {
        // 處理所有接收到的玩家資料
        while let Ok(player_data) = mqrx.try_recv() {
            match player_data.t.as_str() {
                "tower" => {
                    self.handle_tower_request(world, player_data)?;
                }
                "player" => {
                    self.handle_player_request(world, player_data)?;
                }
                "screen" => {
                    self.handle_screen_request(world, player_data)?;
                }
                _ => {
                    log::warn!("未知的玩家類型: {}", player_data.t);
                }
            }
        }
        Ok(())
    }

    /// 處理塔相關請求
    pub fn handle_tower_request(&self, world: &mut World, pd: PlayerData) -> Result<(), Error> {
        use serde_json::json;
        
        match pd.a.as_str() {
            "create" => {
                self.create_tower(world, &pd)?;
                log::info!("創建塔: 玩家 {}", pd.name);
            }
            "upgrade" => {
                self.upgrade_tower(world, &pd)?;
                log::info!("升級塔: 玩家 {}", pd.name);
            }
            "sell" => {
                self.sell_tower(world, &pd)?;
                log::info!("出售塔: 玩家 {}", pd.name);
            }
            _ => {
                log::warn!("未知的塔操作: {}", pd.a);
            }
        }
        
        // 發送確認消息
        let response = json!({
            "action": pd.a,
            "status": "completed",
            "player": pd.name
        });
        self.mqtx.send(MqttMsg::new_s("td/all/res", "tower", "R", response))?;
        
        Ok(())
    }

    /// 處理玩家相關請求
    pub fn handle_player_request(&self, world: &mut World, pd: PlayerData) -> Result<(), Error> {
        use serde_json::json;
        
        match pd.a.as_str() {
            "move" => {
                self.move_player(world, &pd)?;
                log::info!("移動玩家: {}", pd.name);
            }
            "attack" => {
                self.player_attack(world, &pd)?;
                log::info!("玩家攻擊: {}", pd.name);
            }
            "skill" => {
                self.use_skill(world, &pd)?;
                log::info!("使用技能: 玩家 {}", pd.name);
            }
            _ => {
                log::warn!("未知的玩家操作: {}", pd.a);
            }
        }
        
        // 發送確認消息
        let response = json!({
            "action": pd.a,
            "status": "completed",
            "player": pd.name
        });
        self.mqtx.send(MqttMsg::new_s("td/all/res", "player", "R", response))?;
        
        Ok(())
    }

    /// 處理畫面請求
    pub fn handle_screen_request(&self, world: &mut World, pd: PlayerData) -> Result<(), Error> {
        use serde_json::json;
        
        match pd.a.as_str() {
            "get_area" => {
                let area_data = self.get_screen_area_data(world, &pd)?;
                let response = json!({
                    "action": "get_area",
                    "status": "completed",
                    "player": pd.name,
                    "data": area_data
                });
                self.mqtx.send(MqttMsg::new_s("td/all/res", "screen", "R", response))?;
                log::info!("發送畫面區域資料給玩家 {}", pd.name);
            }
            "update_view" => {
                self.update_player_view(world, &pd)?;
                log::info!("更新玩家 {} 視野", pd.name);
            }
            _ => {
                log::warn!("未知的畫面操作: {}", pd.a);
            }
        }
        
        Ok(())
    }

    // 私有實現方法
    fn create_tower(&self, world: &mut World, pd: &PlayerData) -> Result<(), Error> {
        use vek::Vec2;
        use specs::{Builder, WorldExt};
        
        // 從 PlayerData.d 中解析位置信息
        if let Ok(data) = serde_json::from_value::<serde_json::Map<String, serde_json::Value>>(pd.d.clone()) {
            if let (Some(x_val), Some(y_val)) = (data.get("x"), data.get("y")) {
                if let (Some(x), Some(y)) = (x_val.as_f64(), y_val.as_f64()) {
                    // 創建塔的基本屬性
                    let tower_pos = Pos(Vec2::new(x as f32, y as f32));
                    let tower_vel = Vel(Vec2::new(0.0, 0.0));
                    
                    // 創建塔組件
                    let tower = Tower::new();
                    let tower_property = TProperty::new(100.0, 1, 200.0); // HP, 等級, 建造成本
                    let tower_attack = TAttack::new(50.0, 1.5, 300.0, 800.0); // 攻擊力, 攻速, 射程, 彈速
                    
                    // 創建塔實體
                    let _tower_entity = world.create_entity()
                        .with(tower_pos)
                        .with(tower_vel)
                        .with(tower)
                        .with(tower_property)
                        .with(tower_attack)
                        .build();
                        
                    // 添加到結果中通知其他系統
                    let mut outcomes = world.write_resource::<Vec<Outcome>>();
                    outcomes.push(Outcome::Tower { 
                        pos: Vec2::new(x as f32, y as f32), 
                        td: TowerData { 
                            tpty: tower_property, 
                            tatk: tower_attack 
                        } 
                    });
                }
            }
        }
        
        Ok(())
    }

    fn upgrade_tower(&self, _world: &mut World, _pd: &PlayerData) -> Result<(), Error> {
        // 實現塔升級邏輯
        Ok(())
    }

    fn sell_tower(&self, _world: &mut World, _pd: &PlayerData) -> Result<(), Error> {
        // 實現塔出售邏輯
        Ok(())
    }

    fn move_player(&self, _world: &mut World, _pd: &PlayerData) -> Result<(), Error> {
        // 實現玩家移動邏輯
        Ok(())
    }

    fn player_attack(&self, _world: &mut World, _pd: &PlayerData) -> Result<(), Error> {
        // 實現玩家攻擊邏輯
        Ok(())
    }

    fn use_skill(&self, _world: &mut World, _pd: &PlayerData) -> Result<(), Error> {
        // 實現技能使用邏輯
        Ok(())
    }

    fn get_screen_area_data(&self, _world: &mut World, _pd: &PlayerData) -> Result<serde_json::Value, Error> {
        use serde_json::json;
        
        // 實現畫面區域資料獲取邏輯
        // 暫時返回空資料
        Ok(json!({
            "entities": [],
            "terrain": {},
            "effects": []
        }))
    }

    fn update_player_view(&self, _world: &mut World, _pd: &PlayerData) -> Result<(), Error> {
        // 實現玩家視野更新邏輯
        Ok(())
    }

    /// 獲取資源統計信息
    pub fn get_resource_stats(&self, world: &World) -> ResourceStats {
        let outcomes = world.read_resource::<Vec<Outcome>>();
        
        ResourceStats {
            pending_outcomes: outcomes.len(),
            total_entities: world.entities().join().count(),
            active_systems: 0, // 需要實際統計
        }
    }

    /// 清理過期資源
    pub fn cleanup_expired_resources(&self, _world: &mut World) -> Result<(), Error> {
        // 實現資源清理邏輯
        Ok(())
    }
}

/// 資源統計信息
#[derive(Debug, Clone)]
pub struct ResourceStats {
    /// 待處理結果數量
    pub pending_outcomes: usize,
    /// 總實體數量
    pub total_entities: usize,
    /// 活躍系統數量
    pub active_systems: usize,
}