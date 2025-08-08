// 暴力搜尋測試 - 用於驗證 PosData 優化演算法的正確性
// 使用暴力搜尋法作為對照組，確保優化演算法結果正確

use omobab::comp::outcome::{PosData, PosXIndex, PosYIndex, DisIndex};
use vek::Vec2;
use specs::{World, WorldExt, Builder, Entity};
use std::collections::HashSet;

/// 暴力搜尋法 - 檢查所有實體
/// 這是最直接但最慢的方法，用作驗證基準
fn brute_force_search(
    all_entities: &[(Entity, Vec2<f32>)],
    search_pos: Vec2<f32>,
    radius: f32,
    max_results: usize,
) -> Vec<(Entity, f32)> {
    let r2 = radius * radius;
    let mut results = Vec::new();
    
    // 檢查每一個實體
    for (entity, pos) in all_entities {
        let distance_squared = pos.distance_squared(search_pos);
        if distance_squared <= r2 {
            results.push((*entity, distance_squared));
        }
    }
    
    // 按距離排序
    results.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    
    // 截取前 n 個
    results.truncate(max_results);
    results
}

/// 暴力搜尋法 - 雙半徑版本
fn brute_force_search_dual(
    all_entities: &[(Entity, Vec2<f32>)],
    search_pos: Vec2<f32>,
    inner_radius: f32,
    outer_radius: f32,
    max_results: usize,
) -> (Vec<(Entity, f32)>, Vec<(Entity, f32)>) {
    let inner_r2 = inner_radius * inner_radius;
    let outer_r2 = outer_radius * outer_radius;
    let mut inner_results = Vec::new();
    let mut outer_results = Vec::new();
    
    // 檢查每一個實體
    for (entity, pos) in all_entities {
        let distance_squared = pos.distance_squared(search_pos);
        if distance_squared <= inner_r2 {
            inner_results.push((*entity, distance_squared));
        } else if distance_squared <= outer_r2 {
            outer_results.push((*entity, distance_squared));
        }
    }
    
    // 按距離排序
    inner_results.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    inner_results.truncate(max_results);
    
    (inner_results, outer_results)
}

/// 比較兩個結果集是否相同
fn compare_results(
    optimized: &[DisIndex],
    brute_force: &[(Entity, f32)],
    test_name: &str,
) -> bool {
    if optimized.len() != brute_force.len() {
        println!("❌ {} - 結果數量不同: 優化={}, 暴力={}", 
                 test_name, optimized.len(), brute_force.len());
        return false;
    }
    
    for (i, (opt, brute)) in optimized.iter().zip(brute_force.iter()).enumerate() {
        if opt.e != brute.0 {
            println!("❌ {} - 第{}個實體不同", test_name, i);
            return false;
        }
        
        // 允許浮點數有小誤差
        if (opt.dis - brute.1).abs() > 0.0001 {
            println!("❌ {} - 第{}個距離不同: 優化={}, 暴力={}", 
                     test_name, i, opt.dis, brute.1);
            return false;
        }
    }
    
    println!("✓ {} 通過驗證", test_name);
    true
}

/// 建立隨機分布的測試資料
fn create_random_test_data(count: usize, area_size: f32) -> (Vec<Entity>, Vec<Vec2<f32>>) {
    let mut world = World::new();
    let mut entities = Vec::new();
    let mut positions = Vec::new();
    
    use fastrand;
    
    for _ in 0..count {
        let entity = world.create_entity().build();
        entities.push(entity);
        
        // 隨機位置
        let x = fastrand::f32() * area_size;
        let y = fastrand::f32() * area_size;
        positions.push(Vec2::new(x, y));
    }
    
    (entities, positions)
}

