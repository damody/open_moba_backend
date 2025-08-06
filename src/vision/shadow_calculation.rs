use vek::Vec2;
use crate::comp::circular_vision::{ShadowArea, ShadowType, ShadowGeometry, ObstacleInfo, ObstacleType};

pub struct ShadowCalculator;

impl ShadowCalculator {
    /// 計算單個障礙物的陰影
    pub fn calculate_obstacle_shadow(
        observer_pos: Vec2<f32>,
        observer_height: f32,
        vision_range: f32,
        obstacle: &ObstacleInfo,
    ) -> Option<ShadowArea> {
        let distance = observer_pos.distance(obstacle.position);
        
        if distance > vision_range {
            return None;
        }

        match &obstacle.obstacle_type {
            ObstacleType::Circular { radius } => {
                Self::calculate_circular_shadow(
                    observer_pos, vision_range, obstacle.position, 
                    *radius, obstacle.height, observer_height
                )
            },
            ObstacleType::Rectangle { width, height, rotation } => {
                Self::calculate_rectangular_shadow(
                    observer_pos, vision_range, obstacle.position,
                    *width, *height, *rotation, obstacle.height, observer_height
                )
            },
            ObstacleType::Terrain { elevation } => {
                Self::calculate_terrain_shadow(
                    observer_pos, observer_height, vision_range,
                    obstacle.position, *elevation
                )
            },
        }
    }

    /// 圓形陰影計算
    pub fn calculate_circular_shadow(
        observer_pos: Vec2<f32>,
        vision_range: f32,
        obstacle_pos: Vec2<f32>,
        radius: f32,
        obstacle_height: f32,
        observer_height: f32,
    ) -> Option<ShadowArea> {
        let to_obstacle = obstacle_pos - observer_pos;
        let distance = to_obstacle.magnitude();
        
        if obstacle_height <= observer_height * 0.5 {
            return None;
        }
        
        let center_angle = to_obstacle.y.atan2(to_obstacle.x);
        let angle_offset = (radius / distance).asin();
        
        if angle_offset.to_degrees() < 0.5 {
            return None;
        }
        
        Some(ShadowArea {
            shadow_type: ShadowType::Sector,
            blocker_id: None,
            geometry: ShadowGeometry::Sector {
                center: observer_pos,
                start_angle: center_angle - angle_offset,
                end_angle: center_angle + angle_offset,
                radius: vision_range,
            },
            depth: vision_range - distance,
        })
    }

    /// 矩形陰影計算（簡化為圓形）
    pub fn calculate_rectangular_shadow(
        observer_pos: Vec2<f32>,
        vision_range: f32,
        obstacle_pos: Vec2<f32>,
        width: f32,
        height: f32,
        _rotation: f32,
        obstacle_height: f32,
        observer_height: f32,
    ) -> Option<ShadowArea> {
        let effective_radius = (width * width + height * height).sqrt() * 0.5;
        Self::calculate_circular_shadow(
            observer_pos, vision_range, obstacle_pos,
            effective_radius, obstacle_height, observer_height
        )
    }

    /// 地形陰影計算
    pub fn calculate_terrain_shadow(
        observer_pos: Vec2<f32>,
        observer_height: f32,
        vision_range: f32,
        terrain_pos: Vec2<f32>,
        elevation: f32,
    ) -> Option<ShadowArea> {
        if elevation <= observer_height {
            return None;
        }
        
        let to_terrain = terrain_pos - observer_pos;
        let center_angle = to_terrain.y.atan2(to_terrain.x);
        
        Some(ShadowArea {
            shadow_type: ShadowType::Terrain,
            blocker_id: None,
            geometry: ShadowGeometry::Sector {
                center: observer_pos,
                start_angle: center_angle - 0.05,
                end_angle: center_angle + 0.05,
                radius: vision_range,
            },
            depth: vision_range * 0.3,
        })
    }

