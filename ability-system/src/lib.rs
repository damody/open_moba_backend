use serde::{Deserialize, Serialize};
use specs::Entity;
use std::collections::HashMap;
use vek::Vec2;

pub mod handler;
pub mod heroes;

pub use handler::{AbilityHandler, AbilityRegistry};

/// 技能類型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AbilityType {
    Active,    // 主動技能
    Toggle,    // 切換技能
    Ultimate,  // 大招
}

/// 目標類型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TargetType {
    None,  // 無目標
    Point, // 地面目標
    Unit,  // 單位目標
}

/// 施法類型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CastType {
    Instant,   // 瞬發
    Channeled, // 引導
}

/// 技能等級數據
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbilityLevelData {
    pub cooldown: f32,
    pub mana_cost: f32,
    #[serde(default)]
    pub cast_time: f32,
    #[serde(default)]
    pub range: f32,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl Default for AbilityLevelData {
    fn default() -> Self {
        Self {
            cooldown: 0.0,
            mana_cost: 0.0,
            cast_time: 0.0,
            range: 0.0,
            extra: HashMap::new(),
        }
    }
}

/// 技能配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbilityConfig {
    pub name: String,
    pub description: String,
    pub ability_type: AbilityType,
    pub target_type: TargetType,
    pub cast_type: CastType,
    pub levels: HashMap<String, AbilityLevelData>,
    #[serde(default)]
    pub properties: HashMap<String, serde_json::Value>,
}

/// 技能請求
#[derive(Debug, Clone)]
pub struct AbilityRequest {
    pub caster: Entity,
    pub ability_id: String,
    pub level: u8,
    pub target_position: Option<Vec2<f32>>,
    pub target_entity: Option<Entity>,
}

/// 技能效果
#[derive(Debug, Clone)]
pub enum AbilityEffect {
    // 傷害效果
    Damage {
        target: Entity,
        amount: f32,
    },
    // 治療效果
    Heal {
        target: Entity,
        amount: f32,
    },
    // 狀態修改
    StatusModifier {
        target: Entity,
        modifier_type: String,
        value: f32,
        duration: Option<f32>,
    },
    // 召喚效果
    Summon {
        position: Vec2<f32>,
        unit_type: String,
        count: u32,
        duration: Option<f32>,
    },
    // 區域效果
    AreaEffect {
        center: Vec2<f32>,
        radius: f32,
        effect_type: String,
        damage: Option<f32>,
        duration: f32,
    },
}

/// 技能結果
#[derive(Debug, Clone)]
pub struct AbilityResult {
    pub success: bool,
    pub effects: Vec<AbilityEffect>,
    pub error_message: Option<String>,
}

/// 技能狀態
#[derive(Debug, Clone)]
pub struct AbilityState {
    pub cooldown_remaining: f32,
    pub charges: u32,
    pub max_charges: u32,
    pub is_toggled: bool,
    pub is_casting: bool,
    pub cast_time_remaining: f32,
}

impl Default for AbilityState {
    fn default() -> Self {
        Self {
            cooldown_remaining: 0.0,
            charges: 1,
            max_charges: 1,
            is_toggled: false,
            is_casting: false,
            cast_time_remaining: 0.0,
        }
    }
}

/// 技能處理器 - 純邏輯處理，不依賴 ECS
/// 
/// 性能優化特性：
/// - 使用 HashMap 進行 O(1) 技能查找
/// - 預分配效果向量容量
/// - 避免不必要的字符串分配
#[derive(Debug)]
pub struct AbilityProcessor {
    configs: HashMap<String, AbilityConfig>,
    registry: AbilityRegistry,
    // 性能計數器
    #[cfg(feature = "metrics")]
    execution_count: std::sync::atomic::AtomicU64,
}

