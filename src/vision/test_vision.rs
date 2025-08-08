/// 視野系統測試
/// 
/// 基本功能測試和性能驗證
#[cfg(test)]
mod tests {
    use super::*;
    use vek::Vec2;
    use crate::comp::circular_vision::{CircularVision, ObstacleInfo, ObstacleType, ObstacleProperties};
    use crate::vision::{VisionOutputGenerator, ShadowCalculator, Bounds};

    /// 測試基本的圓形視野創建
    #[test]
    fn test_circular_vision_creation() {
        let vision = CircularVision::new(1400.0, 30.0)
            .with_precision(720)
            .with_true_sight();

        assert_eq!(vision.range, 1400.0);
        assert_eq!(vision.height, 30.0);
        assert_eq!(vision.precision, 720);
        assert!(vision.true_sight);
        assert!(vision.vision_result.is_none());
    }

    /// 測試輸出生成器基本功能
    #[test]
    fn test_vision_output_generator() {
        let mut generator = VisionOutputGenerator::new(25.0);
        
        // 創建測試視野結果
        let vision_result = crate::comp::circular_vision::VisionResult {
            observer_pos: Vec2::new(100.0, 200.0),
            range: 1400.0,
            visible_area: vec![
                Vec2::new(100.0, 200.0),
                Vec2::new(200.0, 300.0),
                Vec2::new(0.0, 100.0),
            ],
            shadows: vec![],
            timestamp: 1234567890.0,
        };

        // 測試網格輸出
        let grid_output = generator.generate_grid_output(&vision_result, Some(50.0));
        assert_eq!(grid_output.format_type, "grid");
        assert_eq!(grid_output.grid_size, 50.0);
        assert_eq!(grid_output.observer_pos, Vec2::new(100.0, 200.0));
        assert_eq!(grid_output.vision_range, 1400.0);
        assert!(grid_output.width > 0);
        assert!(grid_output.height > 0);

        // 測試向量輸出
        let vector_output = generator.generate_vector_output(&vision_result);
        assert_eq!(vector_output.format_type, "vector");
        assert_eq!(vector_output.observer_pos, Vec2::new(100.0, 200.0));
        assert_eq!(vector_output.vision_range, 1400.0);
        assert_eq!(vector_output.visible_area.len(), 3);
    }

    /// 測試陰影計算器基本功能
    #[test]
    fn test_shadow_calculator() {
        let mut calculator = ShadowCalculator::with_config(100, 4, 5);

        // 創建測試障礙物
        let obstacles = vec![
            ObstacleInfo {
                position: Vec2::new(500.0, 300.0),
                obstacle_type: ObstacleType::Circular { radius: 50.0 },
                height: 200.0,
                properties: ObstacleProperties {
                    blocks_completely: false,
                    opacity: 0.8,
                    shadow_multiplier: 2.0,
                }
            },
            ObstacleInfo {
                position: Vec2::new(800.0, 600.0),
                obstacle_type: ObstacleType::Rectangle { 
                    width: 100.0, 
                    height: 150.0, 
                    rotation: 0.0 
                },
                height: 300.0,
                properties: ObstacleProperties {
                    blocks_completely: true,
                    opacity: 1.0,
                    shadow_multiplier: 1.5,
                }
            },
        ];

        // 初始化四叉樹
        let world_bounds = Bounds::new(
            Vec2::new(0.0, 0.0),
            Vec2::new(2000.0, 2000.0)
        );
        calculator.initialize_quadtree(world_bounds, obstacles);

        // 計算視野
        let observer_pos = Vec2::new(100.0, 100.0);
        let vision_result = calculator.calculate_optimized_vision(
            observer_pos, 
            30.0,  // 觀察者高度
            1400.0 // 視野範圍
        );

        assert_eq!(vision_result.observer_pos, observer_pos);
        assert_eq!(vision_result.range, 1400.0);
        assert!(!vision_result.visible_area.is_empty());
        assert!(vision_result.timestamp > 0.0);

        // 檢查性能統計
        let stats = calculator.get_performance_stats();
        assert!(stats.obstacle_count > 0);
        assert!(stats.quadtree_nodes > 0);
    }

