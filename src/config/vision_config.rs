//! Vision + Collision 子系統設定（從 omb/game.toml 的 `[vision]` / `[collision]` 區段讀取）。
//!
//! 與 server_config.rs 不同：缺檔 / 缺 section / 解析錯誤都不 panic，
//! 直接 fallback 到內建預設值。理由：vision tests 會在不同 cwd 下跑，
//! 硬性 panic 會干擾 unit test 執行。

use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use std::fs;

fn default_spatial_index() -> String { "quadtree".to_string() }
fn default_collision_index() -> String { "sap".to_string() }
fn default_shg_cell_size() -> f32 { 128.0 }
fn default_quadtree_max_depth() -> usize { 8 }
fn default_quadtree_max_per_node() -> usize { 10 }
fn default_bvh_max_leaf() -> usize { 4 }

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct VisionSetting {
    /// 切換 vision 用 spatial index：`"quadtree"` | `"hash_grid"` | `"bvh"` | `"sap"`
    #[serde(default = "default_spatial_index")]
    pub SPATIAL_INDEX: String,
    /// SpatialHashGrid cell 邊長（世界座標單位），其他 impl 忽略
    #[serde(default = "default_shg_cell_size")]
    pub SHG_CELL_SIZE: f32,
    /// QuadTree 樹深度上限
    #[serde(default = "default_quadtree_max_depth")]
    pub QUADTREE_MAX_DEPTH: usize,
    /// QuadTree 每節點 entry 上限
    #[serde(default = "default_quadtree_max_per_node")]
    pub QUADTREE_MAX_PER_NODE: usize,
    /// BVH 每葉節點 entry 上限
    #[serde(default = "default_bvh_max_leaf")]
    pub BVH_MAX_LEAF: usize,
}

impl Default for VisionSetting {
    fn default() -> Self {
        Self {
            SPATIAL_INDEX: default_spatial_index(),
            SHG_CELL_SIZE: default_shg_cell_size(),
            QUADTREE_MAX_DEPTH: default_quadtree_max_depth(),
            QUADTREE_MAX_PER_NODE: default_quadtree_max_per_node(),
            BVH_MAX_LEAF: default_bvh_max_leaf(),
        }
    }
}

/// Collision pre-detection 的 spatial index 設定。每個類別獨立選演算法。
#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CollisionSetting {
    /// 塔索引演算法 — 塔大多靜態，BVH 也適合（很少 mark_dirty）
    #[serde(default = "default_collision_index")]
    pub SPATIAL_INDEX_TOWER: String,
    /// 小兵索引 — 大量 + 高頻移動，SAP 與 hash_grid 通常最快
    #[serde(default = "default_collision_index")]
    pub SPATIAL_INDEX_CREEP: String,
    /// 英雄索引 — 數量少 + 移動快，SAP 簡單夠用
    #[serde(default = "default_collision_index")]
    pub SPATIAL_INDEX_HERO: String,
    /// Region blocker 索引 — 一次性靜態填充，BVH 對 query 最快但 SAP 也夠
    #[serde(default = "default_collision_index")]
    pub SPATIAL_INDEX_REGION: String,
    /// SHG cell 邊長（共用 vision 的設定即可，這裡可 override）
    #[serde(default = "default_shg_cell_size")]
    pub SHG_CELL_SIZE: f32,
    #[serde(default = "default_quadtree_max_depth")]
    pub QUADTREE_MAX_DEPTH: usize,
    #[serde(default = "default_quadtree_max_per_node")]
    pub QUADTREE_MAX_PER_NODE: usize,
    #[serde(default = "default_bvh_max_leaf")]
    pub BVH_MAX_LEAF: usize,
}

impl Default for CollisionSetting {
    fn default() -> Self {
        Self {
            SPATIAL_INDEX_TOWER: default_collision_index(),
            SPATIAL_INDEX_CREEP: default_collision_index(),
            SPATIAL_INDEX_HERO: default_collision_index(),
            SPATIAL_INDEX_REGION: default_collision_index(),
            SHG_CELL_SIZE: default_shg_cell_size(),
            QUADTREE_MAX_DEPTH: default_quadtree_max_depth(),
            QUADTREE_MAX_PER_NODE: default_quadtree_max_per_node(),
            BVH_MAX_LEAF: default_bvh_max_leaf(),
        }
    }
}

#[derive(Deserialize)]
struct Wrapper {
    vision: Option<VisionSetting>,
    collision: Option<CollisionSetting>,
}

fn load_wrapper() -> Wrapper {
    let raw = match fs::read_to_string("game.toml") {
        Ok(s) => s,
        Err(_) => return Wrapper { vision: None, collision: None },
    };
    match toml::from_str::<Wrapper>(&raw) {
        Ok(w) => w,
        Err(e) => {
            log::warn!("game.toml [vision]/[collision] parse failed: {} — using defaults", e);
            Wrapper { vision: None, collision: None }
        }
    }
}

impl VisionSetting {
    fn load_or_default() -> Self {
        load_wrapper().vision.unwrap_or_default()
    }
}

impl CollisionSetting {
    fn load_or_default() -> Self {
        load_wrapper().collision.unwrap_or_default()
    }
}

lazy_static! {
    pub static ref VISION_CONFIG: VisionSetting = VisionSetting::load_or_default();
    pub static ref COLLISION_CONFIG: CollisionSetting = CollisionSetting::load_or_default();
}
