//! 不可通行多邊形區域。作為 ECS resource 供移動 tick 查詢。
//!
//! 來源：`map.json` 的 `BlockedRegions` 欄位，由
//! `state::initialization::init_creep_wave` 載入時轉為本資料結構。
//!
//! 使用：`crate::geometry::circle_hits_polygon` 判斷單位（圓形碰撞體積）
//! 是否會進入任一個區域。

use vek::Vec2;

#[derive(Debug, Clone)]
pub struct BlockedRegion {
    pub name: String,
    pub points: Vec<Vec2<f32>>,
}

#[derive(Default, Debug, Clone)]
pub struct BlockedRegions(pub Vec<BlockedRegion>);
