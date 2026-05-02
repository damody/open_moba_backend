//! `BuffStore` — host 端統一的 buff 儲存與倒數系統。
//!
//! 取代原本散在多處的 buff 實作（`SlowBuff` component + `slow_buff_tick`）。
//! 所有來自 DLL 腳本的 `world.add_buff` / `world.add_stat_buff` 最終都寫
//! 到這個 resource；每 tick 由 `tick::buff_tick` 倒數，過期自動移除。
//!
//! 每筆 buff 可攜帶 `payload: serde_json::Value`，讓 host 系統（例如
//! `creep_tick` 的移速計算）從 buff 身上讀出數值（如 slow factor）。
//!
//! ## Reserved payload conventions
//!
//! - `slow_factor: f64` — 觸發 add() 的「強蓋弱」語意：同 buff_id 已存在
//!   且雙方 payload 都帶 `slow_factor` 時，較小者勝出（refresh duration 不變、
//!   payload 只在新 factor 較強時才覆寫）。任一邊缺欄位或型別不對 → fallback
//!   到舊行為（直接覆寫）。新增其他需要「比較合併」的 stat 時，請在這裡記載。

use omb_script_abi::buff_ids::BuffId;
use omb_script_abi::stat_keys::StatKey;
use omoba_sim::Fixed32;
use serde_json::Value;
use specs::Entity;
use std::collections::HashMap;

/// Read a numeric stat value out of a JSON payload as Fixed32.
/// JSON payload keeps f64 (script wire format), conversion happens at boundary.
/// TODO Phase 1[d]: BuffStore wire payload migrate to deterministic encoding (raw i32).
#[inline]
fn read_fixed_from_payload(value: &serde_json::Value) -> Fixed32 {
    value
        .as_f64()
        .map(|v| Fixed32::from_raw((v * 1024.0) as i32))
        .unwrap_or(Fixed32::ZERO)
}

#[derive(Debug, Clone)]
pub struct BuffEntry {
    pub remaining: Fixed32,
    pub payload: Value,
}

/// 以 `(Entity, buff_id)` 為 key 的 O(1) buff 索引。
/// `entities_by_key` 是 stat key → entity → 引用計數的反向索引，
/// 加速「哪些 entity 受某類 stat 影響」的查詢（regen / DoT 系統用）。
#[derive(Default, Debug)]
pub struct BuffStore {
    buffs: HashMap<(Entity, String), BuffEntry>,
    entities_by_key: HashMap<String, HashMap<Entity, u32>>,
}

