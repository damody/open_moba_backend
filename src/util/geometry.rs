//! 2D 幾何工具：點-多邊形、圓-多邊形 相交測試。
//!
//! 用於 `BlockedRegions` 移動碰撞判定；map_editor 有一份完全相同演算法
//! (`map_editor/src/geometry.rs`) 以保證結果一致。

use vek::Vec2;

/// Ray-casting：點是否在多邊形（凹/凸皆可）內部。
/// 邊界上的點視為「內部」（方便擋得住單位）。
pub fn point_in_polygon(p: Vec2<f32>, poly: &[Vec2<f32>]) -> bool {
    if poly.len() < 3 {
        return false;
    }
    let mut inside = false;
    let n = poly.len();
    let mut j = n - 1;
    for i in 0..n {
        let pi = poly[i];
        let pj = poly[j];
        // 判斷水平射線是否穿過 edge(pj, pi)
        let cond = (pi.y > p.y) != (pj.y > p.y)
            && p.x < (pj.x - pi.x) * (p.y - pi.y) / (pj.y - pi.y + f32::EPSILON) + pi.x;
        if cond {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// 點到線段 (a-b) 的最短距離平方。
fn point_segment_dist_sq(p: Vec2<f32>, a: Vec2<f32>, b: Vec2<f32>) -> f32 {
    let ab = b - a;
    let ap = p - a;
    let len_sq = ab.x * ab.x + ab.y * ab.y;
    if len_sq < 1e-8 {
        return ap.magnitude_squared();
    }
    let t = (ap.x * ab.x + ap.y * ab.y) / len_sq;
    let t = t.clamp(0.0, 1.0);
    let proj = a + ab * t;
    (p - proj).magnitude_squared()
}

/// 圓（中心 center、半徑 r）是否與多邊形重疊：
/// 1) 圓心在多邊形內 → 是
/// 2) 任一邊距離圓心 < r → 是
pub fn circle_hits_polygon(center: Vec2<f32>, r: f32, poly: &[Vec2<f32>]) -> bool {
    if poly.len() < 3 {
        return false;
    }
    if point_in_polygon(center, poly) {
        return true;
    }
    let r2 = r * r;
    let n = poly.len();
    for i in 0..n {
        let a = poly[i];
        let b = poly[(i + 1) % n];
        if point_segment_dist_sq(center, a, b) < r2 {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Vec<Vec2<f32>> {
        vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(100.0, 0.0),
            Vec2::new(100.0, 100.0),
            Vec2::new(0.0, 100.0),
        ]
    }

    /// 一個 U 形凹多邊形（開口朝上）
    fn concave_u() -> Vec<Vec2<f32>> {
        vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(100.0, 0.0),
            Vec2::new(100.0, 100.0),
            Vec2::new(70.0, 100.0),
            Vec2::new(70.0, 30.0),
            Vec2::new(30.0, 30.0),
            Vec2::new(30.0, 100.0),
            Vec2::new(0.0, 100.0),
        ]
    }

    #[test]
    fn point_inside_convex() {
        assert!(point_in_polygon(Vec2::new(50.0, 50.0), &square()));
    }

    #[test]
    fn point_outside_convex() {
        assert!(!point_in_polygon(Vec2::new(200.0, 50.0), &square()));
    }

    #[test]
    fn point_in_concave_arms() {
        // 凹 U 的兩臂內部
        assert!(point_in_polygon(Vec2::new(15.0, 50.0), &concave_u()));
        assert!(point_in_polygon(Vec2::new(85.0, 50.0), &concave_u()));
        // 凹進去的開口處應在外
        assert!(!point_in_polygon(Vec2::new(50.0, 80.0), &concave_u()));
    }

    #[test]
    fn circle_inside() {
        assert!(circle_hits_polygon(Vec2::new(50.0, 50.0), 10.0, &square()));
    }

    #[test]
    fn circle_touches_edge() {
        assert!(circle_hits_polygon(Vec2::new(-5.0, 50.0), 10.0, &square()));
        assert!(!circle_hits_polygon(Vec2::new(-20.0, 50.0), 10.0, &square()));
    }

    #[test]
    fn circle_separate() {
        assert!(!circle_hits_polygon(Vec2::new(300.0, 300.0), 10.0, &square()));
    }
}
