/// MQTT 測試介面
/// 
/// 提供自動化測試用的 MQTT 查詢介面，支援：
/// - 技能系統狀態查詢
/// - 技能執行測試  
/// - 召喚系統測試
/// - 性能監控
/// - 系統重置

use rumqttc::{Client, Connection, MqttOptions, QoS, Event, Packet};
use serde::{Deserialize, Serialize};
use serde_json;
use log::{info, warn, error, debug};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use ability_system::{AbilityProcessor, AbilityRequest, AbilityState};
use specs::{Entity, world::Generation};
use vek::Vec2;
use crossbeam_channel::{Receiver, Sender, bounded};
use std::thread;
use crate::msg::MqttMsg;

/// MQTT 測試命令
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "command", content = "data")]
pub enum TestCommand {
    /// 查詢系統狀態
    QueryStatus,
    /// 查詢技能列表
    QueryAbilities,
    /// 執行技能測試
    TestAbility {
        ability_id: String,
        level: u8,
        target_position: Option<(f32, f32)>,
        target_entity: Option<u32>,
    },
    /// 測試召喚系統
    TestSummon {
        unit_type: String,
        position: (f32, f32),
        count: u32,
    },
    /// 重置測試環境
    Reset,
    /// 查詢性能統計
    QueryMetrics,
    /// 運行基準測試
    RunBenchmark {
        test_name: String,
        iterations: u32,
    },
}

/// MQTT 測試回應
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TestResponse {
    pub command: String,
    pub success: bool,
    pub data: serde_json::Value,
    pub timestamp: u64,
    pub execution_time_ms: u64,
}

/// 測試統計
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TestMetrics {
    pub total_commands: u64,
    pub successful_commands: u64,
    pub failed_commands: u64,
    pub average_response_time_ms: f64,
    pub ability_executions: HashMap<String, u64>,
    pub summon_counts: HashMap<String, u64>,
}

/// MQTT 測試介面管理器
pub struct MqttTestInterfaceManager {
    mqtt_tx: Sender<MqttMsg>,
    processor: Arc<Mutex<AbilityProcessor>>,
    metrics: Arc<Mutex<TestMetrics>>,
    test_entities: Vec<Entity>,
}

impl MqttTestInterfaceManager {
    /// 創建新的測試介面管理器
    pub fn new(mqtt_tx: Sender<MqttMsg>) -> Self {
        let processor = Arc::new(Mutex::new(AbilityProcessor::new()));
        let metrics = Arc::new(Mutex::new(TestMetrics::new()));
        
        // 創建測試實體
        let mut test_entities = Vec::new();
        for i in 1..=10 {
            test_entities.push(Entity::new(i, Generation::new(1)));
        }
        
        info!("MQTT 測試介面管理器已創建");
        
        Self {
            mqtt_tx,
            processor,
            metrics,
            test_entities,
        }
    }
    
    /// 啟動測試介面（在獨立線程中運行）
    pub fn start(&self, server_addr: String, server_port: String) {
        let mqtt_tx = self.mqtt_tx.clone();
        let processor = self.processor.clone();
        let metrics = self.metrics.clone();
        let test_entities = self.test_entities.clone();
        
        thread::spawn(move || {
            if let Err(e) = Self::run_test_interface(server_addr, server_port, mqtt_tx, processor, metrics, test_entities) {
                error!("MQTT 測試介面運行失敗: {}", e);
            }
        });
    }
    
