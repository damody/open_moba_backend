/// 高度圖地形系統
/// 
/// 使用二維高度圖來表示地形，支援視野計算和遮擋
use serde::{Deserialize, Serialize};
use vek::Vec2;
use std::collections::HashMap;

/// 地形高度圖
#[derive(Debug, Clone)]
pub struct TerrainHeightMap {
    /// 地圖寬度（網格數）
    pub width: usize,
    /// 地圖高度（網格數）
    pub height: usize,
    /// 每個網格的物理大小
    pub grid_size: f32,
    /// 高度數據（二維陣列）
    pub heights: Vec<Vec<f32>>,
    /// 地形類型數據
    pub terrain_types: Vec<Vec<TerrainType>>,
    /// 視野遮擋數據
    pub obstruction_map: Vec<Vec<f32>>,
}

/// 地形類型
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TerrainType {
    /// 平地 - 標準高度，無遮擋
    Ground = 0,
    /// 高地 - 提供視野優勢
    Hill = 1,
    /// 森林 - 部分遮擋視野
    Forest = 2,
    /// 水面 - 較低高度，無遮擋
    Water = 3,
    /// 建築 - 完全遮擋視野
    Building = 4,
    /// 懸崖 - 極高高度，完全遮擋
    Cliff = 5,
}

/// 地形配置結構
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TerrainConfig {
    /// 地圖寬度（米）
    pub map_width: f32,
    /// 地圖高度（米）
    pub map_height: f32,
    /// 網格大小（米）
    pub grid_size: f32,
    /// 基礎高度
    pub base_height: f32,
    /// 地形區域定義
    pub terrain_regions: Vec<TerrainRegion>,
}

/// 地形區域定義
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TerrainRegion {
    /// 區域名稱
    pub name: String,
    /// 地形類型
    pub terrain_type: String,
    /// 中心點 X
    pub center_x: f32,
    /// 中心點 Y  
    pub center_y: f32,
    /// 寬度
    pub width: f32,
    /// 高度
    pub height: f32,
    /// 地面高度
    pub terrain_height: f32,
    /// 視野遮擋程度 (0.0-1.0)
    pub obstruction: f32,
    /// 形狀類型 ("rectangle", "circle", "polygon")
    pub shape: String,
}

impl TerrainHeightMap {
    /// 創建新的地形高度圖
    pub fn new(width: usize, height: usize, grid_size: f32, base_height: f32) -> Self {
        let heights = vec![vec![base_height; width]; height];
        let terrain_types = vec![vec![TerrainType::Ground; width]; height];
        let obstruction_map = vec![vec![0.0; width]; height];
        
        Self {
            width,
            height,
            grid_size,
            heights,
            terrain_types,
            obstruction_map,
        }
    }
    
    /// 從配置文件創建地形
    pub fn from_config(config: &TerrainConfig) -> Self {
        let grid_width = (config.map_width / config.grid_size).ceil() as usize;
        let grid_height = (config.map_height / config.grid_size).ceil() as usize;
        
        let mut heightmap = Self::new(grid_width, grid_height, config.grid_size, config.base_height);
        
        // 應用地形區域
        for region in &config.terrain_regions {
            heightmap.apply_terrain_region(region);
        }
        
        heightmap
    }
    
    /// 應用地形區域
    fn apply_terrain_region(&mut self, region: &TerrainRegion) {
        let terrain_type = match region.terrain_type.as_str() {
            "Ground" => TerrainType::Ground,
            "Hill" => TerrainType::Hill,
            "Forest" => TerrainType::Forest,
            "Water" => TerrainType::Water,
            "Building" => TerrainType::Building,
            "Cliff" => TerrainType::Cliff,
            _ => TerrainType::Ground,
        };
        
        match region.shape.as_str() {
            "rectangle" => self.apply_rectangle_region(region, terrain_type),
            "circle" => self.apply_circle_region(region, terrain_type),
            _ => self.apply_rectangle_region(region, terrain_type),
        }
    }
    
    /// 應用矩形地形區域
    fn apply_rectangle_region(&mut self, region: &TerrainRegion, terrain_type: TerrainType) {
        let min_x = region.center_x - region.width * 0.5;
        let max_x = region.center_x + region.width * 0.5;
        let min_y = region.center_y - region.height * 0.5;
        let max_y = region.center_y + region.height * 0.5;
        
        let grid_min_x = ((min_x / self.grid_size).floor() as i32).max(0) as usize;
        let grid_max_x = ((max_x / self.grid_size).ceil() as i32).min(self.width as i32) as usize;
        let grid_min_y = ((min_y / self.grid_size).floor() as i32).max(0) as usize;
        let grid_max_y = ((max_y / self.grid_size).ceil() as i32).min(self.height as i32) as usize;
        
        for y in grid_min_y..grid_max_y {
            for x in grid_min_x..grid_max_x {
                let world_x = x as f32 * self.grid_size + self.grid_size * 0.5;
                let world_y = y as f32 * self.grid_size + self.grid_size * 0.5;
                
                if world_x >= min_x && world_x <= max_x && world_y >= min_y && world_y <= max_y {
                    self.heights[y][x] = region.terrain_height;
                    self.terrain_types[y][x] = terrain_type;
                    self.obstruction_map[y][x] = region.obstruction;
                }
            }
        }
    }
    