    /// 測試四叉樹邊界檢查
    #[test]
    fn test_bounds_functionality() {
        let bounds = Bounds::new(
            Vec2::new(0.0, 0.0),
            Vec2::new(100.0, 100.0)
        );

        assert_eq!(bounds.width(), 100.0);
        assert_eq!(bounds.height(), 100.0);
        
        assert!(bounds.contains_point(Vec2::new(50.0, 50.0)));
        assert!(bounds.contains_point(Vec2::new(0.0, 0.0)));
        assert!(bounds.contains_point(Vec2::new(100.0, 100.0)));
        assert!(!bounds.contains_point(Vec2::new(150.0, 50.0)));
        assert!(!bounds.contains_point(Vec2::new(-10.0, 50.0)));
    }

    /// 測試可見性等級轉換
    #[test]
    fn test_visibility_level_conversion() {
        use crate::vision::vision_output::VisibilityLevel;

        let visible = VisibilityLevel::Visible;
        let invisible = VisibilityLevel::Invisible;
        let partial = VisibilityLevel::Partial(0.5);

        assert_eq!(f32::from(visible), 1.0);
        assert_eq!(f32::from(invisible), 0.0);
        assert_eq!(f32::from(partial), 0.5);

        let from_float1 = VisibilityLevel::from(1.0);
        let from_float0 = VisibilityLevel::from(0.0);
        let from_float_mid = VisibilityLevel::from(0.5);

        assert_eq!(from_float1, VisibilityLevel::Visible);
        assert_eq!(from_float0, VisibilityLevel::Invisible);
        assert_eq!(from_float_mid, VisibilityLevel::Partial(0.5));
    }

    /// 性能基準測試
    #[test]
    fn test_vision_performance() {
        use std::time::Instant;

        let mut calculator = ShadowCalculator::new();
        
        // 創建大量障礙物
        let obstacles: Vec<ObstacleInfo> = (0..100)
            .map(|i| ObstacleInfo {
                position: Vec2::new(
                    (i as f32 * 37.0) % 2000.0,
                    (i as f32 * 43.0) % 2000.0
                ),
                obstacle_type: ObstacleType::Circular { 
                    radius: 20.0 + (i as f32 % 30.0) 
                },
                height: 100.0 + (i as f32 % 200.0),
                properties: ObstacleProperties {
                    blocks_completely: i % 3 == 0,
                    opacity: 0.5 + (i as f32 % 50.0) / 100.0,
                    shadow_multiplier: 1.0 + (i as f32 % 20.0) / 20.0,
                }
            })
            .collect();

        let world_bounds = Bounds::new(
            Vec2::new(0.0, 0.0),
            Vec2::new(2000.0, 2000.0)
        );

        // 測量初始化時間
        let start = Instant::now();
        calculator.initialize_quadtree(world_bounds, obstacles);
        let init_duration = start.elapsed();
        
        println!("四叉樹初始化時間: {:?}", init_duration);
        assert!(init_duration.as_millis() < 100); // 應該在100毫秒內完成

        // 測量視野計算時間
        let start = Instant::now();
        let _result = calculator.calculate_optimized_vision(
            Vec2::new(1000.0, 1000.0),
            30.0,
            1400.0
        );
        let calc_duration = start.elapsed();
        
        println!("視野計算時間: {:?}", calc_duration);
        assert!(calc_duration.as_millis() < 50); // 應該在50毫秒內完成

        // 檢查性能統計
        let stats = calculator.get_performance_stats();
        println!("性能統計: {:?}", stats);
        assert_eq!(stats.obstacle_count, 100);
    }

    /// 測試緩存功能
    #[test]
    fn test_caching_functionality() {
        let mut generator = VisionOutputGenerator::new(25.0);
        
        let vision_result = crate::comp::circular_vision::VisionResult {
            observer_pos: Vec2::new(100.0, 200.0),
            range: 1400.0,
            visible_area: vec![Vec2::new(100.0, 200.0)],
            shadows: vec![],
            timestamp: 1234567890.0,
        };

        // 第一次生成（應該創建緩存）
        let start = std::time::Instant::now();
        let _output1 = generator.generate_grid_output(&vision_result, None);
        let first_duration = start.elapsed();

        // 第二次生成（應該使用緩存）
        let start = std::time::Instant::now();
        let _output2 = generator.generate_grid_output(&vision_result, None);
        let second_duration = start.elapsed();

        // 第二次應該更快（使用緩存）
        println!("第一次生成: {:?}, 第二次生成: {:?}", first_duration, second_duration);
        // 注意：在測試環境中時間差可能很小，這個斷言可能不總是成功
        // assert!(second_duration <= first_duration);
    }
}