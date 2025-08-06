use vek::Vec2;

pub struct GeometryUtils;

impl GeometryUtils {
    /// 檢查角度是否在扇形範圍內
    pub fn angle_in_sector(angle: f32, start_angle: f32, end_angle: f32) -> bool {
        let normalize_angle = |mut a: f32| {
            while a < 0.0 { a += 2.0 * std::f32::consts::PI; }
            while a >= 2.0 * std::f32::consts::PI { a -= 2.0 * std::f32::consts::PI; }
            a
        };

        let norm_angle = normalize_angle(angle);
        let norm_start = normalize_angle(start_angle);
        let norm_end = normalize_angle(end_angle);

        if norm_start <= norm_end {
            norm_angle >= norm_start && norm_angle <= norm_end
        } else {
            norm_angle >= norm_start || norm_angle <= norm_end
        }
    }

    /// 射線與扇形相交檢測（改進版本）
    pub fn ray_sector_intersection_improved(
        origin: Vec2<f32>,
        direction: Vec2<f32>,
        center: Vec2<f32>,
        start_angle: f32,
        end_angle: f32,
        radius: f32,
    ) -> Option<f32> {
        // 首先檢查射線是否與圓相交
        let to_center = center - origin;
        let proj_length = to_center.dot(direction);
        
        // 射線背向圓心
        if proj_length < 0.0 {
            return None;
        }
        
        let closest_point = origin + direction * proj_length;
        let distance_to_center = center.distance(closest_point);
        
        // 射線與圓不相交
        if distance_to_center > radius {
            return None;
        }
        
        // 計算相交點
        let half_chord = (radius * radius - distance_to_center * distance_to_center).sqrt();
        let intersection_distance = proj_length - half_chord;
        
        if intersection_distance < 0.0 {
            return None;
        }
        
        let intersection_point = origin + direction * intersection_distance;
        let intersection_angle = (intersection_point - center).y.atan2((intersection_point - center).x);
        
        // 計算第二個交點
        let second_intersection_distance = proj_length + half_chord;
        let second_intersection_point = origin + direction * second_intersection_distance;
        let second_intersection_angle = (second_intersection_point - center).y.atan2((second_intersection_point - center).x);
        
        // 檢查兩個交點，返回最近的在扇形內的交點
        let first_in_sector = Self::angle_in_sector(intersection_angle, start_angle, end_angle);
        let second_in_sector = Self::angle_in_sector(second_intersection_angle, start_angle, end_angle);
        
        if first_in_sector && intersection_distance >= 0.0 {
            Some(intersection_distance)
        } else if second_in_sector {
            Some(second_intersection_distance)
        } else {
            None
        }
    }

    /// 計算兩點間距離
    pub fn distance(p1: Vec2<f32>, p2: Vec2<f32>) -> f32 {
        (p1 - p2).magnitude()
    }

    /// 計算點到線段的最短距離
    pub fn point_to_line_distance(point: Vec2<f32>, line_start: Vec2<f32>, line_end: Vec2<f32>) -> f32 {
        let line_vec = line_end - line_start;
        let point_vec = point - line_start;
        
        let line_length_sq = line_vec.magnitude_squared();
        if line_length_sq == 0.0 {
            return point_vec.magnitude();
        }
        
        let t = (point_vec.dot(line_vec) / line_length_sq).max(0.0).min(1.0);
        let projection = line_start + line_vec * t;
        (point - projection).magnitude()
    }

    /// 檢查點是否在圓形內
    pub fn point_in_circle(point: Vec2<f32>, center: Vec2<f32>, radius: f32) -> bool {
        Self::distance(point, center) <= radius
    }

    /// 檢查點是否在矩形內
    pub fn point_in_rectangle(point: Vec2<f32>, min: Vec2<f32>, max: Vec2<f32>) -> bool {
        point.x >= min.x && point.x <= max.x &&
        point.y >= min.y && point.y <= max.y
    }

    /// 計算角度差（考慮環形性質）
    pub fn angle_difference(angle1: f32, angle2: f32) -> f32 {
        let diff = angle2 - angle1;
        let pi2 = 2.0 * std::f32::consts::PI;
        
        if diff > std::f32::consts::PI {
            diff - pi2
        } else if diff < -std::f32::consts::PI {
            diff + pi2
        } else {
            diff
        }
    }

    /// 標準化角度到 [0, 2π) 範圍
    pub fn normalize_angle(mut angle: f32) -> f32 {
        while angle < 0.0 { angle += 2.0 * std::f32::consts::PI; }
        while angle >= 2.0 * std::f32::consts::PI { angle -= 2.0 * std::f32::consts::PI; }
        angle
    }

    /// 線段相交檢測
    pub fn line_segments_intersect(
        p1: Vec2<f32>, q1: Vec2<f32>,
        p2: Vec2<f32>, q2: Vec2<f32>,
    ) -> Option<Vec2<f32>> {
        let d1 = q1 - p1;
        let d2 = q2 - p2;
        let cross = d1.x * d2.y - d1.y * d2.x;
        
        if cross.abs() < 1e-6 {
            return None; // 平行線段
        }
        
        let t1 = ((p2.x - p1.x) * d2.y - (p2.y - p1.y) * d2.x) / cross;
        let t2 = ((p2.x - p1.x) * d1.y - (p2.y - p1.y) * d1.x) / cross;
        
        if t1 >= 0.0 && t1 <= 1.0 && t2 >= 0.0 && t2 <= 1.0 {
            Some(p1 + d1 * t1)
        } else {
            None
        }
    }
}