impl AbilityProcessor {
    pub fn new() -> Self {
        let mut registry = AbilityRegistry::new();
        
        // 註冊雜賀孫市的技能
        registry.register(Box::new(heroes::B01_saika_magoichi::SniperModeHandler::new()));
        registry.register(Box::new(heroes::B01_saika_magoichi::SaikaReinforcementsHandler::new()));
        registry.register(Box::new(heroes::B01_saika_magoichi::RainIronCannonHandler::new()));
        registry.register(Box::new(heroes::B01_saika_magoichi::ThreeStageHandler::new()));
        
        // 註冊伊達政宗的技能
        registry.register(Box::new(heroes::B02_date_masamune::FlameBladeHandler::new()));
        registry.register(Box::new(heroes::B02_date_masamune::FireDashHandler::new()));
        registry.register(Box::new(heroes::B02_date_masamune::FlameAssaultHandler::new()));
        registry.register(Box::new(heroes::B02_date_masamune::MatchlockGunHandler::new()));
        
        Self {
            configs: HashMap::with_capacity(32), // 預分配容量
            registry,
            #[cfg(feature = "metrics")]
            execution_count: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// 註冊技能處理器
    pub fn register_handler(&mut self, handler: Box<dyn AbilityHandler>) {
        self.registry.register(handler);
    }

    /// 獲取已註冊的技能處理器
    pub fn get_handler(&self, ability_id: &str) -> Option<&dyn AbilityHandler> {
        self.registry.get_handler(ability_id)
    }

    /// 獲取註冊表
    pub fn get_registry(&self) -> &AbilityRegistry {
        &self.registry
    }

    /// 從JSON字符串載入技能配置
    pub fn load_from_json(&mut self, json_content: &str) -> Result<(), Box<dyn std::error::Error>> {
        let configs: HashMap<String, AbilityConfig> = serde_json::from_str(json_content)?;
        self.configs.extend(configs);
        Ok(())
    }

    /// 獲取技能配置
    pub fn get_config(&self, ability_id: &str) -> Option<&AbilityConfig> {
        self.configs.get(ability_id)
    }

    /// 獲取技能等級數據
    pub fn get_level_data(&self, ability_id: &str, level: u8) -> Option<&AbilityLevelData> {
        let config = self.get_config(ability_id)?;
        config.levels.get(&level.to_string())
    }

    /// 處理技能請求 - 返回技能效果，由ECS系統應用
    pub fn process_ability(&self, request: &AbilityRequest, current_state: &AbilityState) -> AbilityResult {
        let config = match self.get_config(&request.ability_id) {
            Some(c) => c,
            None => return AbilityResult {
                success: false,
                effects: vec![],
                error_message: Some(format!("技能 {} 不存在", request.ability_id)),
            },
        };

        let level_data = match self.get_level_data(&request.ability_id, request.level) {
            Some(d) => d,
            None => return AbilityResult {
                success: false,
                effects: vec![],
                error_message: Some(format!("技能 {} 等級 {} 不存在", request.ability_id, request.level)),
            },
        };

        // 檢查冷卻時間
        if current_state.cooldown_remaining > 0.0 {
            return AbilityResult {
                success: false,
                effects: vec![],
                error_message: Some("技能冷卻中".to_string()),
            };
        }

        // 檢查充能
        if current_state.charges == 0 {
            return AbilityResult {
                success: false,
                effects: vec![],
                error_message: Some("技能充能不足".to_string()),
            };
        }

        // 根據技能ID生成對應效果
        let effects = self.generate_effects(request, config, level_data);

        AbilityResult {
            success: true,
            effects,
            error_message: None,
        }
    }

    /// 更新技能狀態（時間更新）
    pub fn update_state(&self, ability_id: &str, state: &mut AbilityState, dt: f32) {
        // 更新冷卻時間
        if state.cooldown_remaining > 0.0 {
            state.cooldown_remaining = (state.cooldown_remaining - dt).max(0.0);
        }

        // 更新施法時間
        if state.is_casting && state.cast_time_remaining > 0.0 {
            state.cast_time_remaining = (state.cast_time_remaining - dt).max(0.0);
            if state.cast_time_remaining <= 0.0 {
                state.is_casting = false;
            }
        }

        // 恢復充能（如果有配置）
        if let Some(config) = self.get_config(ability_id) {
            if let Some(charge_time) = config.properties.get("charge_restore_time") {
                if let Some(_charge_time) = charge_time.as_f64() {
                    // 這裡可以實現充能恢復邏輯
                    // 為簡化，暫時跳過
                }
            }
        }
    }

    /// 生成技能效果 - 使用新的模組化處理器
    fn generate_effects(&self, request: &AbilityRequest, config: &AbilityConfig, level_data: &AbilityLevelData) -> Vec<AbilityEffect> {
        #[cfg(feature = "metrics")]
        {
            self.execution_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        
        // 檢查是否有註冊的處理器
        if let Some(handler) = self.registry.get_handler(&request.ability_id) {
            let mut effects = handler.execute(request, config, level_data);
            
            // 性能優化：預分配容量並避免重新分配
            if effects.capacity() == 0 {
                effects.reserve(4); // 大部分技能有 1-4 個效果
            }
            
            effects
        } else {
            // 回退到舊系統（向後相容）
            self.generate_effects_legacy(request, config, level_data)
        }
    }

    /// 舊的技能效果生成系統（向後相容）
    fn generate_effects_legacy(&self, request: &AbilityRequest, _config: &AbilityConfig, level_data: &AbilityLevelData) -> Vec<AbilityEffect> {
        // 性能優化：預分配容量
        let mut effects = Vec::with_capacity(4);

        match request.ability_id.as_str() {
            "sniper_mode" => {
                // 狙擊模式：切換技能，返回狀態修改效果
                if let Some(range_bonus) = level_data.extra.get("range_bonus").and_then(|v| v.as_f64()) {
                    effects.push(AbilityEffect::StatusModifier {
                        target: request.caster,
                        modifier_type: "range_bonus".to_string(),
                        value: range_bonus as f32,
                        duration: None, // 切換技能持續到下次切換
                    });
                }
                if let Some(damage_bonus) = level_data.extra.get("damage_bonus").and_then(|v| v.as_f64()) {
                    effects.push(AbilityEffect::StatusModifier {
                        target: request.caster,
                        modifier_type: "damage_bonus".to_string(),
                        value: damage_bonus as f32,
                        duration: None,
                    });
                }
            },
            "saika_reinforcements" => {
                // 雜賀眾：召喚技能
                if let Some(position) = request.target_position {
                    if let Some(summon_count) = level_data.extra.get("summon_count").and_then(|v| v.as_u64()) {
                        if let Some(duration) = level_data.extra.get("duration").and_then(|v| v.as_f64()) {
                            effects.push(AbilityEffect::Summon {
                                position,
                                unit_type: "saika_gunner".to_string(),
                                count: summon_count as u32,
                                duration: Some(duration as f32),
                            });
                        }
                    }
                }
            },
            "rain_iron_cannon" => {
                // 雨鐵炮：區域傷害技能
                if let Some(position) = request.target_position {
                    if let Some(damage) = level_data.extra.get("damage").and_then(|v| v.as_f64()) {
                        if let Some(radius) = level_data.extra.get("radius").and_then(|v| v.as_f64()) {
                            if let Some(duration) = level_data.extra.get("duration").and_then(|v| v.as_f64()) {
                                effects.push(AbilityEffect::AreaEffect {
                                    center: position,
                                    radius: radius as f32,
                                    effect_type: "iron_rain".to_string(),
                                    damage: Some(damage as f32),
                                    duration: duration as f32,
                                });
                            }
                        }
                    }
                }
            },
            "three_stage_technique" => {
                // 三段擊：對目標造成多次傷害
                if let Some(target) = request.target_entity {
                    if let Some(damage_per_attack) = level_data.extra.get("damage_per_attack").and_then(|v| v.as_f64()) {
                        if let Some(attacks_count) = level_data.extra.get("attacks_count").and_then(|v| v.as_u64()) {
                            for _ in 0..attacks_count {
                                effects.push(AbilityEffect::Damage {
                                    target,
                                    amount: damage_per_attack as f32,
                                });
                            }
                        }
                    }
                }
            },
            _ => {
                // 默認處理或未知技能
            }
        }

        effects
    }
}

impl Default for AbilityProcessor {
    fn default() -> Self {
        Self::new()
    }
}