    /// 運行測試介面主循環
    fn run_test_interface(
        server_addr: String,
        server_port: String,
        mqtt_tx: Sender<MqttMsg>,
        processor: Arc<Mutex<AbilityProcessor>>,
        metrics: Arc<Mutex<TestMetrics>>,
        test_entities: Vec<Entity>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut mqttoptions = MqttOptions::new("ability_test_interface", &server_addr, server_port.parse::<u16>()?);
        mqttoptions.set_keep_alive(Duration::from_secs(30));
        mqttoptions.set_clean_session(true);
        
        let (mut client, mut connection) = Client::new(mqttoptions, 10);
        
        // 訂閱測試命令主題
        client.subscribe("ability_test/command", QoS::AtLeastOnce)?;
        
        info!("MQTT 測試介面已連接到 {}:{}", server_addr, server_port);
        
        // 主事件循環
        for (i, notification) in connection.iter().enumerate() {
            match notification {
                Ok(Event::Incoming(Packet::Publish(publish))) => {
                    if publish.topic == "ability_test/command" {
                        let payload = String::from_utf8_lossy(&publish.payload);
                        debug!("收到測試命令: {}", payload);
                        
                        match serde_json::from_str::<TestCommand>(&payload) {
                            Ok(command) => {
                                let response = Self::handle_command(
                                    command,
                                    &processor,
                                    &metrics,
                                    &test_entities
                                );
                                
                                // 發布回應
                                let response_json = serde_json::to_string(&response)?;
                                if let Err(e) = client.publish("ability_test/response", QoS::AtMostOnce, false, response_json) {
                                    error!("發布測試回應失敗: {}", e);
                                }
                            },
                            Err(e) => {
                                warn!("解析測試命令失敗: {}", e);
                                let error_response = TestResponse {
                                    command: "parse_error".to_string(),
                                    success: false,
                                    data: serde_json::json!({
                                        "error": format!("解析命令失敗: {}", e)
                                    }),
                                    timestamp: Self::current_timestamp(),
                                    execution_time_ms: 0,
                                };
                                
                                let response_json = serde_json::to_string(&error_response)?;
                                let _ = client.publish("ability_test/response", QoS::AtMostOnce, false, response_json);
                            }
                        }
                    }
                },
                Ok(_) => {},
                Err(e) => {
                    error!("MQTT 連接錯誤: {}", e);
                    thread::sleep(Duration::from_secs(1));
                }
            }
        }
        
        Ok(())
    }
    
    /// 處理測試命令
    fn handle_command(
        command: TestCommand,
        processor: &Arc<Mutex<AbilityProcessor>>,
        metrics: &Arc<Mutex<TestMetrics>>,
        test_entities: &[Entity],
    ) -> TestResponse {
        let start_time = Instant::now();
        let command_name = match &command {
            TestCommand::QueryStatus => "query_status",
            TestCommand::QueryAbilities => "query_abilities",
            TestCommand::TestAbility { .. } => "test_ability",
            TestCommand::TestSummon { .. } => "test_summon",
            TestCommand::Reset => "reset",
            TestCommand::QueryMetrics => "query_metrics",
            TestCommand::RunBenchmark { .. } => "run_benchmark",
        };
        
        let result = match command {
            TestCommand::QueryStatus => Self::handle_query_status(),
            TestCommand::QueryAbilities => Self::handle_query_abilities(processor),
            TestCommand::TestAbility { ability_id, level, target_position, target_entity } => {
                Self::handle_test_ability(ability_id, level, target_position, target_entity, processor, test_entities)
            },
            TestCommand::TestSummon { unit_type, position, count } => {
                Self::handle_test_summon(unit_type, position, count)
            },
            TestCommand::Reset => Self::handle_reset(metrics),
            TestCommand::QueryMetrics => Self::handle_query_metrics(metrics),
            TestCommand::RunBenchmark { test_name, iterations } => {
                Self::handle_run_benchmark(test_name, iterations, processor, test_entities)
            },
        };
        
        let execution_time = start_time.elapsed().as_millis() as u64;
        
        // 更新統計
        {
            let mut stats = metrics.lock().unwrap();
            stats.total_commands += 1;
            if result.success {
                stats.successful_commands += 1;
            } else {
                stats.failed_commands += 1;
            }
            
            // 更新平均回應時間
            let total_time = stats.average_response_time_ms * (stats.total_commands - 1) as f64 + execution_time as f64;
            stats.average_response_time_ms = total_time / stats.total_commands as f64;
        }
        
        result
    }
    
