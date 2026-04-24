//! `BuffStore` — host 端統一的 buff 儲存與倒數系統。
//!
//! 取代原本散在多處的 buff 實作（`SlowBuff` component + `slow_buff_tick`）。
//! 所有來自 DLL 腳本的 `world.add_buff` / `world.add_stat_buff` 最終都寫
//! 到這個 resource；每 tick 由 `tick::buff_tick` 倒數，過期自動移除。
//!
//! 每筆 buff 可攜帶 `payload: serde_json::Value`，讓 host 系統（例如
//! `creep_tick` 的移速計算）從 buff 身上讀出數值（如 slow factor）。

use omb_script_abi::buff_ids::BuffId;
use omb_script_abi::stat_keys::StatKey;
use serde_json::Value;
use specs::Entity;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct BuffEntry {
    pub remaining: f32,
    pub payload: Value,
}

/// 以 `(Entity, buff_id)` 為 key 的 O(1) buff 索引。
#[derive(Default, Debug)]
pub struct BuffStore {
    buffs: HashMap<(Entity, String), BuffEntry>,
}

impl BuffStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 新增或刷新 buff。若已存在：duration 取 max、payload 覆蓋（交由呼叫端
    /// 決定是否合併/疊加——例如 slow 用 buff_id = `slow_{attacker}` 每個來源
    /// 一個 entry，由 `sum_add("move_speed_bonus")` 加總聚合）。
    pub fn add(&mut self, entity: Entity, buff_id: &str, duration: f32, payload: Value) {
        let key = (entity, buff_id.to_string());
        match self.buffs.get_mut(&key) {
            Some(e) => {
                if duration > e.remaining {
                    e.remaining = duration;
                }
                e.payload = payload;
            }
            None => {
                self.buffs.insert(
                    key,
                    BuffEntry {
                        remaining: duration,
                        payload,
                    },
                );
            }
        }
    }

    pub fn remove(&mut self, entity: Entity, buff_id: &str) {
        self.buffs.remove(&(entity, buff_id.to_string()));
    }

    pub fn has(&self, entity: Entity, buff_id: &str) -> bool {
        self.buffs.contains_key(&(entity, buff_id.to_string()))
    }

    pub fn get(&self, entity: Entity, buff_id: &str) -> Option<&BuffEntry> {
        self.buffs.get(&(entity, buff_id.to_string()))
    }

    /// 清除 entity 的所有 buff（單位死亡時呼叫）。
    pub fn remove_all_for(&mut self, entity: Entity) {
        self.buffs.retain(|(e, _), _| *e != entity);
    }

    /// 迭代某單位身上所有 buff（供 creep_tick 算移速乘數等）。
    pub fn iter_for(&self, entity: Entity) -> impl Iterator<Item = (&str, &BuffEntry)> {
        self.buffs.iter().filter_map(move |((e, id), v)| {
            if *e == entity {
                Some((id.as_str(), v))
            } else {
                None
            }
        })
    }

    /// 加法聚合：對 entity 身上所有 buff，若 `payload[stat]` 是數字則加總。
    /// 慣例：`_bonus` 後綴的 stat 用這個（例 `range_bonus`、`damage_bonus`）。
    pub fn sum_add(&self, entity: Entity, stat: StatKey) -> f32 {
        let key = stat.as_str();
        self.iter_for(entity)
            .filter_map(|(_, e)| e.payload.get(key).and_then(|v| v.as_f64()))
            .sum::<f64>() as f32
    }

    /// 乘法聚合：對 entity 身上所有 buff，若 `payload[stat]` 是數字則連乘。
    /// 空集合回 1.0。慣例：`_multiplier` 後綴的 stat 用這個
    /// （例 `attack_speed_multiplier`、`move_speed_multiplier`）。
    pub fn product_mult(&self, entity: Entity, stat: StatKey) -> f32 {
        let key = stat.as_str();
        self.iter_for(entity)
            .filter_map(|(_, e)| e.payload.get(key).and_then(|v| v.as_f64()))
            .fold(1.0f64, |acc, v| acc * v) as f32
    }

    /// 控制類 buff 判定 — 這些 buff_id 出現在單位身上代表其處於特定 CC 狀態。
    /// 約定：`stun` 同時禁攻擊與移動；`silence` 禁技能施放；`root` 只禁移動。
    pub fn is_stunned(&self, entity: Entity) -> bool {
        self.has(entity, BuffId::Stun.as_str())
    }

    pub fn is_rooted(&self, entity: Entity) -> bool {
        self.has(entity, BuffId::Root.as_str()) || self.has(entity, BuffId::Stun.as_str())
    }

    pub fn is_silenced(&self, entity: Entity) -> bool {
        self.has(entity, BuffId::Silence.as_str()) || self.has(entity, BuffId::Stun.as_str())
    }

    /// 倒數所有 buff 並回傳過期的 `(Entity, buff_id, payload)` 清單。
    /// 呼叫端可依 payload 內容決定是否廣播（例：payload 含 move_speed_bonus
    /// 表示這是移速影響類 buff，要發 creep/S 還原訊息）。
    pub fn tick(&mut self, dt: f32) -> Vec<(Entity, String, Value)> {
        let mut expired = Vec::new();
        self.buffs.retain(|(e, id), v| {
            v.remaining -= dt;
            if v.remaining <= 0.0 {
                expired.push((*e, id.clone(), v.payload.clone()));
                false
            } else {
                true
            }
        });
        expired
    }

    pub fn len(&self) -> usize {
        self.buffs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buffs.is_empty()
    }
}