    /// 應用圓形地形區域
    fn apply_circle_region(&mut self, region: &TerrainRegion, terrain_type: TerrainType) {
        let radius = region.width.min(region.height) * 0.5;
        let center = Vec2::new(region.center_x, region.center_y);
        
        let grid_min_x = (((region.center_x - radius) / self.grid_size).floor() as i32).max(0) as usize;
        let grid_max_x = (((region.center_x + radius) / self.grid_size).ceil() as i32).min(self.width as i32) as usize;
        let grid_min_y = (((region.center_y - radius) / self.grid_size).floor() as i32).max(0) as usize;
        let grid_max_y = (((region.center_y + radius) / self.grid_size).ceil() as i32).min(self.height as i32) as usize;
        
        for y in grid_min_y..grid_max_y {
            for x in grid_min_x..grid_max_x {
                let world_x = x as f32 * self.grid_size + self.grid_size * 0.5;
                let world_y = y as f32 * self.grid_size + self.grid_size * 0.5;
                let pos = Vec2::new(world_x, world_y);
                
                if center.distance(pos) <= radius {
                    // 應用高度漸變效果
                    let distance_factor = 1.0 - (center.distance(pos) / radius);
                    self.heights[y][x] = region.terrain_height * distance_factor + self.heights[y][x] * (1.0 - distance_factor);
                    self.terrain_types[y][x] = terrain_type;
                    self.obstruction_map[y][x] = region.obstruction * distance_factor;
                }
            }
        }
    }
    
    /// 世界座標轉網格座標
    pub fn world_to_grid(&self, world_pos: Vec2<f32>) -> Option<(usize, usize)> {
        let grid_x = (world_pos.x / self.grid_size).floor() as i32;
        let grid_y = (world_pos.y / self.grid_size).floor() as i32;
        
        if grid_x >= 0 && grid_x < self.width as i32 && 
           grid_y >= 0 && grid_y < self.height as i32 {
            Some((grid_x as usize, grid_y as usize))
        } else {
            None
        }
    }
    
    /// 網格座標轉世界座標
    pub fn grid_to_world(&self, grid_x: usize, grid_y: usize) -> Vec2<f32> {
        Vec2::new(
            grid_x as f32 * self.grid_size + self.grid_size * 0.5,
            grid_y as f32 * self.grid_size + self.grid_size * 0.5,
        )
    }
    
    /// 獲取指定位置的高度（雙線性插值）
    pub fn get_height_at(&self, world_pos: Vec2<f32>) -> f32 {
        let grid_x = world_pos.x / self.grid_size;
        let grid_y = world_pos.y / self.grid_size;
        
        let x0 = grid_x.floor() as i32;
        let y0 = grid_y.floor() as i32;
        let x1 = x0 + 1;
        let y1 = y0 + 1;
        
        // 邊界檢查
        if x0 < 0 || y0 < 0 || x1 >= self.width as i32 || y1 >= self.height as i32 {
            return 0.0; // 超出邊界返回默認高度
        }
        
        let x0 = x0 as usize;
        let y0 = y0 as usize;
        let x1 = x1 as usize;
        let y1 = y1 as usize;
        
        // 雙線性插值
        let fx = grid_x - x0 as f32;
        let fy = grid_y - y0 as f32;
        
        let h00 = self.heights[y0][x0];
        let h10 = self.heights[y0][x1];
        let h01 = self.heights[y1][x0];
        let h11 = self.heights[y1][x1];
        
        let h0 = h00 * (1.0 - fx) + h10 * fx;
        let h1 = h01 * (1.0 - fx) + h11 * fx;
        
        h0 * (1.0 - fy) + h1 * fy
    }
    
    /// 獲取指定位置的地形類型
    pub fn get_terrain_type_at(&self, world_pos: Vec2<f32>) -> TerrainType {
        if let Some((grid_x, grid_y)) = self.world_to_grid(world_pos) {
            self.terrain_types[grid_y][grid_x]
        } else {
            TerrainType::Ground
        }
    }
    
    /// 獲取指定位置的遮擋程度
    pub fn get_obstruction_at(&self, world_pos: Vec2<f32>) -> f32 {
        if let Some((grid_x, grid_y)) = self.world_to_grid(world_pos) {
            self.obstruction_map[grid_y][grid_x]
        } else {
            0.0
        }
    }
    