    /// 合併重疊的陰影
    pub fn merge_overlapping_shadows(shadows: Vec<ShadowArea>) -> Vec<ShadowArea> {
        if shadows.len() <= 1 {
            return shadows;
        }

        let mut merged = Vec::new();
        let mut sector_shadows = Vec::new();
        let mut polygon_shadows = Vec::new();

        // 分離不同類型的陰影
        for shadow in shadows {
            match shadow.geometry {
                ShadowGeometry::Sector { .. } => sector_shadows.push(shadow),
                ShadowGeometry::Polygon { .. } => polygon_shadows.push(shadow),
                _ => merged.push(shadow),
            }
        }

        // 合併扇形陰影
        merged.extend(Self::merge_sector_shadows(sector_shadows));
        
        // 多邊形陰影暫時不合併（太複雜），直接添加
        merged.extend(polygon_shadows);

        merged
    }

    /// 合併扇形陰影
    pub fn merge_sector_shadows(mut sectors: Vec<ShadowArea>) -> Vec<ShadowArea> {
        if sectors.is_empty() {
            return Vec::new();
        }

        // 按起始角度排序
        sectors.sort_by(|a, b| {
            if let (ShadowGeometry::Sector { start_angle: a_start, .. }, 
                    ShadowGeometry::Sector { start_angle: b_start, .. }) = (&a.geometry, &b.geometry) {
                a_start.partial_cmp(b_start).unwrap_or(std::cmp::Ordering::Equal)
            } else {
                std::cmp::Ordering::Equal
            }
        });

        let mut merged = Vec::new();
        let mut current = sectors[0].clone();

        for next in sectors.into_iter().skip(1) {
            if let (ShadowGeometry::Sector { start_angle: curr_start, end_angle: curr_end, center: curr_center, radius: curr_radius },
                    ShadowGeometry::Sector { start_angle: next_start, end_angle: next_end, center: next_center, radius: next_radius }) 
                    = (&current.geometry, &next.geometry) {
                
                if curr_center.distance(*next_center) < 1.0 &&
                   (curr_radius - next_radius).abs() < 1.0 &&
                   Self::sectors_overlap_or_adjacent(*curr_start, *curr_end, *next_start, *next_end) {
                    
                    let merged_start = curr_start.min(*next_start);
                    let merged_end = curr_end.max(*next_end);
                    
                    current.geometry = ShadowGeometry::Sector {
                        center: *curr_center,
                        start_angle: merged_start,
                        end_angle: merged_end,
                        radius: curr_radius.max(*next_radius),
                    };
                    current.depth = current.depth.max(next.depth);
                } else {
                    merged.push(current);
                    current = next;
                }
            }
        }
        
        merged.push(current);
        merged
    }

    /// 檢查兩個扇形是否重疊或相鄰
    pub fn sectors_overlap_or_adjacent(start1: f32, end1: f32, start2: f32, end2: f32) -> bool {
        let normalize_angle = |mut angle: f32| {
            while angle < 0.0 { angle += 2.0 * std::f32::consts::PI; }
            while angle >= 2.0 * std::f32::consts::PI { angle -= 2.0 * std::f32::consts::PI; }
            angle
        };

        let s1 = normalize_angle(start1);
        let e1 = normalize_angle(end1);
        let s2 = normalize_angle(start2);
        let e2 = normalize_angle(end2);

        let gap_tolerance = 5.0_f32.to_radians();

        if s1 <= e1 && s2 <= e2 {
            (e1 + gap_tolerance >= s2 && s1 <= e2 + gap_tolerance)
        } else if s1 > e1 && s2 > e2 {
            true
        } else if s1 > e1 {
            (e1 + gap_tolerance >= s2) || (e2 + gap_tolerance >= s1)
        } else {
            (e2 + gap_tolerance >= s1) || (e1 + gap_tolerance >= s2)
        }
    }

