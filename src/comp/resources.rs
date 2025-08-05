use std::{
    i32,
    ops::{Deref, DerefMut},
    sync::Arc,
    time::{Duration, Instant},
};
use serde::{Deserialize, Serialize};
use specs::Entity;

/// A resource that stores the tick (i.e: physics) time.
#[derive(Copy, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Time(pub f64);

/// A resource that stores the time since the previous tick.
#[derive(Copy, Clone, Debug, Default)]
pub struct DeltaTime(pub f32);

// Start of Tick, used for metrics
#[derive(Copy, Clone)]
pub struct TickStart(pub Instant);

#[derive(Copy, Clone, Default)]
pub struct Tick(pub u64);

#[derive(Copy, Clone, Debug, Serialize, Deserialize, Default)]
pub struct TimeOfDay(pub f64);

#[derive(Debug, Copy, Clone, PartialEq, Eq, Deserialize, Hash)]
pub enum DayPeriod {
    Night,
    Morning,
    Noon,
    Evening,
}

impl From<f64> for DayPeriod {
    fn from(time_of_day: f64) -> Self {
        let tod = time_of_day.rem_euclid(60.0 * 60.0 * 24.0);
        if tod < 60.0 * 60.0 * 6.0 {
            DayPeriod::Night
        } else if tod < 60.0 * 60.0 * 11.0 {
            DayPeriod::Morning
        } else if tod < 60.0 * 60.0 * 16.0 {
            DayPeriod::Noon
        } else if tod < 60.0 * 60.0 * 19.0 {
            DayPeriod::Evening
        } else {
            DayPeriod::Night
        }
    }
}

impl DayPeriod {
    pub fn is_dark(&self) -> bool { *self == DayPeriod::Night }

    pub fn is_light(&self) -> bool { !self.is_dark() }
}