    /// 檢查兩點之間的視線是否被地形遮擋
    pub fn is_line_of_sight_blocked(&self, from: Vec2<f32>, to: Vec2<f32>, observer_height: f32) -> bool {
        let direction = (to - from).normalized();
        let distance = from.distance(to);
        let step_size = self.grid_size * 0.5; // 使用半個網格大小作為步長
        let steps = (distance / step_size) as i32;
        
        let from_ground_height = self.get_height_at(from);
        let to_ground_height = self.get_height_at(to);
        let observer_total_height = from_ground_height + observer_height;
        
        for step in 1..steps {
            let t = step as f32 / steps as f32;
            let check_pos = from + direction * (step as f32 * step_size);
            let ground_height = self.get_height_at(check_pos);
            
            // 計算視線高度（線性插值）
            let line_height = observer_total_height + (to_ground_height - from_ground_height) * t;
            
            // 檢查地形遮擋
            let terrain_type = self.get_terrain_type_at(check_pos);
            let obstruction = self.get_obstruction_at(check_pos);
            
            match terrain_type {
                TerrainType::Forest => {
                    // 森林部分遮擋，根據遮擋程度和高度差判斷
                    if obstruction > 0.5 && ground_height + 50.0 > line_height {
                        return true;
                    }
                },
                TerrainType::Building | TerrainType::Cliff => {
                    // 建築和懸崖完全遮擋
                    if ground_height + 100.0 > line_height {
                        return true;
                    }
                },
                TerrainType::Hill => {
                    // 山丘可能遮擋，取決於高度差
                    if ground_height + 20.0 > line_height {
                        return true;
                    }
                },
                _ => {}
            }
        }
        
        false
    }
    
    /// 計算從指定位置可見的區域
    pub fn calculate_visible_area(&self, observer_pos: Vec2<f32>, observer_height: f32, vision_range: f32) -> Vec<Vec2<f32>> {
        let mut visible_positions = Vec::new();
        
        // 使用極坐標掃描
        let ray_count = 360; // 360度掃描
        let angle_step = 2.0 * std::f32::consts::PI / ray_count as f32;
        
        for i in 0..ray_count {
            let angle = i as f32 * angle_step;
            let direction = Vec2::new(angle.cos(), angle.sin());
            
            // 沿射線方向計算可見點
            let step_size = self.grid_size;
            let max_steps = (vision_range / step_size) as i32;
            
            for step in 1..=max_steps {
                let distance = step as f32 * step_size;
                if distance > vision_range {
                    break;
                }
                
                let check_pos = observer_pos + direction * distance;
                
                // 檢查視線是否被阻擋
                if self.is_line_of_sight_blocked(observer_pos, check_pos, observer_height) {
                    break;
                }
                
                visible_positions.push(check_pos);
            }
        }
        
        // 去重並返回
        self.deduplicate_positions(visible_positions)
    }
    
    /// 去除重複位置
    fn deduplicate_positions(&self, positions: Vec<Vec2<f32>>) -> Vec<Vec2<f32>> {
        let mut unique_positions = Vec::new();
        
        for pos in positions {
            let grid_pos = Vec2::new(
                (pos.x / self.grid_size).round() * self.grid_size,
                (pos.y / self.grid_size).round() * self.grid_size,
            );
            
            if !unique_positions.iter().any(|&existing: &Vec2<f32>| existing.distance(grid_pos) < self.grid_size * 0.1) {
                unique_positions.push(grid_pos);
            }
        }
        
        unique_positions
    }
    
    /// 生成高度圖的調試輸出
    pub fn debug_heightmap(&self, area: (Vec2<f32>, Vec2<f32>)) -> String {
        let (min_pos, max_pos) = area;
        let min_grid = self.world_to_grid(min_pos).unwrap_or((0, 0));
        let max_grid = self.world_to_grid(max_pos).unwrap_or((self.width - 1, self.height - 1));
        
        let mut output = String::new();
        output.push_str(&format!("高度圖區域 ({:.1}, {:.1}) 到 ({:.1}, {:.1}):\n", 
            min_pos.x, min_pos.y, max_pos.x, max_pos.y));
        
        for y in min_grid.1..=max_grid.1.min(self.height - 1) {
            for x in min_grid.0..=max_grid.0.min(self.width - 1) {
                let height = self.heights[y][x];
                let terrain_type = match self.terrain_types[y][x] {
                    TerrainType::Ground => ".",
                    TerrainType::Hill => "^",
                    TerrainType::Forest => "T",
                    TerrainType::Water => "~",
                    TerrainType::Building => "#",
                    TerrainType::Cliff => "▲",
                };
                output.push_str(&format!("{:>3.0}{}", height, terrain_type));
            }
            output.push('\n');
        }
        
        output
    }
}