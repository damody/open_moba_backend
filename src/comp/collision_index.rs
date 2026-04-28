//! Collision spatial index：包 `Box<dyn SpatialIndex<Entity, ()>>` 提供
//! Searcher 用的高層 API（search_nn / search_nn_two_radii / rebuild_from / dirty 標記）。
//!
//! 所有回傳 `DisIndex` 的方法都用「squared distance」當 `.dis` 欄位，與舊 PosData 行為一致 —
//! 多數 caller 都拿 `di.dis < touch * touch` 比較。

use specs::Entity;
use vek::Vec2;

use crate::comp::outcome::DisIndex;
use crate::vision::{Bounds, Entry, SpatialIndex, SpatialIndexParams, build_entity_index};

/// 預設 collision world bounds：給 QuadTree/BVH 用，SAP/SHG 忽略。
/// 寬鬆設定避免 entities 落在 bounds 之外被忽略；之後可從 toml 調整。
const DEFAULT_WORLD_MIN: f32 = -10000.0;
const DEFAULT_WORLD_MAX: f32 = 10000.0;

pub struct CollisionIndex {
    index: Box<dyn SpatialIndex<Entity, ()>>,
    bounds: Bounds,
    dirty: bool,
    kind: &'static str,
}

impl CollisionIndex {
    pub fn new(kind: &str, params: SpatialIndexParams) -> Self {
        let bounds = Bounds::new(
            Vec2::new(DEFAULT_WORLD_MIN, DEFAULT_WORLD_MIN),
            Vec2::new(DEFAULT_WORLD_MAX, DEFAULT_WORLD_MAX),
        );
        let index = build_entity_index(kind, params);
        let kind_static = index.name();
        let mut idx = Self {
            index,
            bounds,
            dirty: false,
            kind: kind_static,
        };
        // 預先 initialize 一個空的索引，讓後續 insert 可以動作
        idx.index.initialize(idx.bounds.clone(), Vec::new());
        idx
    }

    /// 用一組 (Entity, position) 整批替換索引。對應舊 PosData 的 clear+push+sort 流程，
    /// 但走 trait 的 `bulk_replace`：default 等同 initialize 全 reset；SAP override 成
    /// 「保留 slot map、diff 增減的部份、再對 xs/ys 重新排序」的 incremental 路徑。
    pub fn rebuild_from<I>(&mut self, items: I)
    where I: IntoIterator<Item = (Entity, Vec2<f32>)>
    {
        let entries: Vec<Entry<Entity, ()>> = items.into_iter()
            .map(|(e, p)| Entry::point(e, (), p))
            .collect();
        self.index.bulk_replace(self.bounds.clone(), entries);
        self.dirty = false;
    }

    pub fn mark_dirty(&mut self) { self.dirty = true; }
    pub fn is_dirty(&self) -> bool { self.dirty }

    /// Diagnostic counter — 對 SAP/SHG/BVH 是 entry 數，對 QuadTree 是節點數
    pub fn count(&self) -> usize { self.index.count_nodes() }

    pub fn kind(&self) -> &'static str { self.kind }

    /// 範圍查詢：回傳 `Vec<DisIndex { e, dis }>`，dis 為 squared distance；
    /// 按 dis 升冪 sort，取前 n 個。
    pub fn search_nn(&self, pos: Vec2<f32>, radius: f32, n: usize) -> Vec<DisIndex> {
        let r2 = radius * radius;
        let mut out: Vec<DisIndex> = Vec::with_capacity(n);
        for entry in self.index.query_in_range(pos, radius) {
            let d2 = entry.position.distance_squared(pos);
            if d2 < r2 {
                out.push(DisIndex { e: entry.id, dis: d2 });
            }
        }
        out.sort_by(|a, b| a.dis.partial_cmp(&b.dis).unwrap_or(std::cmp::Ordering::Equal));
        out.truncate(n);
        out
    }

    /// 兩半徑查詢：r_inner 圓內的 entries 放 inner，r_inner..=r_outer 區間放 outer。
    /// inner 按 dis 排序並截 n 個；outer 不排序也不截（caller 自決）。
    pub fn search_nn_two_radii(
        &self,
        pos: Vec2<f32>,
        r_inner: f32,
        r_outer: f32,
        n: usize,
    ) -> (Vec<DisIndex>, Vec<DisIndex>) {
        let r2_inner = r_inner * r_inner;
        let r2_outer = r_outer * r_outer;
        let mut inner: Vec<DisIndex> = Vec::new();
        let mut outer: Vec<DisIndex> = Vec::new();
        for entry in self.index.query_in_range(pos, r_outer) {
            let d2 = entry.position.distance_squared(pos);
            if d2 < r2_inner {
                inner.push(DisIndex { e: entry.id, dis: d2 });
            } else if d2 < r2_outer {
                outer.push(DisIndex { e: entry.id, dis: d2 });
            }
        }
        inner.sort_by(|a, b| a.dis.partial_cmp(&b.dis).unwrap_or(std::cmp::Ordering::Equal));
        inner.truncate(n);
        (inner, outer)
    }
}