    /// 查詢系統狀態
    fn handle_query_status() -> TestResponse {
        let status = serde_json::json!({
            "system": "ability_test_interface",
            "version": "1.0.0",
            "status": "running",
            "features": [
                "ability_testing",
                "summon_testing", 
                "performance_metrics",
                "benchmarking"
            ]
        });
        
        TestResponse {
            command: "query_status".to_string(),
            success: true,
            data: status,
            timestamp: Self::current_timestamp(),
            execution_time_ms: 0,
        }
    }
    
    /// 查詢技能列表
    fn handle_query_abilities(processor: &Arc<Mutex<AbilityProcessor>>) -> TestResponse {
        let processor = processor.lock().unwrap();
        let registry = processor.get_registry();
        
        let mut abilities = Vec::new();
        
        // 已知的技能列表
        let known_abilities = vec![
            "sniper_mode", "saika_reinforcements", "rain_iron_cannon", "three_stage_technique",
            "flame_blade", "fire_dash", "flame_assault", "matchlock_gun"
        ];
        
        for ability_id in known_abilities {
            if let Some(_handler) = registry.get_handler(ability_id) {
                abilities.push(serde_json::json!({
                    "id": ability_id,
                    "description": format!("技能: {}", ability_id),
                    "available": true
                }));
            } else {
                abilities.push(serde_json::json!({
                    "id": ability_id,
                    "description": "未註冊",
                    "available": false
                }));
            }
        }
        
        TestResponse {
            command: "query_abilities".to_string(),
            success: true,
            data: serde_json::json!({
                "abilities": abilities,
                "total_count": abilities.len()
            }),
            timestamp: Self::current_timestamp(),
            execution_time_ms: 0,
        }
    }
    
    /// 測試技能執行
    fn handle_test_ability(
        ability_id: String,
        level: u8,
        target_position: Option<(f32, f32)>,
        target_entity: Option<u32>,
        processor: &Arc<Mutex<AbilityProcessor>>,
        test_entities: &[Entity],
    ) -> TestResponse {
        let caster = test_entities[0]; // 使用第一個測試實體作為施法者
        
        let target_pos = target_position.map(|(x, y)| Vec2::new(x, y));
        let target_ent = target_entity.map(|id| Entity::new(id, Generation::new(1)));
        
        let request = AbilityRequest {
            caster,
            ability_id: ability_id.clone(),
            level,
            target_position: target_pos,
            target_entity: target_ent,
        };
        
        let state = AbilityState::default();
        
        let processor = processor.lock().unwrap();
        let result = processor.process_ability(&request, &state);
        
        TestResponse {
            command: "test_ability".to_string(),
            success: result.success,
            data: serde_json::json!({
                "ability_id": ability_id,
                "level": level,
                "effects_count": result.effects.len(),
                "error": result.error_message
            }),
            timestamp: Self::current_timestamp(),
            execution_time_ms: 0,
        }
    }
    
    /// 測試召喚系統
    fn handle_test_summon(
        unit_type: String,
        position: (f32, f32),
        count: u32,
    ) -> TestResponse {
        // 驗證召喚單位類型
        let valid_types = vec!["saika_gunner", "archer", "swordsman", "mage"];
        
        if !valid_types.contains(&unit_type.as_str()) {
            return TestResponse {
                command: "test_summon".to_string(),
                success: false,
                data: serde_json::json!({
                    "error": format!("未知的召喚單位類型: {}", unit_type),
                    "valid_types": valid_types
                }),
                timestamp: Self::current_timestamp(),
                execution_time_ms: 0,
            };
        }
        
        // 模擬召喚測試
        let mut summoned_units = Vec::new();
        
        for i in 0..count {
            let angle = (i as f32 / count as f32) * std::f32::consts::PI * 2.0;
            let offset_x = angle.cos() * 50.0;
            let offset_y = angle.sin() * 50.0;
            
            summoned_units.push(serde_json::json!({
                "unit_type": unit_type,
                "position": (position.0 + offset_x, position.1 + offset_y),
                "id": format!("test_summon_{}", i)
            }));
        }
        
        TestResponse {
            command: "test_summon".to_string(),
            success: true,
            data: serde_json::json!({
                "unit_type": unit_type,
                "requested_count": count,
                "summoned_count": summoned_units.len(),
                "summoned_units": summoned_units
            }),
            timestamp: Self::current_timestamp(),
            execution_time_ms: 0,
        }
    }
    
