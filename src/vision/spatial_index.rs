use crate::comp::circular_vision::ObstacleInfo;

pub use omoba_core::runtime::spatial::{
    build_entity_index, Bounds, Entry, SpatialIndex, SpatialIndexParams,
};

pub fn build_spatial_index(
    kind: &str,
    params: SpatialIndexParams,
) -> Box<dyn SpatialIndex<String, ObstacleInfo>> {
    omoba_core::runtime::spatial::build_spatial_index(kind, params)
}

pub type TreeEntry = Entry<String, ObstacleInfo>;
