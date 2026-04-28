//! Generic spatial index 抽象層：QuadTree / SpatialHashGrid / SAH BVH / SweepAndPrune
//! 共用同一組 trait 與型別。
//!
//! 兩個 type parameter：
//! - `Id`：用來識別 entry，需 Clone + Eq + Hash + Ord + Send + Sync + 'static
//!   - vision 用 `String`（"obstacle_{i}" / 自訂 id）
//!   - collision 用 `specs::Entity`
//! - `Item`：跟 entry 綁的任意 payload（vision 用 `ObstacleInfo`、collision 用 `()`）
//!
//! 設計選擇：
//! - 用 `Box<dyn SpatialIndex<Id, Item>>` 而非 enum dispatch — 不同 impl 內部 size 差距大
//! - Entry 內含 position + bounding_radius，insert/update 收一個 Entry，呼叫端不必傳 4 參數
//! - `query_in_range` 回傳完整 Entry，方便 caller 同時拿到 id / item / position / radius
//! - `query_with_distance` 有 default 實作，SAP 等想一輪掃完者可 override

use std::fmt::Debug;
use std::hash::Hash;
use vek::Vec2;

/// 一筆 spatial index entry。所有 impl 共用。
#[derive(Debug, Clone)]
pub struct Entry<Id, Item> {
    pub id: Id,
    pub item: Item,
    pub position: Vec2<f32>,
    /// 外接半徑。對 query 範圍判斷會做 `distance <= query_radius + bounding_radius`。
    /// 純點狀 entity 設 0.0 即可。
    pub bounding_radius: f32,
}

impl<Id, Item> Entry<Id, Item> {
    pub fn new(id: Id, item: Item, position: Vec2<f32>, bounding_radius: f32) -> Self {
        Self { id, item, position, bounding_radius }
    }

    pub fn point(id: Id, item: Item, position: Vec2<f32>) -> Self {
        Self { id, item, position, bounding_radius: 0.0 }
    }
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

    pub fn width(&self) -> f32 { self.max.x - self.min.x }
    pub fn height(&self) -> f32 { self.max.y - self.min.y }
}

