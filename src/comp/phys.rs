use hashbrown::HashSet;
use serde::{Deserialize, Serialize};
use specs::{Component, FlaggedStorage, NullStorage};
use std::sync::Arc;
use vek::*;
use specs::storage::VecStorage;
use instant_distance::{Builder, Search};
use omoba_sim::{Vec2 as SimVec2, Fixed32};

/// Position
#[derive(Copy, Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pos(pub SimVec2);

impl Pos {
    /// Boundary helper: construct from two `f32` (typically world coords from
    /// config / spawn data). Routes through `Fixed32::from_raw` quantization.
    /// NOTE: legacy f32 helper retained as transition utility for wire-format / config read boundary.
    #[inline]
    pub fn from_xy_f32(x: f32, y: f32) -> Self {
        Pos(SimVec2 {
            x: Fixed32::from_raw((x * 1024.0) as i32),
            y: Fixed32::from_raw((y * 1024.0) as i32),
        })
    }

    /// Boundary helper: lossy `f32` projection of underlying coords. Used at
    /// wire-format / VFX / non-determinism-tolerant query sites.
    /// NOTE: legacy f32 projection retained for wire-format / VFX / determinism-tolerant query sites; sim-side reads SimVec2 directly.
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
        // Euclidean distance metric
        // NOTE: instant_distance::Point trait requires f32. Searcher / spatial index uses f32 internally for
        // instant_distance lib compat. Cache rebuilt per tick from authoritative Pos with deterministic
        // entity-id ordering; final distance check in caller is Fixed32. Boundary lossy is acceptable.
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

/// Velocity
#[derive(Copy, Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
pub struct Vel(pub SimVec2);

impl Vel {
    pub fn zero() -> Self { Vel(SimVec2::ZERO) }

    /// NOTE: legacy f32 helper retained as transition utility for wire-format / config read boundary.
    #[inline]
    pub fn from_xy_f32(x: f32, y: f32) -> Self {
        Vel(SimVec2 {
            x: Fixed32::from_raw((x * 1024.0) as i32),
            y: Fixed32::from_raw((y * 1024.0) as i32),
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
    /// NOTE: legacy f32 helper retained as transition utility for wire-format / config read boundary.
    #[inline]
    pub fn from_xy_f32(x: f32, y: f32) -> Self {
        MoveTarget(SimVec2 {
            x: Fixed32::from_raw((x * 1024.0) as i32),
            y: Fixed32::from_raw((y * 1024.0) as i32),
        })
    }
}

impl Component for MoveTarget {
    type Storage = VecStorage<Self>;
}

/// Used to defer writes to Pos/Vel in nested join loops
#[derive(Copy, Clone, Debug)]
pub struct PosVelOriDefer {
    pub pos: Option<Pos>,
    pub vel: Option<Vel>,
}

impl Component for PosVelOriDefer {
    type Storage = VecStorage<Self>;
}

/// Cache of Velocity (of last tick) * dt (of curent tick)
/// It's updated and read in physics sys to speed up entity<->entity collisions
/// no need to send it via network
#[derive(Copy, Clone, Default, Debug, PartialEq)]
pub struct PreviousPhysCache {
    pub velocity_dt: Vec2<f32>,
    /// Center of bounding sphere that encompasses the entity along its path for
    /// this tick
    pub center: Vec2<f32>,
    /// Calculates a Sphere over the Entity for quick boundary checking
    pub collision_boundary: f32,
    pub scale: f32,
    /// Approximate radius of cylinder of collider.
    pub scaled_radius: f32,
    /// Radius of stadium of collider.
    pub neighborhood_radius: f32,
    /// relative p0 and p1 of collider's statium, None if cylinder.
    pub origins: Option<(Vec2<f32>, Vec2<f32>)>,
    pub pos: Option<Pos>,
}

impl Component for PreviousPhysCache {
    type Storage = VecStorage<Self>;
}

// Scale
#[derive(Copy, Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
pub struct Scale(pub Fixed32);

impl Component for Scale {
    type Storage = FlaggedStorage<Self, VecStorage<Self>>;
}

// Mass
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mass(pub Fixed32);

impl Default for Mass {
    fn default() -> Mass { Mass(Fixed32::ONE) }
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

/// Used to forcefully update the position, velocity, and orientation of the
/// client
#[derive(Copy, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ForceUpdate;

impl Component for ForceUpdate {
    type Storage = NullStorage<Self>;
}

/// 單位的碰撞半徑。用於 BlockedRegions 阻擋判定。
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CollisionRadius(pub Fixed32);

impl Default for CollisionRadius {
    fn default() -> Self { CollisionRadius(Fixed32::from_i32(20)) }
}

impl Component for CollisionRadius {
    type Storage = VecStorage<Self>;
}

/// 單位-單位碰撞查詢時使用的半徑上限（對方半徑上界）。
/// 目前 config 最大為 tower=50；取 80 留空間給未來調整，不必動此常數。
pub const MAX_COLLISION_RADIUS: f32 = 80.0;
