use hashbrown::HashSet;
use instant_distance::{Builder, Search};
use omoba_sim::{Fixed64, Vec2 as SimVec2};
use serde::{Deserialize, Serialize};
use specs::storage::VecStorage;
use specs::{Component, FlaggedStorage, NullStorage};
use std::sync::Arc;
use vek::*;

/// 位置
#[derive(Copy, Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pos(pub SimVec2);

impl Pos {
    /// 邊界助手：由兩個「f32」建構（通常是世界座標
    /// 配置/生成數據）。以 `Fixed64::from_raw` 量化進行路由。
    /// 注意：舊版 f32 幫助程式保留為線格式/配置讀取邊界的轉換實用程式。
    #[inline]
    pub fn from_xy_f32(x: f32, y: f32) -> Self {
        Pos(SimVec2 {
            x: Fixed64::from_raw((x * 1024.0) as i64),
            y: Fixed64::from_raw((y * 1024.0) as i64),
        })
    }

    /// 边界助手：底层坐标的有损“f32”投影。使用於
    /// 有線格式/視覺特效/非確定性容忍查詢網站。
    /// 注意：傳統的 f32 投影保留用於有線格式/VFX/確定性容忍查詢站點；sim-side直接讀取SimVec2。
    #[inline]
    pub fn xy_f32(&self) -> (f32, f32) {
        (self.0.x.to_f32_for_render(), self.0.y.to_f32_for_render())
    }
}

impl Component for Pos {
    type Storage = VecStorage<Self>;
}

impl instant_distance::Point for Pos {
    fn distance(&self, other: &Self) -> f32 {
        // 歐氏距離測量
        // 注意： instant_distance::Point 特徵需要 f32。搜尋器/空間索引在內部使用 f32
        // instant_distance lib 相容。根據具有確定性的權威 Pos 在每次更新時重建緩存
        // 實體 ID 排序；呼叫者的最終距離檢查是固定64。邊界有損是可以接受的。
        let dx = (self.0.x - other.0.x).to_f32_for_render();
        let dy = (self.0.y - other.0.y).to_f32_for_render();
        dx * dx + dy * dy
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Rot(f32);

impl Component for Rot {
    type Storage = VecStorage<Self>;
}

impl Rot {
    pub fn x(&self) -> f32 {
        self.0.cos()
    }
    pub fn y(&self) -> f32 {
        self.0.sin()
    }
}

/// 速度
#[derive(Copy, Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
pub struct Vel(pub SimVec2);

impl Vel {
    pub fn zero() -> Self {
        Vel(SimVec2::ZERO)
    }

    /// 注意：舊版 f32 幫助程式保留為線格式/配置讀取邊界的轉換實用程式。
    #[inline]
    pub fn from_xy_f32(x: f32, y: f32) -> Self {
        Vel(SimVec2 {
            x: Fixed64::from_raw((x * 1024.0) as i64),
            y: Fixed64::from_raw((y * 1024.0) as i64),
        })
    }
}

impl Component for Vel {
    type Storage = VecStorage<Self>;
}

/// 移動目標 — 實體每 tick 向此位置移動
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MoveTarget(pub SimVec2);

impl MoveTarget {
    /// 注意：舊版 f32 幫助程式保留為線格式/配置讀取邊界的轉換實用程式。
    #[inline]
    pub fn from_xy_f32(x: f32, y: f32) -> Self {
        MoveTarget(SimVec2 {
            x: Fixed64::from_raw((x * 1024.0) as i64),
            y: Fixed64::from_raw((y * 1024.0) as i64),
        })
    }
}

impl Component for MoveTarget {
    type Storage = VecStorage<Self>;
}

/// 用於延遲對嵌套連接循環中的​​ Pos/Vel 的寫入
#[derive(Copy, Clone, Debug)]
pub struct PosVelOriDefer {
    pub pos: Option<Pos>,
    pub vel: Option<Vel>,
}

impl Component for PosVelOriDefer {
    type Storage = VecStorage<Self>;
}

/// 速度緩存（最後一個刻度）* dt（當前刻度）
/// 它在物理系統中更新和讀取以加速實體<->實體碰撞
/// 無需透過網路傳送
#[derive(Copy, Clone, Default, Debug, PartialEq)]
pub struct PreviousPhysCache {
    pub velocity_dt: Vec2<f32>,
    /// 沿著實體的路徑包圍實體的邊界球的中心
    /// 這個勾號
    pub center: Vec2<f32>,
    /// 計算實體上的球體以進行快速邊界檢查
    pub collision_boundary: f32,
    pub scale: f32,
    /// 對撞機圓柱體的近似半徑。
    pub scaled_radius: f32,
    /// 對撞機體育場的半徑。
    pub neighborhood_radius: f32,
    /// 對撞機狀態的相對 p0 和 p1，如果是圓柱體，則無。
    pub origins: Option<(Vec2<f32>, Vec2<f32>)>,
    pub pos: Option<Pos>,
}

impl Component for PreviousPhysCache {
    type Storage = VecStorage<Self>;
}

// 規模
#[derive(Copy, Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
pub struct Scale(pub Fixed64);

impl Component for Scale {
    type Storage = FlaggedStorage<Self, VecStorage<Self>>;
}

// 大量的
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mass(pub Fixed64);

impl Default for Mass {
    fn default() -> Mass {
        Mass(Fixed64::ONE)
    }
}

impl Component for Mass {
    type Storage = FlaggedStorage<Self, VecStorage<Self>>;
}

#[derive(Copy, Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sticky;

impl Component for Sticky {
    type Storage = FlaggedStorage<Self, NullStorage<Self>>;
}

#[derive(Copy, Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
pub struct Immovable;

impl Component for Immovable {
    type Storage = FlaggedStorage<Self, NullStorage<Self>>;
}

/// 用於強制更新物體的位置、速度和方向
/// 客戶
#[derive(Copy, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ForceUpdate;

impl Component for ForceUpdate {
    type Storage = NullStorage<Self>;
}

/// 單位的碰撞半徑。用於 BlockedRegions 阻擋判定。
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CollisionRadius(pub Fixed64);

impl Default for CollisionRadius {
    fn default() -> Self {
        CollisionRadius(Fixed64::from_i32(20))
    }
}

impl Component for CollisionRadius {
    type Storage = VecStorage<Self>;
}

/// 單位-單位碰撞查詢時使用的半徑上限（對方半徑上界）。
/// 目前 config 最大為 tower=50；取 80 留空間給未來調整，不必動此常數。
pub const MAX_COLLISION_RADIUS: f32 = 80.0;
