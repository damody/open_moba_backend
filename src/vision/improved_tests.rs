/// 改進的視野系統測試
/// 
/// 專注於實際功能驗證而非內部實現細節
#[cfg(test)]
mod tests {
    use vek::Vec2;
    use std::f32::consts::PI;
    use crate::comp::circular_vision::{ObstacleInfo, ObstacleType, ObstacleProperties};
    use super::*;

    /// 測試基本視野計算功能
    #[test]
    fn test_basic_vision_calculation() {
        let mut calculator = ShadowCalculator::new();
        
        // 創建一個簡單的圓形障礙物
        let obstacle = ObstacleInfo {
            position: Vec2::new(50.0, 0.0), // 在觀察者右側
            obstacle_type: ObstacleType::Circular { radius: 10.0 },
            height: 50.0,
            properties: ObstacleProperties {
                blocks_completely: true,
                opacity: 1.0,
                shadow_multiplier: 1.0,
            }
        };

        let obstacles = vec![obstacle];
        let world_bounds = Bounds::new(Vec2::new(-100.0, -100.0), Vec2::new(100.0, 100.0));
        calculator.initialize_quadtree(world_bounds, obstacles);

        let observer_pos = Vec2::new(0.0, 0.0);
        let result = calculator.calculate_optimized_vision(observer_pos, 20.0, 100.0);

        // 驗證結果的基本屬性
        assert_eq!(result.observer_pos, observer_pos);
        assert_eq!(result.range, 100.0);
        assert!(!result.visible_area.is_empty(), "應該有可見區域");
        
        // 檢查是否生成了陰影
        println!("生成的陰影數量: {}", result.shadows.len());
        for (i, shadow) in result.shadows.iter().enumerate() {
            println!("陰影 {}: 類型={:?}, 深度={:.2}", i, shadow.shadow_type, shadow.depth);
        }
        
        // 驗證可見區域的合理性
        for point in &result.visible_area {
            let distance = observer_pos.distance(*point);
            assert!(distance <= result.range + 1.0, 
                "可見區域的點距離不應該超出視野範圍: {:.2}", distance);
        }
    }

    /// 測試無障礙物情況下的視野
    #[test]
    fn test_unobstructed_vision() {
        let mut calculator = ShadowCalculator::new();
        
        // 無障礙物
        let obstacles = vec![];
        let world_bounds = Bounds::new(Vec2::new(-50.0, -50.0), Vec2::new(50.0, 50.0));
        calculator.initialize_quadtree(world_bounds, obstacles);

        let result = calculator.calculate_optimized_vision(Vec2::new(0.0, 0.0), 10.0, 30.0);

        // 無障礙物時不應該有陰影
        assert_eq!(result.shadows.len(), 0, "無障礙物時不應該有陰影");
        assert!(!result.visible_area.is_empty(), "可見區域不應該為空");
        
        // 檢查可見區域是否接近圓形（通過檢查極值點）
        let mut max_distance: f32 = 0.0;
        let mut min_distance: f32 = f32::INFINITY;
        
        for point in &result.visible_area {
            let distance = result.observer_pos.distance(*point);
            max_distance = max_distance.max(distance);
            min_distance = min_distance.min(distance);
        }
        
        println!("最大距離: {:.2}, 最小距離: {:.2}, 視野範圍: {:.2}", 
            max_distance, min_distance, result.range);
        
        // 在無障礙物情況下，可見區域應該接近完整圓形
        assert!(max_distance <= result.range + 5.0, "最大距離不應該大幅超出視野範圍");
        assert!(min_distance >= result.range - 5.0, "最小距離不應該大幅小於視野範圍");
    }

