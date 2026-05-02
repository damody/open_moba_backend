use specs::storage::VecStorage;
use specs::Component;
use serde::{Deserialize, Serialize};
use omoba_sim::{Fixed32, Angle};

/// 當前面向角度（4096-tick `Angle`，0 = +X 方向，CCW 為正）
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct Facing(pub Angle);

impl Default for Facing {
    fn default() -> Self { Facing(Angle::ZERO) }
}

impl Component for Facing {
    type Storage = VecStorage<Self>;
}

/// 上次廣播給 client 的 facing 值。`None` = 從未廣播（第一次必發）。
///
/// **為什麼需要這個 component**：原本 `creep/hero/tower_tick` 用「這 tick 之前的
/// `old_facing`」做門檻比較：`(facing - old_facing).abs() > threshold`。但每 tick
/// 旋轉量 ≤ `turn_speed × dt` ≈ `π/2 × 1/30` ≈ 3°，永遠小於 15° 門檻，**廣播從未
/// 觸發**。修正：累計差距以「自上次廣播以來」計算，須要單獨儲存上次廣播值。
///
/// **Phase 1b 註**：暫時保留 `Option<f32>`（delta-broadcast hint），會在廣播層
/// 改寫時一併遷移。
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
pub struct FacingBroadcast(pub Option<f32>);

impl Component for FacingBroadcast {
    type Storage = VecStorage<Self>;
}

/// 每秒可轉向的最大弧度（rad/s 為單位的 `Fixed32`；tick-rate 換算暫由消費端處理）
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct TurnSpeed(pub Fixed32);

impl Default for TurnSpeed {
    fn default() -> Self {
        // 預設 90°/秒 ≈ 1.5708 rad/s（π/2）→ Fixed32 raw = round(π/2 * 1024) = 1608
        TurnSpeed(Fixed32::from_raw(1608))
    }
}

impl Component for TurnSpeed {
    type Storage = VecStorage<Self>;
}

/// 將角度標準化到 [-π, π]
///
/// **Phase 1b.4 deprecation note**: legacy f32-radian helper kept only for
/// `hero_tick.rs` / `tower_tick.rs` until those migrate to `Angle`. New code
/// must use `omoba_sim::trig::Angle` arithmetic + `angle_rotate_toward` instead.
pub fn normalize_angle(a: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    let mut a = a % tau;
    if a > std::f32::consts::PI {
        a -= tau;
    } else if a < -std::f32::consts::PI {
        a += tau;
    }
    a
}

/// 可移動角度門檻：面向與目標方向夾角 < 30° 才能移動（f32-radian, legacy）
pub const MOVE_ANGLE_THRESHOLD: f32 = std::f32::consts::FRAC_PI_6; // 30° = π/6

/// Angle-tick equivalent of `MOVE_ANGLE_THRESHOLD`. 30° = `TAU_TICKS / 12`.
/// Used by tick systems that are on Angle (creep_tick, hero_move_tick).
pub const MOVE_ANGLE_THRESHOLD_TICKS: i32 = omoba_sim::trig::TAU_TICKS / 12;
