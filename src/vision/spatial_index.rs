//! Spatial index 抽象層：QuadTree / SpatialHashGrid / SAH BVH 三者的共用 trait 與型別。
//!
//! 切換邏輯：`build_spatial_index(kind, params)` factory 從字串選擇 impl，
//! 字串通常源自 `omb/game.toml` 的 `[vision] SPATIAL_INDEX`。
//!
//! 設計選擇：用 `Box<dyn SpatialIndex>` 而非 enum dispatch — 三 impl 內部 size 差異大，
//! enum 會 bloat；vtable 在 vision query 量級下不是熱點（浮點 shadow casting 才是）。

use vek::Vec2;

use crate::comp::circular_vision::ObstacleInfo;

/// 一筆儲存於 spatial index 內的障礙物連同它的字串 id。
/// id 由呼叫端決定，用於增量 update/remove 與 cache invalidation 追蹤。
#[derive(Debug, Clone)]
pub struct TreeEntry {
    pub id: String,
    pub obstacle: ObstacleInfo,
}

/// 軸對齊邊界矩形（AABB），所有 spatial impl 共用。
#[derive(Debug, Clone)]
pub struct Bounds {
    pub min: Vec2<f32>,
    pub max: Vec2<f32>,
}

impl Bounds {
    pub fn new(min: Vec2<f32>, max: Vec2<f32>) -> Self {
        Self { min, max }
    }

    pub fn contains_point(&self, point: Vec2<f32>) -> bool {
        point.x >= self.min.x && point.x <= self.max.x &&
        point.y >= self.min.y && point.y <= self.max.y
    }

    pub fn width(&self) -> f32 {
        self.max.x - self.min.x
    }

    pub fn height(&self) -> f32 {
        self.max.y - self.min.y
    }
}

/// 構造 spatial index 時的可選參數。各 impl 只讀自己需要的欄位，未提供時用 impl-local default。
#[derive(Debug, Clone)]
pub struct SpatialIndexParams {
    /// QuadTree 用：樹深度上限
    pub quadtree_max_depth: usize,
    /// QuadTree 用：每節點 entry 上限（超過則 subdivide）
    pub quadtree_max_per_node: usize,
    /// SpatialHashGrid 用：cell 邊長（世界座標單位）
    pub hash_grid_cell_size: f32,
    /// BVH 用：每葉節點 entry 上限
    pub bvh_max_leaf: usize,
}

impl Default for SpatialIndexParams {
    fn default() -> Self {
        Self {
            quadtree_max_depth: 8,
            quadtree_max_per_node: 10,
            hash_grid_cell_size: 128.0,
            bvh_max_leaf: 4,
        }
    }
}

/// Spatial index 抽象。三個 impl（QuadTree / SpatialHashGrid / SAH BVH）必須實作這組方法。
///
/// `&self` query / `&mut self` mutation 由借用檢查器自然分隔；vision 是單執行緒所以不切兩個 trait。
pub trait SpatialIndex: Send + Sync {
    /// 用一組 entries 整批建立索引（取代舊的內容）。
    fn initialize(&mut self, bounds: Bounds, entries: Vec<TreeEntry>);

    /// 增量插入單一 entry。要求事先 `initialize` 過。
    fn insert(&mut self, id: String, obstacle: ObstacleInfo);

    /// 移除 id 對應的所有 entry（跨 cell/leaf 的 clone 一次清掉）。
    /// 回傳是否真的有 entry 被刪除。
    fn remove(&mut self, id: &str) -> bool;

    /// 等同 `remove(id)` 後 `insert(id, obstacle)`。
    fn update(&mut self, id: &str, obstacle: ObstacleInfo);

    /// 範圍查詢，回傳 (id, obstacle) 配對；同 id 的多個副本只回傳一次。
    fn query_entries_in_range(&self, center: Vec2<f32>, radius: f32) -> Vec<(String, ObstacleInfo)>;

    /// 範圍查詢，只回障礙物本體（shadow casting 不需要 id）。
    fn query_obstacles_in_range(&self, center: Vec2<f32>, radius: f32) -> Vec<ObstacleInfo> {
        self.query_entries_in_range(center, radius)
            .into_iter()
            .map(|(_, o)| o)
            .collect()
    }

    /// 結構大小指標（QuadTree = 節點數、SHG = cell 數、BVH = node 數）。
    fn count_nodes(&self) -> usize;

    /// debug / log 用名稱（"quadtree" / "hash_grid" / "bvh"）。
    fn name(&self) -> &'static str;
}

/// Factory：依字串名挑 impl。未知 name 落回 quadtree 並 log warn。
pub fn build_spatial_index(kind: &str, params: SpatialIndexParams) -> Box<dyn SpatialIndex> {
    match kind {
        "quadtree" => {
            let idx = super::quadtree::QuadTree::new(params.quadtree_max_depth, params.quadtree_max_per_node);
            log::info!("SpatialIndex initialized: quadtree (depth={}, per_node={})",
                       params.quadtree_max_depth, params.quadtree_max_per_node);
            Box::new(idx)
        }
        "hash_grid" => {
            let idx = super::hash_grid::SpatialHashGrid::new(params.hash_grid_cell_size);
            log::info!("SpatialIndex initialized: hash_grid (cell_size={})",
                       params.hash_grid_cell_size);
            Box::new(idx)
        }
        "bvh" => {
            let idx = super::bvh::Bvh::new(params.bvh_max_leaf);
            log::info!("SpatialIndex initialized: bvh (max_leaf={})", params.bvh_max_leaf);
            Box::new(idx)
        }
        other => {
            log::warn!("Unknown SPATIAL_INDEX = {:?}, falling back to quadtree", other);
            let idx = super::quadtree::QuadTree::new(params.quadtree_max_depth, params.quadtree_max_per_node);
            Box::new(idx)
        }
    }
}