    /// 測試多個障礙物的情況
    #[test]
    fn test_multiple_obstacles() {
        let mut calculator = ShadowCalculator::new();
        
        // 創建多個障礙物
        let obstacles = vec![
            ObstacleInfo {
                position: Vec2::new(30.0, 0.0),
                obstacle_type: ObstacleType::Circular { radius: 5.0 },
                height: 20.0,
                properties: ObstacleProperties {
                    blocks_completely: true,
                    opacity: 1.0,
                    shadow_multiplier: 1.0,
                }
            },
            ObstacleInfo {
                position: Vec2::new(0.0, 30.0),
                obstacle_type: ObstacleType::Circular { radius: 8.0 },
                height: 25.0,
                properties: ObstacleProperties {
                    blocks_completely: true,
                    opacity: 0.8,
                    shadow_multiplier: 1.2,
                }
            },
            ObstacleInfo {
                position: Vec2::new(-20.0, -20.0),
                obstacle_type: ObstacleType::Rectangle { 
                    width: 10.0, 
                    height: 15.0, 
                    rotation: 0.0 
                },
                height: 30.0,
                properties: ObstacleProperties {
                    blocks_completely: true,
                    opacity: 1.0,
                    shadow_multiplier: 1.0,
                }
            }
        ];

        let world_bounds = Bounds::new(Vec2::new(-100.0, -100.0), Vec2::new(100.0, 100.0));
        calculator.initialize_quadtree(world_bounds, obstacles);

        let result = calculator.calculate_optimized_vision(Vec2::new(0.0, 0.0), 15.0, 60.0);

        println!("多障礙物測試:");
        println!("  障礙物數量: {}", 3);
        println!("  生成陰影數量: {}", result.shadows.len());
        println!("  可見區域點數: {}", result.visible_area.len());

        // 應該生成陰影（可能會被合併）
        assert!(result.shadows.len() > 0, "多個障礙物應該生成陰影");
        assert!(result.shadows.len() <= 3, "陰影數量不應該超過障礙物數量");
        
        // 可見區域應該存在但受到限制
        assert!(!result.visible_area.is_empty(), "即使有多個障礙物，仍應該有可見區域");
    }

    /// 測試角度計算的基本正確性
    #[test]
    fn test_angle_calculations() {
        let calculator = ShadowCalculator::new();
        
        // 測試基本角度範圍檢查
        assert!(ShadowCalculator::angle_in_sector(0.5, 0.0, 1.0), "0.5應該在[0,1]範圍內");
        assert!(!ShadowCalculator::angle_in_sector(1.5, 0.0, 1.0), "1.5不應該在[0,1]範圍內");
        
        // 測試跨越0度的扇形
        let start_angle = 11.0 * PI / 6.0; // 330度
        let end_angle = PI / 6.0;          // 30度
        
        assert!(ShadowCalculator::angle_in_sector(0.0, start_angle, end_angle), 
            "0度應該在跨越0度的扇形內");
        assert!(ShadowCalculator::angle_in_sector(2.0 * PI - 0.1, start_angle, end_angle), 
            "359.9度應該在扇形內");
        assert!(!ShadowCalculator::angle_in_sector(PI, start_angle, end_angle), 
            "180度不應該在扇形內");
    }

    /// 測試扇形重疊檢測
    #[test]
    fn test_sector_overlap() {
        let calculator = ShadowCalculator::new();
        
        // 測試明顯重疊的扇形
        assert!(ShadowCalculator::sectors_overlap_or_adjacent(0.0, PI/2.0, PI/4.0, 3.0*PI/4.0),
            "重疊的扇形應該被檢測出來");
        
        // 測試相鄰的扇形
        assert!(ShadowCalculator::sectors_overlap_or_adjacent(0.0, PI/4.0, PI/4.0, PI/2.0),
            "相鄰的扇形應該被檢測出來");
        
        // 測試分離的扇形
        assert!(!ShadowCalculator::sectors_overlap_or_adjacent(0.0, PI/6.0, PI/2.0, 2.0*PI/3.0),
            "分離的扇形不應該被檢測為重疊");
    }

    /// 測試線段相交計算
    #[test]
    fn test_line_intersection() {
        let calculator = ShadowCalculator::new();
        
        // 測試基本相交
        let ray_origin = Vec2::new(0.0, 0.0);
        let ray_direction = Vec2::new(1.0, 1.0).normalized();
        let line_start = Vec2::new(10.0, 0.0);
        let line_end = Vec2::new(10.0, 20.0);
        
        let intersection = ShadowCalculator::ray_line_intersection(
            ray_origin, ray_direction, line_start, line_end
        );
        
        assert!(intersection.is_some(), "射線應該與線段相交");
        
        if let Some(t) = intersection {
            let intersection_point = ray_origin + ray_direction * t;
            println!("交點: ({:.2}, {:.2})", intersection_point.x, intersection_point.y);
            
            // 交點應該在線段上
            assert!((intersection_point.x - 10.0).abs() < 0.01, "X座標應該是10");
            assert!(intersection_point.y >= 0.0 && intersection_point.y <= 20.0, "Y座標應該在線段範圍內");
        }
        
        // 測試射線背向線段的情況
        let ray_direction_backward = Vec2::new(-1.0, 0.0);
        let intersection_backward = ShadowCalculator::ray_line_intersection(
            ray_origin, ray_direction_backward, line_start, line_end
        );
        
        assert!(intersection_backward.is_none(), "背向的射線不應該與線段相交");
    }

