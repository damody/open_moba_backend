/// 技能處理器介面
/// 
/// 統一的技能處理介面，每個技能都需要實作此 trait
/// 提供標準化的技能執行、條件檢查和效果生成

use crate::*;
// use serde_json::Value; // 暫時不需要

/// 技能處理器 trait
pub trait AbilityHandler: Send + Sync {
    /// 獲取技能 ID
    fn get_ability_id(&self) -> &str;
    
    /// 執行技能
    /// 
    /// # 參數
    /// - `request`: 技能請求
    /// - `config`: 技能配置 
    /// - `level_data`: 等級數據
    /// 
    /// # 返回
    /// 技能效果列表
    fn execute(
        &self, 
        request: &AbilityRequest, 
        config: &AbilityConfig, 
        level_data: &AbilityLevelData
    ) -> Vec<AbilityEffect>;
    
    /// 檢查技能是否可執行
    /// 
    /// # 參數
    /// - `request`: 技能請求
    /// - `config`: 技能配置
    /// - `state`: 技能狀態
    /// 
    /// # 返回
    /// 是否可執行
    fn can_execute(
        &self, 
        request: &AbilityRequest, 
        config: &AbilityConfig, 
        state: &AbilityState
    ) -> bool {
        // 預設實現：檢查基本條件
        self.check_cooldown(state) && 
        self.check_charges(state) &&
        self.check_mana(request, config, &AbilityLevelData::default()) &&
        self.check_range(request, config, &AbilityLevelData::default()) &&
        self.check_target(request, config)
    }
    
    /// 檢查冷卻時間
    fn check_cooldown(&self, state: &AbilityState) -> bool {
        state.cooldown_remaining <= 0.0
    }
    
    /// 檢查充能
    fn check_charges(&self, state: &AbilityState) -> bool {
        state.charges > 0
    }
    
    /// 檢查法力值
    fn check_mana(&self, _request: &AbilityRequest, _config: &AbilityConfig, _level_data: &AbilityLevelData) -> bool {
        // TODO: 實作法力值檢查邏輯
        // 需要通過 world_access 獲取施法者的法力值
        _level_data.mana_cost >= 0.0 // 暫時返回基本檢查
    }
    
    /// 檢查射程
    fn check_range(&self, request: &AbilityRequest, _config: &AbilityConfig, _level_data: &AbilityLevelData) -> bool {
        // 如果沒有目標位置，視為有效
        if request.target_position.is_none() && request.target_entity.is_none() {
            return true;
        }
        
        // TODO: 實作射程檢查邏輯
        // 需要通過 world_access 獲取位置信息
        _level_data.range >= 0.0 // 暫時返回基本檢查
    }
    
    /// 檢查目標
    fn check_target(&self, request: &AbilityRequest, config: &AbilityConfig) -> bool {
        match config.target_type {
            TargetType::None => true,
            TargetType::Point => request.target_position.is_some(),
            TargetType::Unit => request.target_entity.is_some(),
        }
    }
    
    /// 從 level_data 中獲取自定義數值
    /// 
    /// # 參數
    /// - `level_data`: 等級數據
    /// - `key`: 數值鍵名
    /// 
    /// # 返回
    /// Option<f32> 數值
    fn get_custom_value(&self, level_data: &AbilityLevelData, key: &str) -> Option<f32> {
        level_data.extra.get(key)
            .and_then(|v| v.as_f64())
            .map(|v| v as f32)
    }
    
    /// 從 level_data 中獲取自定義整數值
    fn get_custom_int(&self, level_data: &AbilityLevelData, key: &str) -> Option<u32> {
        level_data.extra.get(key)
            .and_then(|v| v.as_u64())
            .map(|v| v as u32)
    }
    
    /// 獲取技能描述（用於調試和UI）
    fn get_description(&self) -> &str {
        "技能處理器"
    }
}

/// 技能註冊表
/// 
/// 管理所有技能處理器的註冊和查找
pub struct AbilityRegistry {
    handlers: std::collections::HashMap<String, Box<dyn AbilityHandler>>,
}

impl std::fmt::Debug for AbilityRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AbilityRegistry")
            .field("handlers", &format!("{} handlers", self.handlers.len()))
            .finish()
    }
}

impl AbilityRegistry {
    /// 建立新的註冊表
    pub fn new() -> Self {
        Self {
            handlers: std::collections::HashMap::new(),
        }
    }
    
    /// 註冊技能處理器
    /// 
    /// # 參數
    /// - `handler`: 實作 AbilityHandler 的處理器
    pub fn register(&mut self, handler: Box<dyn AbilityHandler>) {
        let ability_id = handler.get_ability_id().to_string();
        self.handlers.insert(ability_id, handler);
    }
    
    /// 獲取技能處理器
    /// 
    /// # 參數
    /// - `ability_id`: 技能 ID
    /// 
    /// # 返回
    /// Option<&dyn AbilityHandler>
    pub fn get_handler(&self, ability_id: &str) -> Option<&dyn AbilityHandler> {
        self.handlers.get(ability_id).map(|h| h.as_ref())
    }
    
    /// 獲取所有已註冊的技能 ID
    pub fn get_all_ability_ids(&self) -> Vec<&String> {
        self.handlers.keys().collect()
    }
    
    /// 獲取已註冊技能數量
    pub fn len(&self) -> usize {
        self.handlers.len()
    }
    
    /// 註冊表是否為空
    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }
}

impl Default for AbilityRegistry {
    fn default() -> Self {
        Self::new()
    }
}