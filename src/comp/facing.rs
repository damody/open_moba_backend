use omoba_sim::{Angle, Fixed64};
use serde::{Deserialize, Serialize};
use specs::storage::VecStorage;
use specs::Component;

/// 當前面向角度（4096-tick `Angle`，0 = +X 方向，CCW 為正）
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct Facing(pub Angle);

impl Facing {
    /// 邊界助手：從弧度 f32 構造。用於腳本/配置/產生邊界。
    /// 注意：傳統的 f32 弧度幫助程式保留為線格式/配置讀取邊界的轉換實用程式。
    #[inline]
    pub fn from_rad_f32(rad: f32) -> Self {
        let ticks =
            (rad / (2.0 * std::f32::consts::PI) * omoba_sim::trig::TAU_TICKS as f32).round() as i32;
        Facing(Angle::from_ticks(ticks))
    }

    /// 邊界助手：有損弧度投影。用於有線格式/戰鬥蜱站點。
    /// 注意：傳統的 f32 弧度投影保留用於線格式/對數邊界； sim-side 原生使用 Angle。
    #[inline]
    pub fn rad_f32(&self) -> f32 {
        (self.0.ticks() as f32 / omoba_sim::trig::TAU_TICKS as f32) * 2.0 * std::f32::consts::PI
    }
}

impl Default for Facing {
    fn default() -> Self {
        Facing(Angle::ZERO)
    }
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

/// 每秒可轉向的最大弧度（rad/s 為單位的 `Fixed64`；tick-rate 換算暫由消費端處理）
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct TurnSpeed(pub Fixed64);

impl Default for TurnSpeed {
    fn default() -> Self {
        // 預設 90°/秒 ≈ 1.5708 rad/s（π/2）→ Fixed64 raw = round(π/2 * 1024) = 1608
        TurnSpeed(Fixed64::from_raw(1608))
    }
}

impl Component for TurnSpeed {
    type Storage = VecStorage<Self>;
}

/// 將角度標準化到 [-π, π]
///
/// **階段 1b.4 棄用說明**：遺留的 f32 弧度助手僅保留用於
/// `hero_tick.rs` / `tower_tick.rs` 直到它們遷移到 `Angle`。新程式碼
/// 必須使用 `omoba_sim::trig::Angle` 算術 + `angle_rotate_toward` 來代替。
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

/// 往 `target` 旋轉至多 `max_step` 弧度，回傳新角度。f32 radians, legacy.
///
/// **階段 1b.4 棄用說明**：僅為 `hero_tick.rs` / 保留 f32 弧度幫助程序
/// `tower_tick.rs` 直到遷移到 `Angle`。新代碼必須使用
/// 改為 `omoba_sim::trig::angle_rotate_toward`。
pub fn rotate_toward(current: f32, target: f32, max_step: f32) -> f32 {
    let diff = normalize_angle(target - current);
    if diff.abs() <= max_step {
        normalize_angle(target)
    } else if diff > 0.0 {
        normalize_angle(current + max_step)
    } else {
        normalize_angle(current - max_step)
    }
}

/// 可移動角度門檻：面向與目標方向夾角 < 30° 才能移動（f32-radian, legacy）
pub const MOVE_ANGLE_THRESHOLD: f32 = std::f32::consts::FRAC_PI_6; // 30° = π/6

/// 相當於“MOVE_ANGLE_THRESHOLD”的角度刻度。 30° = `TAU_TICKS / 12`。
/// 由角度上的刻度系統使用（creep_tick、hero_move_tick）。
pub const MOVE_ANGLE_THRESHOLD_TICKS: i32 = omoba_sim::trig::TAU_TICKS / 12;
