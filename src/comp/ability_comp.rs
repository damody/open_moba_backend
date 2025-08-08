use ability_system::{AbilityProcessor, AbilityState, AbilityRequest, AbilityResult};
use specs::{Component, VecStorage, DenseVecStorage, Entity};
use std::collections::HashMap;
use vek::Vec2;

/// 技能組件 - 存儲單位的技能狀態
#[derive(Debug, Clone, Component)]
#[storage(VecStorage)]
pub struct AbilityComponent {
    /// 技能狀態 (技能ID -> 狀態)
    pub abilities: HashMap<String, AbilityState>,
    /// 技能等級 (技能ID -> 等級)
    pub levels: HashMap<String, u8>,
}

impl AbilityComponent {
    pub fn new() -> Self {
        Self {
            abilities: HashMap::new(),
            levels: HashMap::new(),
        }
    }

    /// 添加技能
    pub fn add_ability(&mut self, ability_id: String, level: u8) {
        self.abilities.insert(ability_id.clone(), AbilityState::default());
        self.levels.insert(ability_id, level);
    }

    /// 獲取技能狀態
    pub fn get_ability_state(&self, ability_id: &str) -> Option<&AbilityState> {
        self.abilities.get(ability_id)
    }

    /// 獲取技能狀態（可變）
    pub fn get_ability_state_mut(&mut self, ability_id: &str) -> Option<&mut AbilityState> {
        self.abilities.get_mut(ability_id)
    }

    /// 獲取技能等級
    pub fn get_ability_level(&self, ability_id: &str) -> Option<u8> {
        self.levels.get(ability_id).copied()
    }

    /// 升級技能
    pub fn upgrade_ability(&mut self, ability_id: &str) -> bool {
        if let Some(level) = self.levels.get_mut(ability_id) {
            if *level < 4 {
                *level += 1;
                return true;
            }
        }
        false
    }
}

impl Default for AbilityComponent {
    fn default() -> Self {
        Self::new()
    }
}

/// 技能請求組件 - 存儲待處理的技能請求
#[derive(Debug, Clone, Component)]
#[storage(DenseVecStorage)]
pub struct AbilityRequestComponent {
    pub requests: Vec<AbilityRequest>,
}

impl AbilityRequestComponent {
    pub fn new() -> Self {
        Self {
            requests: Vec::new(),
        }
    }

    /// 添加技能請求
    pub fn add_request(&mut self, request: AbilityRequest) {
        self.requests.push(request);
    }

    /// 清空請求
    pub fn clear(&mut self) {
        self.requests.clear();
    }
}

impl Default for AbilityRequestComponent {
    fn default() -> Self {
        Self::new()
    }
}

/// 技能結果組件 - 存儲技能處理結果
#[derive(Debug, Clone, Component)]
#[storage(DenseVecStorage)]
pub struct AbilityResultComponent {
    pub results: Vec<AbilityResult>,
}

impl AbilityResultComponent {
    pub fn new() -> Self {
        Self {
            results: Vec::new(),
        }
    }

    /// 添加結果
    pub fn add_result(&mut self, result: AbilityResult) {
        self.results.push(result);
    }

    /// 清空結果
    pub fn clear(&mut self) {
        self.results.clear();
    }
}

impl Default for AbilityResultComponent {
    fn default() -> Self {
        Self::new()
    }
}