impl BuffStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 新增或刷新 buff。若已存在：duration 取 max、payload 依 should_replace
    /// 策略決定是否覆寫——例如 slow 採單一 instance（buff_id = "slow"），
    /// 由 payload 的 `slow_factor` 欄位驅動「強蓋弱」比較（見上方 Reserved
    /// payload conventions）。
    pub fn add(&mut self, entity: Entity, buff_id: &str, duration: Fixed32, payload: Value) {
        let key = (entity, buff_id.to_string());
        match self.buffs.get_mut(&key) {
            Some(e) => {
                if duration > e.remaining {
                    e.remaining = duration;
                }
                // payload 替換策略：
                //   - 若雙方 payload 都帶 `slow_factor`，較小者（更強）勝出，
                //     僅在新 payload 較強時才覆寫；否則保留原 payload。
                //   - 否則維持原本行為：覆寫。
                let should_replace = match (
                    e.payload.get("slow_factor").and_then(|v| v.as_f64()),
                    payload.get("slow_factor").and_then(|v| v.as_f64()),
                ) {
                    (Some(old), Some(new)) => new < old,
                    // 任一邊缺 slow_factor 或型別非數字 → fallback 到舊行為（覆寫）
                    _ => true,
                };
                if should_replace {
                    let old_keys: Vec<String> =
                        Self::payload_keys(&e.payload).map(String::from).collect();
                    let new_keys: Vec<String> =
                        Self::payload_keys(&payload).map(String::from).collect();
                    e.payload = payload;
                    for k in &old_keys {
                        if !new_keys.contains(k) {
                            self.index_dec(entity, k);
                        }
                    }
                    for k in &new_keys {
                        if !old_keys.contains(k) {
                            self.index_inc(entity, k);
                        }
                    }
                }
                // should_replace == false: duration 已 refresh, payload 不動, 索引不動
            }
            None => {
                let new_keys: Vec<String> = Self::payload_keys(&payload).map(String::from).collect();
                self.buffs.insert(
                    key,
                    BuffEntry {
                        remaining: duration,
                        payload,
                    },
                );
                for k in &new_keys {
                    self.index_inc(entity, k);
                }
            }
        }
    }

    pub fn remove(&mut self, entity: Entity, buff_id: &str) {
        if let Some(entry) = self.buffs.remove(&(entity, buff_id.to_string())) {
            let keys: Vec<String> = Self::payload_keys(&entry.payload).map(String::from).collect();
            for k in &keys {
                self.index_dec(entity, k);
            }
        }
    }

    pub fn has(&self, entity: Entity, buff_id: &str) -> bool {
        self.buffs.contains_key(&(entity, buff_id.to_string()))
    }

    pub fn get(&self, entity: Entity, buff_id: &str) -> Option<&BuffEntry> {
        self.buffs.get(&(entity, buff_id.to_string()))
    }

    /// 清除 entity 的所有 buff（單位死亡時呼叫）。
    pub fn remove_all_for(&mut self, entity: Entity) {
        // 收集要清掉的 buff 索引（避免 retain 內部觸 &mut self）
        let drained: Vec<(Entity, String)> = self
            .buffs
            .iter()
            .filter(|((e, _), _)| *e == entity)
            .map(|((e, id), _)| (*e, id.clone()))
            .collect();
        for (e, id) in drained {
            if let Some(entry) = self.buffs.remove(&(e, id.clone())) {
                let keys: Vec<String> =
                    Self::payload_keys(&entry.payload).map(String::from).collect();
                for k in &keys {
                    self.index_dec(e, k);
                }
            }
        }
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

    /// 從 payload 抽出所有頂層 key（這些就是 stat key 字串）。
    /// payload 不是 Object 時返回空 iterator。
    fn payload_keys(payload: &Value) -> impl Iterator<Item = &str> {
        payload
            .as_object()
            .into_iter()
            .flat_map(|m| m.keys().map(|s| s.as_str()))
    }

    fn index_inc(&mut self, entity: Entity, key: &str) {
        let inner = self.entities_by_key.entry(key.to_string()).or_default();
        *inner.entry(entity).or_insert(0) += 1;
    }

    fn index_dec(&mut self, entity: Entity, key: &str) {
        if let Some(inner) = self.entities_by_key.get_mut(key) {
            if let Some(cnt) = inner.get_mut(&entity) {
                *cnt = cnt.saturating_sub(1);
                if *cnt == 0 {
                    inner.remove(&entity);
                }
            }
            if inner.is_empty() {
                self.entities_by_key.remove(key);
            }
        }
    }

    /// 反向查詢：哪些 entity 身上有 buff payload 含 `key`。
    /// 配合 `regen_tick` / `buff_tick` 的 DoT 掃描，把「對全表 sum_add」
    /// 變成「只對候選 entity sum_add」。返回 iterator，呼叫端可 collect 或 filter。
    pub fn entities_with_key<'a>(&'a self, key: &str) -> impl Iterator<Item = Entity> + 'a {
        self.entities_by_key
            .get(key)
            .into_iter()
            .flat_map(|m| m.keys().copied())
    }

    /// 加法聚合：對 entity 身上所有 buff，若 `payload[stat]` 是數字則加總。
    /// 慣例：`_bonus` 後綴的 stat 用這個（例 `range_bonus`、`damage_bonus`）。
    /// Phase 1c.3: returns Fixed32; payload still f64 (JSON wire format), converted at boundary.
    /// TODO Phase 1[d]: BuffStore wire payload migrate to deterministic encoding (raw i32).
    pub fn sum_add(&self, entity: Entity, stat: StatKey) -> Fixed32 {
        let key = stat.as_str();
        self.iter_for(entity)
            .filter_map(|(_, e)| e.payload.get(key))
            .map(read_fixed_from_payload)
            .fold(Fixed32::ZERO, |acc, v| acc + v)
    }

    /// 乘法聚合：對 entity 身上所有 buff，若 `payload[stat]` 是數字則連乘。
    /// 空集合回 1.0 (Fixed32::ONE)。慣例：`_multiplier` 後綴的 stat 用這個
    /// （例 `attack_speed_multiplier`、`move_speed_multiplier`）。
    /// Phase 1c.3: returns Fixed32; payload still f64 (JSON wire format), converted at boundary.
    /// TODO Phase 1[d]: BuffStore wire payload migrate to deterministic encoding (raw i32).
    pub fn product_mult(&self, entity: Entity, stat: StatKey) -> Fixed32 {
        let key = stat.as_str();
        self.iter_for(entity)
            .filter_map(|(_, e)| e.payload.get(key))
            .map(read_fixed_from_payload)
            .fold(Fixed32::ONE, |acc, v| acc * v)
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
    /// Phase 1c.3: dt is Fixed32 seconds.
    pub fn tick(&mut self, dt: Fixed32) -> Vec<(Entity, String, Value)> {
        let mut expired = Vec::new();
        // 先收集 expired，避免 retain 內動態借 self（index_dec 也要 &mut self）
        let mut to_drop: Vec<(Entity, String)> = Vec::new();
        for ((e, id), v) in self.buffs.iter_mut() {
            v.remaining -= dt;
            if v.remaining <= Fixed32::ZERO {
                to_drop.push((*e, id.clone()));
            }
        }
        for (e, id) in to_drop {
            if let Some(entry) = self.buffs.remove(&(e, id.clone())) {
                let keys: Vec<String> = Self::payload_keys(&entry.payload).map(String::from).collect();
                for k in &keys {
                    self.index_dec(e, k);
                }
                expired.push((e, id, entry.payload));
            }
        }
        expired
    }

    pub fn len(&self) -> usize {
        self.buffs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buffs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use specs::world::Generation;

    fn ent(id: u32, gen: i32) -> Entity {
        Entity::new(id, Generation::new(gen))
    }

    fn fx(seconds: f32) -> Fixed32 {
        Fixed32::from_raw((seconds * 1024.0) as i32)
    }

    #[test]
    fn entities_with_key_returns_entity_after_add() {
        let mut s = BuffStore::new();
        let e = ent(1, 1);
        s.add(e, "buff_a", fx(5.0), json!({ "move_speed_bonus": -0.5 }));
        let found: Vec<Entity> = s.entities_with_key("move_speed_bonus").collect();
        assert_eq!(found, vec![e]);
    }

    #[test]
    fn remove_clears_index() {
        let mut s = BuffStore::new();
        let e = ent(1, 1);
        s.add(e, "b", fx(5.0), json!({ "x": 1.0 }));
        s.remove(e, "b");
        let found: Vec<Entity> = s.entities_with_key("x").collect();
        assert!(found.is_empty(), "expected empty, got {:?}", found);
    }

    #[test]
    fn tick_expired_clears_index() {
        let mut s = BuffStore::new();
        let e = ent(1, 1);
        s.add(e, "b", fx(1.0), json!({ "x": 1.0 }));
        let expired = s.tick(fx(2.0)); // duration < dt → expire
        assert_eq!(expired.len(), 1);
        let found: Vec<Entity> = s.entities_with_key("x").collect();
        assert!(found.is_empty(), "expected empty after expire, got {:?}", found);
    }

    #[test]
    fn remove_all_for_clears_index() {
        let mut s = BuffStore::new();
        let e = ent(1, 1);
        s.add(e, "a", fx(5.0), json!({ "x": 1.0, "y": 2.0 }));
        s.add(e, "b", fx(5.0), json!({ "z": 3.0 }));
        s.remove_all_for(e);
        for k in &["x", "y", "z"] {
            let found: Vec<Entity> = s.entities_with_key(k).collect();
            assert!(found.is_empty(), "key {} not cleared: {:?}", k, found);
        }
    }

    #[test]
    fn refcount_multiple_buffs_same_key() {
        let mut s = BuffStore::new();
        let e = ent(1, 1);
        s.add(e, "buff1", fx(5.0), json!({ "k": 1.0 }));
        s.add(e, "buff2", fx(5.0), json!({ "k": 2.0 }));

        // both present — entity still in index
        assert_eq!(s.entities_with_key("k").count(), 1);

        s.remove(e, "buff1");
        // one still left → still indexed
        let found: Vec<Entity> = s.entities_with_key("k").collect();
        assert_eq!(found, vec![e], "after removing 1 of 2, entity should still be indexed");

        s.remove(e, "buff2");
        // both gone → not indexed
        assert!(s.entities_with_key("k").next().is_none());
    }

    #[test]
    fn slow_dedup_stronger_replaces_weaker() {
        let mut s = BuffStore::new();
        let e = ent(1, 1);
        // 先加弱 slow（factor 越小越強，0.5 比 0.3 弱）
        s.add(e, "slow", fx(5.0), json!({ "move_speed_bonus": -0.5, "slow_factor": 0.5 }));
        // 加強 slow
        s.add(e, "slow", fx(5.0), json!({ "move_speed_bonus": -0.7, "slow_factor": 0.3 }));
        // 應該保留強 slow（factor=0.3）
        let entry = s.get(e, "slow").expect("slow buff missing");
        let factor = entry.payload.get("slow_factor").and_then(|v| v.as_f64()).unwrap();
        assert!((factor - 0.3).abs() < 1e-6, "expected 0.3 (stronger), got {}", factor);
    }

    #[test]
    fn slow_dedup_weaker_does_not_replace_stronger() {
        let mut s = BuffStore::new();
        let e = ent(1, 1);
        s.add(e, "slow", fx(3.0), json!({ "move_speed_bonus": -0.7, "slow_factor": 0.3 }));
        s.add(e, "slow", fx(10.0), json!({ "move_speed_bonus": -0.5, "slow_factor": 0.5 }));
        let entry = s.get(e, "slow").expect("slow buff missing");
        let factor = entry.payload.get("slow_factor").and_then(|v| v.as_f64()).unwrap();
        assert!((factor - 0.3).abs() < 1e-6, "expected 0.3 to be preserved, got {}", factor);
        // duration 應取 max（既有行為）
        assert!(entry.remaining >= fx(9.99), "expected duration ≥ 10, got {:?}", entry.remaining);
    }
}
