// 獨立的 PosData 測試檔案
// 這個檔案用於單獨測試 PosData 的功能

use omobab::comp::outcome::{PosData, PosXIndex, PosYIndex, DisIndex, DisIndex2};
use vek::Vec2;
use specs::{World, WorldExt, Builder, Entity};

/// 建立測試用的實體和位置資料
fn create_test_data() -> (Vec<Entity>, Vec<Vec2<f32>>) {
    let mut world = World::new();
    let mut entities = vec![];
    let mut positions = vec![];
    
    // 建立測試實體和位置
    // 在 10x10 的網格上建立實體
    for x in 0..5 {
        for y in 0..5 {
            let entity = world.create_entity().build();
            entities.push(entity);
            positions.push(Vec2::new(x as f32 * 2.0, y as f32 * 2.0));
        }
    }
    
    (entities, positions)
}

fn main() {
    println!("開始測試 PosData 功能...\n");
    
    // 測試 1: PosData::new()
    println!("測試 1: PosData::new()");
    let posdata = PosData::new();
    assert_eq!(posdata.xpos.len(), 0);
    assert_eq!(posdata.ypos.len(), 0);
    assert_eq!(posdata.needsort, false);
    println!("✓ PosData::new() 測試通過\n");
    
    // 測試 2: 添加資料並排序
    println!("測試 2: 添加資料並排序");
    let mut posdata = PosData::new();
    let (entities, positions) = create_test_data();
    
    for (entity, pos) in entities.iter().zip(positions.iter()) {
        posdata.xpos.push(PosXIndex { e: *entity, p: *pos });
        posdata.ypos.push(PosYIndex { e: *entity, p: *pos });
    }
    
    posdata.xpos.sort_by(|a, b| a.p.x.partial_cmp(&b.p.x).unwrap());
    posdata.ypos.sort_by(|a, b| a.p.y.partial_cmp(&b.p.y).unwrap());
    
    // 驗證排序
    for i in 1..posdata.xpos.len() {
        assert!(posdata.xpos[i-1].p.x <= posdata.xpos[i].p.x);
    }
    for i in 1..posdata.ypos.len() {
        assert!(posdata.ypos[i-1].p.y <= posdata.ypos[i].p.y);
    }
    println!("✓ 排序測試通過\n");
    
    // 測試 3: SearchNN_X
    println!("測試 3: SearchNN_X");
    let search_pos = Vec2::new(0.0, 0.0);
    let radius = 3.0;
    let max_results = 5;
    let results = posdata.SearchNN_X(search_pos, radius, max_results);
    
    println!("  找到 {} 個實體（最大：{}）", results.len(), max_results);
    assert!(results.len() <= max_results);
    
    // 驗證所有結果都在半徑內
    for result in &results {
        let actual_distance = result.dis.sqrt();
        assert!(actual_distance <= radius);
        println!("  - 實體距離: {:.2}", actual_distance);
    }
    
    // 驗證結果按距離排序
    for i in 1..results.len() {
        assert!(results[i-1].dis <= results[i].dis);
    }
    println!("✓ SearchNN_X 測試通過\n");
    
    // 測試 4: SearchNN_XY
    println!("測試 4: SearchNN_XY");
    let search_pos = Vec2::new(2.0, 2.0);
    let radius = 2.5;
    let max_results = 5;
    let results = posdata.SearchNN_XY(search_pos, radius, max_results);
    
    println!("  找到 {} 個實體（最大：{}）", results.len(), max_results);
    assert!(results.len() > 0);
    assert!(results.len() <= max_results);
    
    for result in &results {
        let actual_distance = result.dis.sqrt();
        assert!(actual_distance <= radius);
        println!("  - 實體距離: {:.2}", actual_distance);
    }
    println!("✓ SearchNN_XY 測試通過\n");
    
    // 測試 5: SearchNN_XY2
    println!("測試 5: SearchNN_XY2（雙半徑搜尋）");
    let search_pos = Vec2::new(4.0, 4.0);
    let inner_radius = 3.0;
    let outer_radius = 6.0;
    let max_results = 10;
    
    let (inner_results, outer_results) = 
        posdata.SearchNN_XY2(search_pos, inner_radius, outer_radius, max_results);
    
    println!("  內圈找到 {} 個實體", inner_results.len());
    println!("  外圈找到 {} 個實體", outer_results.len());
    
    // 驗證內圈結果
    assert!(inner_results.len() <= max_results);
    for result in &inner_results {
        let actual_distance = result.dis.sqrt();
        assert!(actual_distance <= inner_radius);
    }
    
    // 驗證外圈結果
    for result in &outer_results {
        let actual_distance = result.dis.sqrt();
        assert!(actual_distance >= inner_radius);
        assert!(actual_distance <= outer_radius);
    }
    println!("✓ SearchNN_XY2 測試通過\n");
    
    // 測試 6: 邊界情況 - 空資料
    println!("測試 6: 邊界情況 - 空資料");
    let empty_posdata = PosData::new();
    let search_pos = Vec2::new(0.0, 0.0);
    let radius = 10.0;
    let max_results = 5;
    
    let results_x = empty_posdata.SearchNN_X(search_pos, radius, max_results);
    assert_eq!(results_x.len(), 0);
    
    let results_xy = empty_posdata.SearchNN_XY(search_pos, radius, max_results);
    assert_eq!(results_xy.len(), 0);
    
    let (inner, outer) = empty_posdata.SearchNN_XY2(search_pos, radius/2.0, radius, max_results);
    assert_eq!(inner.len(), 0);
    assert_eq!(outer.len(), 0);
    println!("✓ 空資料測試通過\n");
    
    // 測試 7: 性能測試
    println!("測試 7: 性能測試（大量資料）");
    let mut large_posdata = PosData::new();
    let mut world = World::new();
    
    // 建立 50x50 網格
    println!("  建立 2500 個實體...");
    for x in 0..50 {
        for y in 0..50 {
            let entity = world.create_entity().build();
            let pos = Vec2::new(x as f32, y as f32);
            large_posdata.xpos.push(PosXIndex { e: entity, p: pos });
            large_posdata.ypos.push(PosYIndex { e: entity, p: pos });
        }
    }
    
    // 排序
    println!("  排序索引...");
    large_posdata.xpos.sort_by(|a, b| a.p.x.partial_cmp(&b.p.x).unwrap());
    large_posdata.ypos.sort_by(|a, b| a.p.y.partial_cmp(&b.p.y).unwrap());
    
    // 測試搜尋性能
    let search_pos = Vec2::new(25.0, 25.0);
    let radius = 10.0;
    let max_results = 100;
    
    println!("  執行搜尋...");
    let start = std::time::Instant::now();
    let results = large_posdata.SearchNN_XY(search_pos, radius, max_results);
    let duration = start.elapsed();
    
    println!("  找到 {} 個實體", results.len());
    println!("  搜尋耗時: {:?}", duration);
    
    assert!(results.len() > 0);
    assert!(results.len() <= max_results);
    assert!(duration.as_millis() < 100, "搜尋應該在100毫秒內完成");
    println!("✓ 性能測試通過\n");
    
    println!("========================================");
    println!("所有測試通過！ 🎉");
    println!("========================================");
}