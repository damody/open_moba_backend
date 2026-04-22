//! TD 塔的 runtime registry — 由腳本 `tower_metadata()` 回報、host 在 load_scripts
//! 結束時填好，當作 resource 存在 ECS。新增第 5 種塔只需寫新腳本 + 重 build DLL，
//! host 這邊什麼都不用動（placement validation、spawn、UI broadcast 都從這讀）。
//!
//! 與 `tower_template.rs` 的不同：
//! - `tower_template.rs` 已移除（舊的硬編 TowerKind enum）
//! - 這支 `tower_registry.rs` 是執行期動態填的 resource

use std::collections::HashMap;

/// 一座塔的完整模板：與腳本的 `TowerMetadata` 一對一，但用 owned `String` 方便 host 使用。
#[derive(Clone, Debug)]
pub struct TowerTemplate {
    pub unit_id: String,
    pub label: String,

    pub atk: f32,
    pub asd_interval: f32,
    pub range: f32,
    pub bullet_speed: f32,
    pub splash_radius: f32,
    pub hit_radius: f32,
    pub slow_factor: f32,
    pub slow_duration: f32,

    pub cost: i32,
    pub footprint: f32,
    pub hp: f32,
    pub turn_speed_deg: f32,
}

/// 所有已註冊塔的 registry。順序 = 腳本 DLL `units()` 回傳順序，供前端 UI 按鈕排序用。
#[derive(Default, Clone, Debug)]
pub struct TowerTemplateRegistry {
    pub templates: HashMap<String, TowerTemplate>,
    pub order: Vec<String>,
}

impl TowerTemplateRegistry {
    pub fn get(&self, unit_id: &str) -> Option<&TowerTemplate> {
        self.templates.get(unit_id)
    }

    pub fn insert(&mut self, t: TowerTemplate) {
        if !self.templates.contains_key(&t.unit_id) {
            self.order.push(t.unit_id.clone());
        }
        self.templates.insert(t.unit_id.clone(), t);
    }

    pub fn iter_ordered(&self) -> impl Iterator<Item = &TowerTemplate> {
        self.order.iter().filter_map(|id| self.templates.get(id))
    }

    pub fn is_empty(&self) -> bool {
        self.templates.is_empty()
    }
}
