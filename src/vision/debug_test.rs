/// 調試測試 - 檢查實際的視野計算邏輯
use vek::Vec2;
use crate::comp::circular_vision::{ObstacleInfo, ObstacleType, ObstacleProperties};
use crate::vision::{ShadowCalculator, shadow_calculator::Bounds};

pub fn debug_vision_calculation() {
    println!("=== 調試視野計算 ===");
    
    let mut calculator = ShadowCalculator::new();
    
    // 創建一個簡單的障礙物
    let obstacles = vec![
        ObstacleInfo {
            position: Vec2::new(200.0, 200.0), // 在觀察者右上方
            obstacle_type: ObstacleType::Circular { radius: 50.0 },
            height: 200.0,
            properties: ObstacleProperties {
                blocks_completely: false,
                opacity: 0.8,
                shadow_multiplier: 2.0,
            }
        }
    ];

    // 初始化四叉樹
    let world_bounds = Bounds::new(
        Vec2::new(0.0, 0.0),
        Vec2::new(1000.0, 1000.0)
    );
    calculator.initialize_quadtree(world_bounds, obstacles);

    // 計算視野
    let observer_pos = Vec2::new(100.0, 100.0);
    let result = calculator.calculate_optimized_vision(
        observer_pos,
        30.0,  // 觀察者高度
        500.0  // 視野範圍
    );

    println!("觀察者位置: {:?}", result.observer_pos);
    println!("視野範圍: {}", result.range);
    println!("可見區域點數: {}", result.visible_area.len());
    println!("陰影數量: {}", result.shadows.len());
    
    if !result.shadows.is_empty() {
        for (i, shadow) in result.shadows.iter().enumerate() {
            println!("陰影 {}: 類型={:?}, 深度={:.2}", i, shadow.shadow_type, shadow.depth);
        }
    } else {
        println!("警告: 沒有計算出任何陰影！");
    }

    if result.visible_area.is_empty() {
        println!("錯誤: 沒有計算出可見區域！");
    } else {
        println!("前幾個可見點: {:?}", &result.visible_area[0..5.min(result.visible_area.len())]);
    }
    
    println!("=== 調試結束 ===");
}