    /// 計算可見區域
    pub fn calculate_visible_area(
        observer_pos: Vec2<f32>,
        radius: f32,
        shadows: &[ShadowArea],
    ) -> Vec<Vec2<f32>> {
        let mut visible_points = Vec::new();
        let precision = 360;
        let angle_step = 2.0 * std::f32::consts::PI / precision as f32;
        
        for i in 0..precision {
            let angle = i as f32 * angle_step;
            let direction = Vec2::new(angle.cos(), angle.sin());
            
            let mut max_distance = radius;
            for shadow in shadows {
                if let Some(intersection) = Self::ray_shadow_intersection(
                    observer_pos, direction, shadow
                ) {
                    max_distance = max_distance.min(intersection);
                }
            }
            
            visible_points.push(observer_pos + direction * max_distance);
        }
        
        visible_points
    }

    /// 射線與陰影相交測試
    pub fn ray_shadow_intersection(
        origin: Vec2<f32>,
        direction: Vec2<f32>,
        shadow: &ShadowArea,
    ) -> Option<f32> {
        match &shadow.geometry {
            ShadowGeometry::Sector { center, start_angle, end_angle, radius } => {
                let ray_angle = direction.y.atan2(direction.x);
                
                let normalize = |mut angle: f32| {
                    while angle < 0.0 { angle += 2.0 * std::f32::consts::PI; }
                    while angle >= 2.0 * std::f32::consts::PI { angle -= 2.0 * std::f32::consts::PI; }
                    angle
                };
                
                let norm_ray = normalize(ray_angle);
                let norm_start = normalize(*start_angle);
                let norm_end = normalize(*end_angle);
                
                let in_sector = if norm_start <= norm_end {
                    norm_ray >= norm_start && norm_ray <= norm_end
                } else {
                    norm_ray >= norm_start || norm_ray <= norm_end
                };
                
                if in_sector {
                    Some(*radius)
                } else {
                    None
                }
            },
            ShadowGeometry::Polygon { vertices } => {
                Self::ray_polygon_intersection(origin, direction, vertices)
            },
            ShadowGeometry::Trapezoid { vertices } => {
                Self::ray_polygon_intersection(origin, direction, &vertices.to_vec())
            },
        }
    }

    /// 射線與多邊形相交檢測
    pub fn ray_polygon_intersection(
        origin: Vec2<f32>,
        direction: Vec2<f32>,
        vertices: &[Vec2<f32>],
    ) -> Option<f32> {
        if vertices.len() < 3 {
            return None;
        }

        let mut closest_distance = f32::INFINITY;
        let mut found_intersection = false;

        for i in 0..vertices.len() {
            let v1 = vertices[i];
            let v2 = vertices[(i + 1) % vertices.len()];
            
            if let Some(distance) = Self::ray_line_intersection(origin, direction, v1, v2) {
                if distance > 0.0 && distance < closest_distance {
                    closest_distance = distance;
                    found_intersection = true;
                }
            }
        }

        if found_intersection {
            Some(closest_distance)
        } else {
            None
        }
    }

    /// 射線與線段相交檢測
    pub fn ray_line_intersection(
        ray_origin: Vec2<f32>,
        ray_direction: Vec2<f32>,
        line_start: Vec2<f32>,
        line_end: Vec2<f32>,
    ) -> Option<f32> {
        let line_direction = line_end - line_start;
        let cross = ray_direction.x * line_direction.y - ray_direction.y * line_direction.x;
        
        if cross.abs() < 1e-6 {
            return None;
        }

        let to_line_start = line_start - ray_origin;
        let t = (to_line_start.x * line_direction.y - to_line_start.y * line_direction.x) / cross;
        let u = (to_line_start.x * ray_direction.y - to_line_start.y * ray_direction.x) / cross;

        if t >= 0.0 && u >= 0.0 && u <= 1.0 {
            Some(t)
        } else {
            None
        }
    }
}