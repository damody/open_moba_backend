use hashbrown::HashSet;
use serde_derive::{Deserialize, Serialize};
use specs::{Component, FlaggedStorage, NullStorage};
use std::sync::Arc;
use vek::*;
use specs::storage::VecStorage;
use instant_distance::{Builder, Search};

/// Position
#[derive(Copy, Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pos(pub Vec2<f32>);

impl Component for Pos {
    type Storage = VecStorage<Self>;
}

impl instant_distance::Point for Pos {
    fn distance(&self, other: &Self) -> f32 {
        // Euclidean distance metric
        let r = self.0 - other.0;
        r.magnitude_squared()
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
pub struct Vel(pub Vec2<f32>);

impl Vel {
    pub fn zero() -> Self { Vel(Vec2::zero()) }
}

impl Component for Vel {
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
pub struct Scale(pub f32);

impl Component for Scale {
    type Storage = FlaggedStorage<Self, VecStorage<Self>>;
}

// Mass
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mass(pub f32);

impl Default for Mass {
    fn default() -> Mass { Mass(1.0) }
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
