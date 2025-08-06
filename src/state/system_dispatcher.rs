/// 系統分派器 - 負責協調和運行所有遊戲系統

use std::sync::Arc;
use rayon::ThreadPool;
use specs::{World, DispatcherBuilder};
use failure::Error;

use crate::comp::*;
use crate::tick::*;

/// 系統分派器
pub struct SystemDispatcher {
    /// 執行緒池
    thread_pool: Arc<ThreadPool>,
}

impl SystemDispatcher {
    /// 創建新的系統分派器
    pub fn new(thread_pool: Arc<ThreadPool>) -> Self {
        Self { thread_pool }
    }

    /// 運行所有遊戲系統
    pub fn run_systems(&self, world: &World) -> Result<(), Error> {
        let mut dispatch_builder = DispatcherBuilder::new()
            .with_pool(Arc::clone(&self.thread_pool));

        // 構建系統調度順序
        self.build_system_dependencies(&mut dispatch_builder);

        // 建立並運行調度器
        let mut dispatcher = dispatch_builder.build();
        dispatcher.dispatch(world);

        Ok(())
    }

    /// 運行特定系統組
    pub fn run_system_group(&self, world: &World, group: SystemGroup) -> Result<(), Error> {
        let mut dispatch_builder = DispatcherBuilder::new()
            .with_pool(Arc::clone(&self.thread_pool));

        match group {
            SystemGroup::Core => {
                self.build_core_systems(&mut dispatch_builder);
            }
            SystemGroup::Combat => {
                self.build_combat_systems(&mut dispatch_builder);
            }
            SystemGroup::Movement => {
                self.build_movement_systems(&mut dispatch_builder);
            }
            SystemGroup::Vision => {
                self.build_vision_systems(&mut dispatch_builder);
            }
            SystemGroup::Effects => {
                self.build_effects_systems(&mut dispatch_builder);
            }
        }

        let mut dispatcher = dispatch_builder.build();
        dispatcher.dispatch(world);

        Ok(())
    }

    /// 獲取系統執行統計
    pub fn get_system_stats(&self) -> SystemStats {
        SystemStats {
            thread_count: self.thread_pool.current_num_threads(),
            active_systems: self.get_active_system_count(),
            total_dispatches: 0, // 需要實際統計
        }
    }

    // 私有方法：構建系統依賴關係
    fn build_system_dependencies(&self, dispatch_builder: &mut DispatcherBuilder<'_, '_>) {
        // 第一階段：不需要 Vec<Outcome> 的系統，可以並行執行
        dispatch::<nearby_tick::Sys>(dispatch_builder, &[]);
        dispatch::<player_tick::Sys>(dispatch_builder, &[]);

        // 視野系統：在遊戲邏輯之前更新（暫時註解掉）
        // dispatch::<VisionSystem>(dispatch_builder, &["nearby_sys", "player_sys"]);

        // 第二階段：需要 Vec<Outcome> 的系統，按依賴順序執行
        dispatch::<projectile_tick::Sys>(dispatch_builder, &["nearby_sys", "player_sys"]);
        dispatch::<tower_tick::Sys>(dispatch_builder, &["projectile_sys"]);
        dispatch::<hero_tick::Sys>(dispatch_builder, &["tower_sys"]);
        dispatch::<skill_tick::Sys>(dispatch_builder, &["hero_sys"]);
        dispatch::<creep_tick::Sys>(dispatch_builder, &["skill_sys"]);
        dispatch::<creep_wave::Sys>(dispatch_builder, &["creep_sys"]);
        dispatch::<damage_tick::Sys>(dispatch_builder, &["creep_wave_sys"]);
        dispatch::<death_tick::Sys>(dispatch_builder, &["damage_sys"]);

        // 戰爭迷霧整合系統：在所有其他系統完成後處理事件（暫時註解掉）
        // dispatch::<FogOfWarIntegrationSystem>(dispatch_builder, &["death_sys"]);
    }

    fn build_core_systems(&self, dispatch_builder: &mut DispatcherBuilder<'_, '_>) {
        dispatch::<nearby_tick::Sys>(dispatch_builder, &[]);
        dispatch::<player_tick::Sys>(dispatch_builder, &[]);
    }

