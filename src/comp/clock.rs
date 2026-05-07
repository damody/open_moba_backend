use crate::comp::span;
use ordered_float::NotNan;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};
use log::info;

/// 這個時鐘試圖透過休眠其餘的時間來使這個滴答聲保持恆定的時間
/// 蜱蟲
/// - 如果我們實際花費的時間比計劃的少：按計劃睡覺和返回
/// 時間
/// - 如果我們落後了：不要睡覺並返回實際時間
/// 我們不會對逐筆變動的增量進行任何花俏的平均，原因有二：
/// - 所有系統都必須基於「dt」工作，我們不能假設這是
/// 所有刻度都為常數
/// - 當我們有一個緩慢的滴答聲、一個滯後時，我們有 10 個快速的滴答聲並沒有幫助
/// 之後直接打勾
/// 我們返回平滑版本僅供顯示！
pub struct Clock {
    /// 這是時鐘嘗試在每次呼叫 tick 時存檔的 dt。
    target_dt: Duration,
    /// 上次調用“tick”的時間
    last_sys_time: Instant,
    /// 將在 `tick` 中計算傳回下次迭代使用的 dt
    /// 主循環的
    last_dt: Duration,
    /// 總結`last_dt`
    total_tick_time: Duration,
    target_total_tick_time: Duration,
    // 僅統計數據
    // 使用 f32，因此我們有足夠的精度來顯示 fps 值，同時節省空間
    // 這是以秒為單位的
    last_dts: VecDeque<NotNan<f32>>,
    last_dts_sorted: Vec<NotNan<f32>>,
    last_busy_dts: VecDeque<NotNan<f32>>,
    stats: ClockStats,
}

pub struct ClockStats {
    /// Busy dt是我們沒有睡覺的tick部分。
    /// 例如總時間為 33 毫秒，其中包括 25 毫秒休眠時間。然後這返回
    /// 8ms
    /// 這是以秒為單位的
    pub average_busy_dt: Duration,
    /// 過去 NUMBER_OF_OLD_DELTAS_KEPT 個刻度的平均值
    pub average_tps: f64,
    /// = 50% 百分位
    pub median_tps: f64,
    /// 最低 10% 的幀
    pub percentile_90_tps: f64,
    /// 最低 5% 的幀
    pub percentile_95_tps: f64,
    /// 最低 1% 的幀
    pub percentile_99_tps: f64,
}

const NUMBER_OF_OLD_DELTAS_KEPT: usize = 100;
const NUMBER_OF_DELTAS_COMPARED: usize = 5;

impl Clock {
    pub fn new(target_dt: Duration) -> Self {
        Self {
            target_dt,
            last_sys_time: Instant::now(),
            last_dt: target_dt,
            target_total_tick_time: Duration::default(),
            total_tick_time: Duration::default(),
            last_dts: VecDeque::with_capacity(NUMBER_OF_OLD_DELTAS_KEPT),
            last_dts_sorted: Vec::with_capacity(NUMBER_OF_OLD_DELTAS_KEPT),
            last_busy_dts: VecDeque::with_capacity(NUMBER_OF_OLD_DELTAS_KEPT),
            stats: ClockStats::new(&[], &VecDeque::new()),
        }
    }

    pub fn set_target_dt(&mut self, target_dt: Duration) { self.target_dt = target_dt; }

    pub fn stats(&self) -> &ClockStats { &self.stats }

    pub fn dt(&self) -> Duration { self.last_dt }

    pub fn get_stable_dt(&self) -> Duration {
        let stable_dt = Duration::from_secs_f32(
            self.last_dts
                .iter()
                .skip(self.last_dts.len() - NUMBER_OF_DELTAS_COMPARED)
                .min()
                .map_or(self.last_dt.as_secs_f32(), |t| t.into_inner()),
        );
        if self.last_dts.len() >= NUMBER_OF_DELTAS_COMPARED && self.last_dt > 2 * stable_dt {
            tracing::trace!(?self.last_dt, ?self.total_tick_time, "lag spike detected, unusually slow tick");
            stable_dt
        } else {
            self.last_dt
        }
    }

