pub mod bvh;
pub mod debug_test;
pub mod geometry_utils;
pub mod hash_grid;
pub mod improved_tests;
pub mod mathematical_tests;
pub mod quadtree;
pub mod shadow_calculation;
pub mod shadow_calculator;
pub mod spatial_index;
#[cfg(test)]
pub mod spatial_index_consistency_tests;
pub mod sweep_and_prune;
pub mod test_vision;
pub mod vision_cache;
pub mod vision_ecs;
/// 視野系統模組
///
/// 包含視野計算、輸出格式、性能優化等非ECS組件
pub mod vision_output;

pub use self::{
    bvh::Bvh,
    hash_grid::SpatialHashGrid,
    quadtree::QuadTree,
    shadow_calculator::ShadowCalculator,
    spatial_index::{
        build_entity_index, build_spatial_index, Bounds, Entry, SpatialIndex, SpatialIndexParams,
        TreeEntry,
    },
    sweep_and_prune::SweepAndPrune,
    vision_ecs::*,
    vision_output::*,
};