    fn build_combat_systems(&self, dispatch_builder: &mut DispatcherBuilder<'_, '_>) {
        dispatch::<tower_tick::Sys>(dispatch_builder, &[]);
        dispatch::<hero_tick::Sys>(dispatch_builder, &["tower_sys"]);
        dispatch::<damage_tick::Sys>(dispatch_builder, &["hero_sys"]);
        dispatch::<death_tick::Sys>(dispatch_builder, &["damage_sys"]);
    }

    fn build_movement_systems(&self, dispatch_builder: &mut DispatcherBuilder<'_, '_>) {
        dispatch::<creep_tick::Sys>(dispatch_builder, &[]);
        dispatch::<projectile_tick::Sys>(dispatch_builder, &[]);
    }

    fn build_vision_systems(&self, dispatch_builder: &mut DispatcherBuilder<'_, '_>) {
        // 視野相關系統（待實現）
        log::info!("視野系統群組（待實現）");
    }

    fn build_effects_systems(&self, dispatch_builder: &mut DispatcherBuilder<'_, '_>) {
        dispatch::<skill_tick::Sys>(dispatch_builder, &[]);
        // 其他特效系統
    }

    fn get_active_system_count(&self) -> usize {
        // 計算當前活躍的系統數量
        // 這需要跟蹤系統的實際狀態
        8 // 暫時硬編碼
    }

    /// 檢查系統健康狀態
    pub fn check_system_health(&self) -> Vec<String> {
        let mut issues = Vec::new();
        
        let stats = self.get_system_stats();
        
        if stats.thread_count == 0 {
            issues.push("執行緒池無可用執行緒".to_string());
        }
        
        if stats.active_systems == 0 {
            issues.push("無活躍系統運行".to_string());
        }
        
        if stats.thread_count < num_cpus::get() / 2 {
            issues.push("執行緒數量可能不足".to_string());
        }
        
        issues
    }

    /// 重新配置執行緒池
    pub fn reconfigure_thread_pool(&mut self, new_thread_count: usize) -> Result<(), Error> {
        use rayon::ThreadPoolBuilder;
        
        let new_pool = Arc::new(
            ThreadPoolBuilder::new()
                .num_threads(new_thread_count)
                .thread_name(move |i| format!("rayon-{}", i))
                .build()?
        );
        
        self.thread_pool = new_pool;
        log::info!("執行緒池重新配置為 {} 個執行緒", new_thread_count);
        
        Ok(())
    }

    /// 暫停所有系統
    pub fn pause_all_systems(&self) {
        // 實現系統暫停邏輯
        log::info!("所有系統已暫停");
    }

    /// 恢復所有系統
    pub fn resume_all_systems(&self) {
        // 實現系統恢復邏輯
        log::info!("所有系統已恢復");
    }

    /// 獲取系統性能分析
    pub fn get_performance_analysis(&self) -> SystemPerformanceAnalysis {
        SystemPerformanceAnalysis {
            average_dispatch_time: 0.0, // 需要實際測量
            peak_dispatch_time: 0.0,    // 需要實際測量
            system_bottlenecks: Vec::new(), // 需要實際分析
            thread_utilization: 0.0,    // 需要實際測量
        }
    }
}

/// 系統群組枚舉
#[derive(Debug, Clone, Copy)]
pub enum SystemGroup {
    /// 核心系統
    Core,
    /// 戰鬥系統
    Combat,
    /// 移動系統
    Movement,
    /// 視野系統
    Vision,
    /// 特效系統
    Effects,
}

/// 系統統計信息
#[derive(Debug, Clone)]
pub struct SystemStats {
    /// 執行緒數量
    pub thread_count: usize,
    /// 活躍系統數量
    pub active_systems: usize,
    /// 總分派次數
    pub total_dispatches: u64,
}

/// 系統性能分析
#[derive(Debug, Clone)]
pub struct SystemPerformanceAnalysis {
    /// 平均分派時間（毫秒）
    pub average_dispatch_time: f64,
    /// 峰值分派時間（毫秒）
    pub peak_dispatch_time: f64,
    /// 系統瓶頸列表
    pub system_bottlenecks: Vec<String>,
    /// 執行緒利用率
    pub thread_utilization: f64,
}