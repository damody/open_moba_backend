/// 視野系統模組
///
/// 包含視野計算、輸出格式、性能優化等非ECS組件
pub mod vision_output;
pub mod shadow_calculator;
pub mod vision_ecs;
pub mod test_vision;
pub mod debug_test;
pub mod mathematical_tests;
pub mod improved_tests;
pub mod spatial_index;
pub mod quadtree;
pub mod hash_grid;
pub mod bvh;
pub mod sweep_and_prune;
pub mod shadow_calculation;
pub mod vision_cache;
pub mod geometry_utils;
#[cfg(test)]
pub mod spatial_index_consistency_tests;

pub use self::{
    vision_output::*,
    shadow_calculator::ShadowCalculator,
    spatial_index::{SpatialIndex, SpatialIndexParams, Entry, TreeEntry, Bounds, build_spatial_index, build_entity_index},
    quadtree::QuadTree,
    hash_grid::SpatialHashGrid,
    bvh::Bvh,
    sweep_and_prune::SweepAndPrune,
    vision_ecs::*,
};