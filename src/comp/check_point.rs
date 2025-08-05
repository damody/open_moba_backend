use serde::{Deserialize, Serialize};
use vek::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CheckPoint {
    pub name: String,
    pub class: String,
    pub pos: Vec2<f32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Path {
    pub check_points: Vec<CheckPoint>,
}