    /// 重置測試環境
    fn handle_reset(metrics: &Arc<Mutex<TestMetrics>>) -> TestResponse {
        let mut stats = metrics.lock().unwrap();
        *stats = TestMetrics::new();
        
        TestResponse {
            command: "reset".to_string(),
            success: true,
            data: serde_json::json!({"message": "測試環境已重置"}),
            timestamp: Self::current_timestamp(),
            execution_time_ms: 0,
        }
    }
    
    /// 查詢性能統計
    fn handle_query_metrics(metrics: &Arc<Mutex<TestMetrics>>) -> TestResponse {
        let stats = metrics.lock().unwrap();
        
        TestResponse {
            command: "query_metrics".to_string(),
            success: true,
            data: serde_json::to_value(&*stats).unwrap_or_default(),
            timestamp: Self::current_timestamp(),
            execution_time_ms: 0,
        }
    }
    
    /// 運行基準測試
    fn handle_run_benchmark(
        test_name: String,
        iterations: u32,
        processor: &Arc<Mutex<AbilityProcessor>>,
        test_entities: &[Entity],
    ) -> TestResponse {
        let start_time = Instant::now();
        let caster = test_entities[0];
        
        let mut results = Vec::new();
        
        match test_name.as_str() {
            "ability_execution_speed" => {
                // 測試技能執行速度
                let abilities = vec!["flame_blade", "fire_dash", "saika_reinforcements"];
                
                for ability_id in abilities {
                    let ability_start = Instant::now();
                    
                    for _ in 0..iterations {
                        let request = AbilityRequest {
                            caster,
                            ability_id: ability_id.to_string(),
                            level: 1,
                            target_position: Some(Vec2::new(100.0, 100.0)),
                            target_entity: None,
                        };
                        
                        let state = AbilityState::default();
                        let processor = processor.lock().unwrap();
                        let _ = processor.process_ability(&request, &state);
                    }
                    
                    let duration = ability_start.elapsed();
                    results.push(serde_json::json!({
                        "ability_id": ability_id,
                        "iterations": iterations,
                        "total_time_ms": duration.as_millis(),
                        "avg_time_us": duration.as_micros() / iterations as u128
                    }));
                }
            },
            _ => {
                return TestResponse {
                    command: "run_benchmark".to_string(),
                    success: false,
                    data: serde_json::json!({
                        "error": format!("未知的基準測試: {}", test_name),
                        "available_tests": ["ability_execution_speed"]
                    }),
                    timestamp: Self::current_timestamp(),
                    execution_time_ms: 0,
                };
            }
        }
        
        let total_duration = start_time.elapsed();
        
        TestResponse {
            command: "run_benchmark".to_string(),
            success: true,
            data: serde_json::json!({
                "test_name": test_name,
                "total_duration_ms": total_duration.as_millis(),
                "iterations": iterations,
                "results": results
            }),
            timestamp: Self::current_timestamp(),
            execution_time_ms: 0,
        }
    }
    
    /// 獲取當前時間戳
    fn current_timestamp() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }
}

impl TestMetrics {
    pub fn new() -> Self {
        Self {
            total_commands: 0,
            successful_commands: 0,
            failed_commands: 0,
            average_response_time_ms: 0.0,
            ability_executions: HashMap::new(),
            summon_counts: HashMap::new(),
        }
    }
}