    /// 未經先詢問@xMAC94x，請勿修改！
    pub fn tick(&mut self) {
        span!(_guard, "tick", "Clock::tick");
        span!(guard, "clock work");
        let current_sys_time = Instant::now();
        let estimated_time = self.last_sys_time.checked_add(self.target_dt).unwrap();
        let mut  busy_delta = current_sys_time.duration_since(self.last_sys_time);
        let busy_delta2 = self.total_tick_time.checked_sub(self.target_total_tick_time);
        if let Some(busy_delta2) = busy_delta2 {
            busy_delta = busy_delta.checked_add(busy_delta2).unwrap();
        }
        // 維持TPS
        self.last_dts_sorted = self.last_dts.iter().copied().collect();
        self.last_dts_sorted.sort_unstable();
        self.stats = ClockStats::new(&self.last_dts_sorted, &self.last_busy_dts);
        drop(guard);
        // 嘗試睡覺來填補空白。
        if let Some(sleep_dur) = self.target_dt.checked_sub(busy_delta) {
            //log::info!("busy_delta {:?}", busy_delta);
            spin_sleep::sleep(sleep_dur);
        }

        let after_sleep_sys_time = Instant::now();
        self.last_dt = after_sleep_sys_time.duration_since(self.last_sys_time);
        if self.last_dts.len() >= NUMBER_OF_OLD_DELTAS_KEPT {
            self.last_dts.pop_front();
        }
        if self.last_busy_dts.len() >= NUMBER_OF_OLD_DELTAS_KEPT {
            self.last_busy_dts.pop_front();
        }
        self.last_dts.push_back(
            NotNan::new(self.last_dt.as_secs_f32())
                .expect("Duration::as_secs_f32 never returns NaN"),
        );
        self.last_busy_dts.push_back(
            NotNan::new(busy_delta.as_secs_f32()).expect("Duration::as_secs_f32 never returns NaN"),
        );
        self.total_tick_time += self.last_dt;
        self.target_total_tick_time += self.target_dt;
        self.last_sys_time = after_sleep_sys_time;
    }
}

impl ClockStats {
    fn new(sorted: &[NotNan<f32>], busy_dt_list: &VecDeque<NotNan<f32>>) -> Self {
        let average_frame_time =
            sorted.iter().sum::<NotNan<f32>>().into_inner() / sorted.len().max(1) as f32;

        let average_busy_dt = busy_dt_list.iter().sum::<NotNan<f32>>().into_inner()
            / busy_dt_list.len().max(1) as f32;

        let average_tps = 1.0 / average_frame_time as f64;
        let (median_tps, percentile_90_tps, percentile_95_tps, percentile_99_tps) =
            if sorted.len() >= NUMBER_OF_OLD_DELTAS_KEPT {
                let median_frame_time = *sorted[sorted.len() / 2];
                let percentile_90_frame_time =
                    *sorted[(NUMBER_OF_OLD_DELTAS_KEPT as f32 * 0.1) as usize];
                let percentile_95_frame_time =
                    *sorted[(NUMBER_OF_OLD_DELTAS_KEPT as f32 * 0.05) as usize];
                let percentile_99_frame_time =
                    *sorted[(NUMBER_OF_OLD_DELTAS_KEPT as f32 * 0.01) as usize];

                let median_tps = 1.0 / median_frame_time as f64;
                let percentile_90_tps = 1.0 / percentile_90_frame_time as f64;
                let percentile_95_tps = 1.0 / percentile_95_frame_time as f64;
                let percentile_99_tps = 1.0 / percentile_99_frame_time as f64;
                (
                    median_tps,
                    percentile_90_tps,
                    percentile_95_tps,
                    percentile_99_tps,
                )
            } else {
                let avg_tps = 1.0 / average_busy_dt as f64;
                (avg_tps, avg_tps, avg_tps, avg_tps)
            };

        Self {
            average_busy_dt: Duration::from_secs_f32(average_busy_dt),
            average_tps,
            median_tps,
            percentile_90_tps,
            percentile_95_tps,
            percentile_99_tps,
        }
    }
}