/// 構造 spatial index 時的可選參數。各 impl 只讀自己需要的欄位。
#[derive(Debug, Clone)]
pub struct SpatialIndexParams {
    pub quadtree_max_depth: usize,
    pub quadtree_max_per_node: usize,
    pub hash_grid_cell_size: f32,
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

/// Spatial index 抽象。實作必須對 (Id, Item) 為 generic 的方式設計。
///
/// 關於 trait object：用 `Box<dyn SpatialIndex<MyId, MyItem>>` 即可。`Id`/`Item`
/// 是 trait-level type params，故 trait 仍 object-safe。
pub trait SpatialIndex<Id, Item>: Send + Sync
where
    Id: Clone + Eq + Hash + Ord + Send + Sync + 'static,
    Item: Clone + Send + Sync + 'static,
{
    /// 用一組 entries 整批建立索引（取代舊內容）。
    fn initialize(&mut self, bounds: Bounds, entries: Vec<Entry<Id, Item>>);

    /// 增量插入單一 entry。要求事先 `initialize` 過。
    fn insert(&mut self, entry: Entry<Id, Item>);

    /// 移除 id 對應的所有 entry（跨 cell/leaf 的 clone 一次清掉）。
    /// 回傳是否真的有 entry 被刪除。
    fn remove(&mut self, id: &Id) -> bool;

    /// 等同 `remove(id)` 後 `insert(entry)`。
    fn update(&mut self, entry: Entry<Id, Item>);

    /// 範圍查詢；同 id 的多個副本只回傳一次。
    fn query_in_range(&self, center: Vec2<f32>, radius: f32) -> Vec<Entry<Id, Item>>;

    /// 範圍查詢 + 對中心距離 + 按距離升冪 sort。default 實作走 `query_in_range`，
    /// SAP 等可在自己掃 axis 時順便算 distance 取代之。
    fn query_with_distance(
        &self,
        center: Vec2<f32>,
        radius: f32,
    ) -> Vec<(Entry<Id, Item>, f32)> {
        let mut out: Vec<_> = self.query_in_range(center, radius)
            .into_iter()
            .map(|e| {
                let d = e.position.distance(center);
                (e, d)
            })
            .collect();
        out.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        out
    }

    /// 用一組新 entries 整批替換索引內容。
    /// Default 直接呼叫 `initialize`（全 reset 再 rebuild）。
    /// SAP 等 impl 可 override 成「保留 slot map、diff 增減 + 對 xs/ys 重新排序」
    /// 的 incremental 路徑，避免 high-churn 場景（每 tick 重建 hundreds creep）的浪費。
    fn bulk_replace(&mut self, bounds: Bounds, entries: Vec<Entry<Id, Item>>) {
        self.initialize(bounds, entries);
    }

    /// 結構大小指標（節點數 / cell 數 / array 長度，視 impl 而定）。
    fn count_nodes(&self) -> usize;

    /// debug / log 用名稱（"quadtree" / "hash_grid" / "bvh" / "sap"）。
    fn name(&self) -> &'static str;
}

// ---- Vision factory：(Id=String, Item=ObstacleInfo) ----

use crate::comp::circular_vision::ObstacleInfo;

/// Vision 用 factory：依字串名挑 impl。未知 name 落回 quadtree 並 log warn。
pub fn build_spatial_index(
    kind: &str,
    params: SpatialIndexParams,
) -> Box<dyn SpatialIndex<String, ObstacleInfo>> {
    match kind {
        "quadtree" => {
            log::info!("SpatialIndex initialized: quadtree (depth={}, per_node={})",
                       params.quadtree_max_depth, params.quadtree_max_per_node);
            Box::new(super::quadtree::QuadTree::new(
                params.quadtree_max_depth,
                params.quadtree_max_per_node,
            ))
        }
        "hash_grid" => {
            log::info!("SpatialIndex initialized: hash_grid (cell_size={})",
                       params.hash_grid_cell_size);
            Box::new(super::hash_grid::SpatialHashGrid::new(params.hash_grid_cell_size))
        }
        "bvh" => {
            log::info!("SpatialIndex initialized: bvh (max_leaf={})", params.bvh_max_leaf);
            Box::new(super::bvh::Bvh::new(params.bvh_max_leaf))
        }
        "sap" => {
            log::info!("SpatialIndex initialized: sap");
            Box::new(super::sweep_and_prune::SweepAndPrune::new())
        }
        other => {
            log::warn!("Unknown SPATIAL_INDEX = {:?}, falling back to quadtree", other);
            Box::new(super::quadtree::QuadTree::new(
                params.quadtree_max_depth,
                params.quadtree_max_per_node,
            ))
        }
    }
}

/// Collision 用 factory：(Id=specs::Entity, Item=()) 版本，供 Searcher 使用。
pub fn build_entity_index(
    kind: &str,
    params: SpatialIndexParams,
) -> Box<dyn SpatialIndex<specs::Entity, ()>> {
    match kind {
        "quadtree" => Box::new(super::quadtree::QuadTree::new(
            params.quadtree_max_depth,
            params.quadtree_max_per_node,
        )),
        "hash_grid" => Box::new(super::hash_grid::SpatialHashGrid::new(params.hash_grid_cell_size)),
        "bvh" => Box::new(super::bvh::Bvh::new(params.bvh_max_leaf)),
        "sap" => Box::new(super::sweep_and_prune::SweepAndPrune::new()),
        other => {
            log::warn!("Unknown SPATIAL_INDEX = {:?} for entity index, falling back to sap", other);
            Box::new(super::sweep_and_prune::SweepAndPrune::new())
        }
    }
}

// ---- Backward-compat alias for the old vision-only TreeEntry name ----
pub type TreeEntry = Entry<String, ObstacleInfo>;
