//! `IsBuilding` — 非移動結構的 ZST 標記組件（塔樓、
//! 未來的營房/病房/陷阱）。
//!
//! 由`UnitStats`用來跳過移動速度/重生/視覺/幻象/
//! 賞金修正聚合－建築物不會移動，也不會重生。
//! 檢查“has IsBuilding”是詢問“這是一棟建築物嗎？”的規範方式。
//! 而不是查看“Tower”/未來的特定組件。

use specs::storage::NullStorage;
use specs::Component;

#[derive(Default, Debug, Clone, Copy)]
pub struct IsBuilding;

impl Component for IsBuilding {
    type Storage = NullStorage<Self>;
}
