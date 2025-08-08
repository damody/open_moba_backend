/// 時間管理器 - 負責遊戲時間循環和日夜週期

use std::time::Duration;
use specs::{World, WorldExt};
use failure::Error;

use crate::comp::{TimeOfDay, Time, DeltaTime, DayPeriod};

/// 時間管理器
pub struct TimeManager {
    /// 日夜循環倍率
    day_cycle_factor: f64,
    /// 最大增量時間
    max_delta_time: f32,
}

impl TimeManager {
    /// 創建新的時間管理器
    pub fn new() -> Self {
        Self {
            day_cycle_factor: 24.0, // 預設日夜循環倍率
            max_delta_time: 1.0,    // 預設最大增量時間
        }
    }

    /// 使用自定義配置創建時間管理器
    pub fn with_config(day_cycle_factor: f64, max_delta_time: f32) -> Self {
        Self {
            day_cycle_factor,
            max_delta_time,
        }
    }

    /// 更新時間系統
    pub fn update(&self, world: &mut World, dt: Duration) -> Result<(), Error> {
        // 更新基本時間
        {
            let mut time_of_day = world.write_resource::<TimeOfDay>();
            time_of_day.0 += dt.as_secs_f64() * self.day_cycle_factor;
        }

        {
            let mut time = world.write_resource::<Time>();
            time.0 += dt.as_secs_f64();
        }

        {
            let mut delta_time = world.write_resource::<DeltaTime>();
            delta_time.0 = dt.as_secs_f32().min(self.max_delta_time);
        }

        Ok(())
    }

    /// 獲取當前一天中的時間
    pub fn get_time_of_day(&self) -> f64 {
        // 這個方法需要 World 才能獲取實際值
        // 在實際使用中應該從 State 調用
        0.0
    }

    /// 獲取當前遊戲時間
    pub fn get_time(&self) -> f64 {
        // 這個方法需要 World 才能獲取實際值
        // 在實際使用中應該從 State 調用
        0.0
    }

    /// 獲取當前增量時間
    pub fn get_delta_time(&self) -> f32 {
        // 這個方法需要 World 才能獲取實際值
        // 在實際使用中應該從 State 調用
        0.0
    }

    /// 獲取當前日期週期
    pub fn get_day_period(&self) -> DayPeriod {
        // 這個方法需要 World 才能獲取實際值
        // 在實際使用中應該從 State 調用
        DayPeriod::Noon // 預設返回中午
    }

    /// 設置日夜循環倍率
    pub fn set_day_cycle_factor(&mut self, factor: f64) {
        self.day_cycle_factor = factor;
        log::info!("日夜循環倍率設置為: {}", factor);
    }

    /// 設置最大增量時間
    pub fn set_max_delta_time(&mut self, max_dt: f32) {
        self.max_delta_time = max_dt;
        log::info!("最大增量時間設置為: {}", max_dt);
    }

    /// 獲取日夜循環倍率
    pub fn get_day_cycle_factor(&self) -> f64 {
        self.day_cycle_factor
    }

    /// 獲取最大增量時間
    pub fn get_max_delta_time(&self) -> f32 {
        self.max_delta_time
    }

    /// 暫停時間（設置增量時間為0）
    pub fn pause_time(&self, world: &mut World) {
        let mut delta_time = world.write_resource::<DeltaTime>();
        delta_time.0 = 0.0;
        log::info!("遊戲時間已暫停");
    }

    /// 恢復時間
    pub fn resume_time(&self, world: &mut World, dt: Duration) {
        let mut delta_time = world.write_resource::<DeltaTime>();
        delta_time.0 = dt.as_secs_f32().min(self.max_delta_time);
        log::info!("遊戲時間已恢復");
    }

    /// 加速時間
    pub fn accelerate_time(&mut self, multiplier: f64) {
        self.day_cycle_factor *= multiplier;
        log::info!("時間加速，新倍率: {}", self.day_cycle_factor);
    }

    /// 重置時間加速
    pub fn reset_time_acceleration(&mut self) {
        self.day_cycle_factor = 24.0;
        log::info!("時間加速已重置");
    }

    /// 檢查是否為白天
    pub fn is_day_time(world: &World) -> bool {
        let time_of_day = world.read_resource::<TimeOfDay>().0;
        let day_period: DayPeriod = time_of_day.into();
        matches!(day_period, DayPeriod::Noon | DayPeriod::Morning)
    }

    /// 檢查是否為夜晚
    pub fn is_night_time(world: &World) -> bool {
        let time_of_day = world.read_resource::<TimeOfDay>().0;
        let day_period: DayPeriod = time_of_day.into();
        matches!(day_period, DayPeriod::Night)
    }

    /// 獲取光照強度（基於時間）
    pub fn get_light_intensity(world: &World) -> f32 {
        let time_of_day = world.read_resource::<TimeOfDay>().0;
        let day_period: DayPeriod = time_of_day.into();
        
        match day_period {
            DayPeriod::Night => 0.2,        // 夜晚低光照
            DayPeriod::Morning => 0.6,      // 早晨中等光照
            DayPeriod::Noon => 1.0,         // 中午全光照
            DayPeriod::Evening => 0.7,      // 働晚中高光照
        }
    }

    /// 計算光照對視野的影響
    pub fn get_vision_modifier(world: &World) -> f32 {
        let light_intensity = Self::get_light_intensity(world);
        
        // 光照越低，視野範圍越小
        if light_intensity >= 0.8 {
            1.0         // 白天全視野
        } else if light_intensity >= 0.5 {
            0.85        // 黃昏/黎明減少 15%
        } else {
            0.6         // 夜晚減少 40%
        }
    }

    /// 獲取時間統計信息
    pub fn get_time_stats(world: &World) -> TimeStats {
        let time_of_day = world.read_resource::<TimeOfDay>().0;
        let total_time = world.read_resource::<Time>().0;
        let delta_time = world.read_resource::<DeltaTime>().0;
        
        TimeStats {
            time_of_day,
            total_game_time: total_time,
            current_delta_time: delta_time,
            day_period: time_of_day.into(),
            light_intensity: Self::get_light_intensity(world),
            vision_modifier: Self::get_vision_modifier(world),
        }
    }
}

impl Default for TimeManager {
    fn default() -> Self {
        Self::new()
    }
}

/// 時間統計信息
#[derive(Debug, Clone)]
pub struct TimeStats {
    /// 一天中的時間
    pub time_of_day: f64,
    /// 總遊戲時間
    pub total_game_time: f64,
    /// 當前增量時間
    pub current_delta_time: f32,
    /// 日期週期
    pub day_period: DayPeriod,
    /// 光照強度
    pub light_intensity: f32,
    /// 視野修正值
    pub vision_modifier: f32,
}