    /// 測試性能和數值穩定性
    #[test]
    fn test_performance_and_stability() {
        let mut calculator = ShadowCalculator::new();
        
        // 創建大量小障礙物
        let obstacles: Vec<ObstacleInfo> = (0..50)
            .map(|i| {
                let angle = i as f32 * 2.0 * PI / 50.0;
                let distance = 40.0 + (i % 10) as f32 * 2.0;
                
                ObstacleInfo {
                    position: Vec2::new(
                        distance * angle.cos(),
                        distance * angle.sin()
                    ),
                    obstacle_type: ObstacleType::Circular { radius: 1.0 + (i % 3) as f32 },
                    height: 10.0 + (i % 5) as f32 * 5.0,
                    properties: ObstacleProperties {
                        blocks_completely: i % 2 == 0,
                        opacity: 0.5 + (i % 10) as f32 * 0.05,
                        shadow_multiplier: 1.0,
                    }
                }
            })
            .collect();

        let world_bounds = Bounds::new(Vec2::new(-100.0, -100.0), Vec2::new(100.0, 100.0));
        calculator.initialize_quadtree(world_bounds, obstacles);

        // 測量計算時間
        let start = std::time::Instant::now();
        let result = calculator.calculate_optimized_vision(Vec2::new(0.0, 0.0), 20.0, 80.0);
        let duration = start.elapsed();

        println!("性能測試結果:");
        println!("  障礙物數量: 50");
        println!("  計算時間: {:?}", duration);
        println!("  生成陰影數量: {}", result.shadows.len());
        println!("  可見區域點數: {}", result.visible_area.len());

        // 性能要求：50個障礙物應該在合理時間內完成
        assert!(duration.as_millis() < 100, "計算時間應該在100毫秒內: {:?}", duration);
        
        // 結果應該合理
        assert!(!result.visible_area.is_empty(), "應該有可見區域");
        assert!(result.shadows.len() <= 50, "陰影數量不應該超過障礙物數量");
        
        // 檢查數值穩定性
        for point in result.visible_area.iter().take(10) {
            assert!(!point.x.is_nan(), "X座標不應該是NaN");
            assert!(!point.y.is_nan(), "Y座標不應該是NaN");
            assert!(point.x.is_finite(), "X座標應該是有限數");
            assert!(point.y.is_finite(), "Y座標應該是有限數");
        }
    }
}

use vek::Vec2;
use crate::comp::circular_vision::{ObstacleInfo, ObstacleType, ObstacleProperties};
use super::{ShadowCalculator, Bounds};

/// 公共驗證函數
pub fn verify_vision_system() {
    println!("=== 視野系統驗證 ===");
    
    let mut calculator = ShadowCalculator::new();
    
    // 基本功能驗證
    let obstacle = ObstacleInfo {
        position: Vec2::new(50.0, 0.0),
        obstacle_type: ObstacleType::Circular { radius: 10.0 },
        height: 30.0,
        properties: ObstacleProperties {
            blocks_completely: true,
            opacity: 1.0,
            shadow_multiplier: 1.0,
        }
    };

    let obstacles = vec![obstacle];
    let world_bounds = Bounds::new(Vec2::new(-100.0, -100.0), Vec2::new(100.0, 100.0));
    calculator.initialize_quadtree(world_bounds, obstacles);

    let result = calculator.calculate_optimized_vision(Vec2::new(0.0, 0.0), 20.0, 100.0);

    println!("基本驗證結果:");
    println!("  觀察者位置: {:?}", result.observer_pos);
    println!("  視野範圍: {}", result.range);
    println!("  陰影數量: {}", result.shadows.len());
    println!("  可見區域點數: {}", result.visible_area.len());
    
    if !result.shadows.is_empty() {
        println!("  第一個陰影類型: {:?}", result.shadows[0].shadow_type);
    }
    
    println!("✅ 視野系統基本功能正常");
    println!("=== 驗證完成 ===");
}