/// 主測試函數
fn main() {
    println!("========================================");
    println!("暴力搜尋交叉驗證測試");
    println!("========================================\n");
    
    // 測試配置
    let test_configs = vec![
        (100, 50.0),   // 100個實體，50x50區域
        (500, 100.0),  // 500個實體，100x100區域
        (1000, 200.0), // 1000個實體，200x200區域
    ];
    
    let mut total_tests = 0;
    let mut passed_tests = 0;
    
    for (entity_count, area_size) in test_configs {
        println!("\n--- 測試配置：{}個實體，{}x{}區域 ---", 
                 entity_count, area_size, area_size);
        
        // 建立隨機測試資料
        let (entities, positions) = create_random_test_data(entity_count, area_size);
        
        // 建立 PosData 並填充資料
        let mut posdata = PosData::new();
        let mut all_entities = Vec::new();
        
        for (entity, pos) in entities.iter().zip(positions.iter()) {
            posdata.xpos.push(PosXIndex { e: *entity, p: *pos });
            posdata.ypos.push(PosYIndex { e: *entity, p: *pos });
            all_entities.push((*entity, *pos));
        }
        
        // 排序索引
        posdata.xpos.sort_by(|a, b| a.p.x.partial_cmp(&b.p.x).unwrap());
        posdata.ypos.sort_by(|a, b| a.p.y.partial_cmp(&b.p.y).unwrap());
        
        // 執行多個隨機搜尋測試
        for test_idx in 0..10 {
            // 隨機搜尋參數
            let search_x = fastrand::f32() * area_size;
            let search_y = fastrand::f32() * area_size;
            let search_pos = Vec2::new(search_x, search_y);
            let radius = fastrand::f32() * (area_size / 5.0) + 1.0;
            let max_results = (fastrand::usize(10..50)).min(entity_count);
            
            println!("\n測試 #{}: 位置({:.1}, {:.1}), 半徑={:.1}, 最大結果={}", 
                     test_idx + 1, search_x, search_y, radius, max_results);
            
            // 測試 SearchNN_X
            {
                total_tests += 1;
                let opt_results = posdata.SearchNN_X(search_pos, radius, max_results);
                let brute_results = brute_force_search(&all_entities, search_pos, radius, max_results);
                
                if compare_results(&opt_results, &brute_results, "SearchNN_X") {
                    passed_tests += 1;
                } else {
                    // 顯示詳細差異
                    println!("  優化結果: {:?}", opt_results.iter().map(|r| r.dis.sqrt()).collect::<Vec<_>>());
                    println!("  暴力結果: {:?}", brute_results.iter().map(|r| r.1.sqrt()).collect::<Vec<_>>());
                }
            }
            
            // 測試 SearchNN_XY
            {
                total_tests += 1;
                let opt_results = posdata.SearchNN_XY(search_pos, radius, max_results);
                let brute_results = brute_force_search(&all_entities, search_pos, radius, max_results);
                
                if compare_results(&opt_results, &brute_results, "SearchNN_XY") {
                    passed_tests += 1;
                } else {
                    println!("  優化結果: {:?}", opt_results.iter().map(|r| r.dis.sqrt()).collect::<Vec<_>>());
                    println!("  暴力結果: {:?}", brute_results.iter().map(|r| r.1.sqrt()).collect::<Vec<_>>());
                }
            }
            
            // 測試 SearchNN_XY2
            {
                total_tests += 1;
                let inner_radius = radius * 0.6;
                let outer_radius = radius;
                
                let (opt_inner, opt_outer) = posdata.SearchNN_XY2(search_pos, inner_radius, outer_radius, max_results);
                let (brute_inner, brute_outer) = brute_force_search_dual(&all_entities, search_pos, inner_radius, outer_radius, max_results);
                
                let inner_match = compare_results(&opt_inner, &brute_inner, "SearchNN_XY2 內圈");
                let outer_match = opt_outer.len() == brute_outer.len();
                
                if inner_match && outer_match {
                    passed_tests += 1;
                    println!("✓ SearchNN_XY2 外圈數量匹配: {}", opt_outer.len());
                } else if !outer_match {
                    println!("❌ SearchNN_XY2 外圈數量不同: 優化={}, 暴力={}", 
                             opt_outer.len(), brute_outer.len());
                }
            }
        }
    }
    
    // 特殊邊界測試
    println!("\n\n--- 特殊邊界測試 ---");
    
    // 測試1: 所有實體在同一點
    {
        println!("\n測試：所有實體在同一點");
        total_tests += 1;
        
        let mut world = World::new();
        let mut posdata = PosData::new();
        let mut all_entities = Vec::new();
        let same_pos = Vec2::new(10.0, 10.0);
        
        for _ in 0..10 {
            let entity = world.create_entity().build();
            posdata.xpos.push(PosXIndex { e: entity, p: same_pos });
            posdata.ypos.push(PosYIndex { e: entity, p: same_pos });
            all_entities.push((entity, same_pos));
        }
        
        posdata.xpos.sort_by(|a, b| a.p.x.partial_cmp(&b.p.x).unwrap());
        posdata.ypos.sort_by(|a, b| a.p.y.partial_cmp(&b.p.y).unwrap());
        
        let opt_results = posdata.SearchNN_XY(same_pos, 1.0, 5);
        let brute_results = brute_force_search(&all_entities, same_pos, 1.0, 5);
        
        if compare_results(&opt_results, &brute_results, "同點測試") {
            passed_tests += 1;
        }
    }
    
    // 測試2: 實體在圓周上均勻分布
    {
        println!("\n測試：實體在圓周上均勻分布");
        total_tests += 1;
        
        let mut world = World::new();
        let mut posdata = PosData::new();
        let mut all_entities = Vec::new();
        let center = Vec2::new(50.0, 50.0);
        let circle_radius = 20.0;
        
        for i in 0..16 {
            let angle = (i as f32) * std::f32::consts::PI * 2.0 / 16.0;
            let x = center.x + angle.cos() * circle_radius;
            let y = center.y + angle.sin() * circle_radius;
            let pos = Vec2::new(x, y);
            
            let entity = world.create_entity().build();
            posdata.xpos.push(PosXIndex { e: entity, p: pos });
            posdata.ypos.push(PosYIndex { e: entity, p: pos });
            all_entities.push((entity, pos));
        }
        
        posdata.xpos.sort_by(|a, b| a.p.x.partial_cmp(&b.p.x).unwrap());
        posdata.ypos.sort_by(|a, b| a.p.y.partial_cmp(&b.p.y).unwrap());
        
        let opt_results = posdata.SearchNN_XY(center, circle_radius + 1.0, 20);
        let brute_results = brute_force_search(&all_entities, center, circle_radius + 1.0, 20);
        
        if compare_results(&opt_results, &brute_results, "圓周分布測試") {
            passed_tests += 1;
        }
    }
    
    // 顯示最終結果
    println!("\n========================================");
    println!("測試結果總結");
    println!("========================================");
    println!("總測試數: {}", total_tests);
    println!("通過測試: {}", passed_tests);
    println!("失敗測試: {}", total_tests - passed_tests);
    
    let pass_rate = (passed_tests as f32 / total_tests as f32) * 100.0;
    println!("通過率: {:.1}%", pass_rate);
    
    if passed_tests == total_tests {
        println!("\n🎉 所有測試通過！優化演算法結果完全正確！");
    } else {
        println!("\n⚠️ 有測試失敗，請檢查演算法實作");
    }
}