use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use crate::json_preprocessor::JsonPreprocessor;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ItemBonus {
    #[serde(default)] pub atk: f32,
    #[serde(default)] pub hp: f32,
    #[serde(default)] pub mp: f32,
    #[serde(default)] pub ms: f32,
    #[serde(default)] pub armor: f32,
    #[serde(default)] pub mp_regen: f32,
}

/// 主動效果類型（MVP 簡化版）
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActiveEffect {
    /// 下次普攻附加額外傷害
    HeadshotNext { bonus_damage: f32 },
    /// 立即為自身施加護盾（以血量吸收形式 - 暫以瞬回 HP 代替）
    Shield { amount: f32, duration: f32 },
    /// 立即回復魔力
    RestoreMana { amount: f32 },
    /// 短時間增加移速
    SprintBuff { ms_bonus: f32, duration: f32 },
    /// 短時間傷害減免
    DamageReduce { percent: f32, duration: f32 },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ItemConfig {
    pub id: String,
    pub name: String,
    pub cost: i32,
    #[serde(default)] pub bonus: ItemBonus,
    #[serde(default)] pub active: Option<ActiveEffect>,
    #[serde(default)] pub cooldown: f32,
    /// 升級所需組件 item_id（可為空）。購買時會從背包消耗這些組件
    #[serde(default)] pub recipe: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct ItemRegistry {
    pub items: HashMap<String, Arc<ItemConfig>>,
}

impl ItemRegistry {
    pub fn load_from_path(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let raw = std::fs::read_to_string(path)?;
        let cleaned = JsonPreprocessor::remove_comments(&raw);
        let list: Vec<ItemConfig> = serde_json::from_str(&cleaned)?;
        let mut items = HashMap::new();
        for cfg in list {
            items.insert(cfg.id.clone(), Arc::new(cfg));
        }
        log::info!("已載入 {} 件裝備", items.len());
        Ok(ItemRegistry { items })
    }

    pub fn get(&self, id: &str) -> Option<Arc<ItemConfig>> {
        self.items.get(id).cloned()
    }
}

/// 回收售價（50%）
pub fn sell_price(cost: i32) -> i32 {
    (cost as f32 * 0.5) as i32
}
