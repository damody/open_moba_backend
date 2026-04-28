//! Vision 子系統設定（從 omb/game.toml 的 `[vision]` 區段讀取）。
//!
//! 與 server_config.rs 不同：缺檔 / 缺 section / 解析錯誤都不 panic，
//! 直接 fallback 到內建預設值（quadtree, cell_size 128）。理由：vision tests
//! 會在不同 cwd 下跑，硬性 panic 會干擾 unit test 執行。

use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use std::fs;

fn default_spatial_index() -> String { "quadtree".to_string() }
fn default_shg_cell_size() -> f32 { 128.0 }
fn default_quadtree_max_depth() -> usize { 8 }
fn default_quadtree_max_per_node() -> usize { 10 }
fn default_bvh_max_leaf() -> usize { 4 }

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct VisionSetting {
    /// 切換 spatial index：`"quadtree"` | `"hash_grid"` | `"bvh"`
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

#[derive(Deserialize)]
struct Wrapper {
    vision: Option<VisionSetting>,
}

impl VisionSetting {
    /// 從 game.toml 讀；任何錯誤（檔案不存在、無 `[vision]` section、解析失敗）
    /// 都靜默 fallback 到 Default，不 panic。
    fn load_or_default() -> Self {
        let raw = match fs::read_to_string("game.toml") {
            Ok(s) => s,
            Err(_) => return Self::default(),
        };
        match toml::from_str::<Wrapper>(&raw) {
            Ok(w) => w.vision.unwrap_or_default(),
            Err(e) => {
                log::warn!("[vision] section parse failed: {} — using defaults", e);
                Self::default()
            }
        }
    }
}

lazy_static! {
    pub static ref VISION_CONFIG: VisionSetting = VisionSetting::load_or_default();
}
