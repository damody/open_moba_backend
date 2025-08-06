// 獨立的視野測試程序
use std::path::Path;

// 加入相對路徑引用
#[path = "src/vision/debug_test.rs"]
mod debug_test;

fn main() {
    println!("開始調試視野計算...");
    debug_test::debug_vision_calculation();
}