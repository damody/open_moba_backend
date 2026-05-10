//! TD 塔的 runtime registry — 由腳本 `tower_metadata()` 回報、host 在 load_scripts
//! 結束時填好，當作 resource 存在 ECS。新增第 5 種塔只需寫新腳本 + 重 build DLL，
//! host 這邊什麼都不用動（placement validation、spawn、UI broadcast 都從這讀）。
//!
//! 與 `tower_template.rs` 的不同：
//! - `tower_template.rs` 已移除（舊的硬編 TowerKind enum）
//! - 這支 `tower_registry.rs` 是執行期動態填的 resource

use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct TowerRenderPoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Debug)]
pub struct TowerRenderAnimation {
    pub fps: f32,
    pub loop_animation: bool,
    pub fire_fps: f32,
    pub fire_once: bool,
}

#[derive(Clone, Debug)]
pub struct TowerBarrelVariant {
    pub min_path: u8,
    pub min_level: u8,
    pub count: u16,
    pub image: String,
    pub frames: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct TowerRecoil {
    pub mode: String,
    pub distance: f32,
    pub scale: f32,
    pub duration_ms: u32,
    pub return_ms: u32,
}

#[derive(Clone, Debug)]
pub struct TowerRenderMetadata {
    pub render_mode: String,
    pub base: String,
    pub barrel: String,
    pub visual_size: f32,
    pub barrel_frames: Vec<String>,
    pub body_frames: Vec<String>,
    pub barrel_animation: TowerRenderAnimation,
    pub body_animation: TowerRenderAnimation,
    pub rotation_mode: String,
    pub barrel_layout: String,
    pub barrel_variants: Vec<TowerBarrelVariant>,
    pub barrel_offset: TowerRenderPoint,
    pub barrel_pivot: TowerRenderPoint,
    pub muzzle_offset: TowerRenderPoint,
    pub default_angle_deg: f32,
    pub recoil: TowerRecoil,
}

#[derive(Copy, Clone, Debug)]
pub struct AttackTimingMetadata {
    pub windup: u16,
    pub backswing: u16,
}

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
    pub placement_radius: f32,
    pub hp: f32,
    pub turn_speed_deg: f32,
    pub render: TowerRenderMetadata,
    pub attack_timing: AttackTimingMetadata,
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
