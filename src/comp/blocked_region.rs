//! 不可通行多邊形區域。
//!
//! 來源：`map.json` 的 `BlockedRegions` 欄位，由 `state::initialization` 載入。
//!
//! 碰撞路徑：啟動時用 `blocker_circles_for_polygon` 把每個 polygon 近似成
//! **大圓填內部 + 小圓貼邊界** 的一組 (中心, 半徑) pair，建成靜態 blocker ECS
//! entities（含 `Pos + CollisionRadius + RegionBlocker`），推進 Searcher 的 `region`
//! 索引。移動 tick 透過 `search_collidable` 做圓對圓查詢，統一成單一空間索引路徑。
//!
//! Polygon 原始資料仍留作前端視覺 payload（`("map","regions")` 事件）。

use specs::{Component, NullStorage};
use vek::Vec2;

use crate::util::geometry::{point_in_polygon, point_segment_dist_sq};

#[derive(Debug, Clone)]
pub struct BlockedRegion {
    pub name: String,
    pub points: Vec<Vec2<f32>>,
}

#[derive(Default, Debug, Clone)]
pub struct BlockedRegions(pub Vec<BlockedRegion>);

/// Marker component：標示該 entity 是 region 填充出的 blocker。
/// 不參與 hero/creep/tower 各別 join，因此其他 tick 不會誤抓。
#[derive(Default, Debug, Clone)]
pub struct RegionBlocker;

impl Component for RegionBlocker {
    type Storage = NullStorage<Self>;
}

/// 大圓（填充 polygon 內部）。只在距邊 ≥ BIG_RADIUS 處擺放，保證不超出邊界。
pub const BLOCKER_BIG_RADIUS: f32 = 60.0;
/// 大圓格距。= 半徑 → 相鄰大圓高度重疊，保證無縫隙。
pub const BLOCKER_BIG_SPACING: f32 = 60.0;
/// 小圓（沿邊界）。半徑與小兵同量級；向內法線方向縮，剛好內切邊。
pub const BLOCKER_SMALL_RADIUS: f32 = 35.0;
/// 小圓沿邊取樣間距。
pub const BLOCKER_SMALL_SPACING: f32 = 30.0;

/// 回傳 polygon 近似成一組圓：`(中心, 半徑)`。
/// - 內部：以 `BIG_SPACING` 網格放 `BIG_RADIUS` 圓，條件是中心距邊 ≥ BIG_RADIUS
/// - 邊界：沿邊取樣、向內法線縮 `SMALL_RADIUS`；半徑動態 clamp 到中心到邊的距離
///   （corner/窄段會自動縮小，確保不外擴）
pub fn blocker_circles_for_polygon(poly: &[Vec2<f32>]) -> Vec<(Vec2<f32>, f32)> {
    if poly.len() < 3 {
        return Vec::new();
    }
    let (mut min_x, mut max_x) = (f32::INFINITY, f32::NEG_INFINITY);
    let (mut min_y, mut max_y) = (f32::INFINITY, f32::NEG_INFINITY);
    for p in poly {
        if p.x < min_x { min_x = p.x; }
        if p.x > max_x { max_x = p.x; }
        if p.y < min_y { min_y = p.y; }
        if p.y > max_y { max_y = p.y; }
    }

    let mut out: Vec<(Vec2<f32>, f32)> = Vec::new();

    // 1. 內部大圓：只放在距邊 ≥ BIG_RADIUS 的格點，避免外擴
    let mut y = min_y + BLOCKER_BIG_RADIUS;
    while y <= max_y - BLOCKER_BIG_RADIUS + 0.001 {
        let mut x = min_x + BLOCKER_BIG_RADIUS;
        while x <= max_x - BLOCKER_BIG_RADIUS + 0.001 {
            let p = Vec2::new(x, y);
            if point_in_polygon(p, poly) && min_edge_dist(p, poly) >= BLOCKER_BIG_RADIUS {
                out.push((p, BLOCKER_BIG_RADIUS));
            }
            x += BLOCKER_BIG_SPACING;
        }
        y += BLOCKER_BIG_SPACING;
    }

    // 2. 邊界小圓：沿每條邊取樣 → 往內法線方向縮 SMALL_RADIUS → 半徑 clamp 到實際距邊
    let n = poly.len();
    for i in 0..n {
        let a = poly[i];
        let b = poly[(i + 1) % n];
        let edge = b - a;
        let len = edge.magnitude();
        if len < f32::EPSILON { continue; }
        let dir = edge / len;
        // 兩個候選法線（邊方向 rotate 90°），選能把中心推進 polygon 的那個
        let normal_a = Vec2::new(-dir.y, dir.x);
        let normal_b = -normal_a;

        let steps = (len / BLOCKER_SMALL_SPACING).ceil().max(1.0) as usize;
        for s in 0..=steps {
            let t = (s as f32) / (steps as f32);
            let edge_pt = a + edge * t;
            let cand_a = edge_pt + normal_a * BLOCKER_SMALL_RADIUS;
            let cand_b = edge_pt + normal_b * BLOCKER_SMALL_RADIUS;
            let center = if point_in_polygon(cand_a, poly) { cand_a }
                         else if point_in_polygon(cand_b, poly) { cand_b }
                         else { edge_pt };
            // clamp 半徑到 center 與邊的距離，避免 corner 處外擴
            let eff_r = min_edge_dist(center, poly).min(BLOCKER_SMALL_RADIUS);
            if eff_r > 2.0 {
                out.push((center, eff_r));
            }
        }
    }
    out
}

/// 點到 polygon 最近邊的距離（回傳非負值，含邊界等於 0）。
fn min_edge_dist(p: Vec2<f32>, poly: &[Vec2<f32>]) -> f32 {
    let n = poly.len();
    let mut min_d2 = f32::INFINITY;
    for i in 0..n {
        let a = poly[i];
        let b = poly[(i + 1) % n];
        let d2 = point_segment_dist_sq(p, a, b);
        if d2 < min_d2 { min_d2 = d2; }
    }
    min_d2.sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square_100() -> Vec<Vec2<f32>> {
        vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(100.0, 0.0),
            Vec2::new(100.0, 100.0),
            Vec2::new(0.0, 100.0),
        ]
    }

    fn rect_300() -> Vec<Vec2<f32>> {
        vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(300.0, 0.0),
            Vec2::new(300.0, 300.0),
            Vec2::new(0.0, 300.0),
        ]
    }

    #[test]
    fn empty_for_degenerate() {
        assert!(blocker_circles_for_polygon(&[]).is_empty());
        assert!(blocker_circles_for_polygon(&[
            Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0)
        ]).is_empty());
    }

    #[test]
    fn no_outward_extension() {
        // 任何 blocker 圓不得超出 polygon 邊界：center 距邊的距離必須 ≥ radius
        for poly in [square_100(), rect_300()] {
            let circles = blocker_circles_for_polygon(&poly);
            for (c, r) in &circles {
                let d = min_edge_dist(*c, &poly);
                assert!(
                    d + 0.01 >= *r,
                    "circle ({},{}) r={} extends outward; min_edge_dist={}",
                    c.x, c.y, r, d
                );
            }
        }
    }

    #[test]
    fn covers_interior_small_poly() {
        // 100x100 polygon（小於 BIG_RADIUS × 2）只靠小圓覆蓋內部中心
        let circles = blocker_circles_for_polygon(&square_100());
        let target = Vec2::new(50.0_f32, 50.0);
        let covered = circles.iter().any(|(c, r)| {
            (*c - target).magnitude_squared() < r * r
        });
        assert!(covered, "center uncovered; {} circles", circles.len());
    }

    #[test]
    fn covers_edge() {
        let circles = blocker_circles_for_polygon(&square_100());
        let target = Vec2::new(0.0_f32, 50.0);
        let covered = circles.iter().any(|(c, r)| {
            (*c - target).magnitude_squared() <= r * r + 0.1
        });
        assert!(covered, "edge point uncovered");
    }

    #[test]
    fn big_circles_used_for_large_poly() {
        // 300x300 polygon 應該有多個大圓填內部
        let circles = blocker_circles_for_polygon(&rect_300());
        let big_count = circles.iter()
            .filter(|(_, r)| (*r - BLOCKER_BIG_RADIUS).abs() < 0.01)
            .count();
        assert!(big_count >= 4, "expected ≥4 big circles for 300x300, got {}", big_count);
    }

    #[test]
    fn count_reasonable() {
        // 100x100：只有小圓，數量不爆
        let n = blocker_circles_for_polygon(&square_100()).len();
        assert!(n >= 8 && n < 100, "small poly circle count off: {}", n);
        // 300x300：大小圓混合；仍在合理範圍
        let n = blocker_circles_for_polygon(&rect_300()).len();
        assert!(n >= 20 && n < 300, "large poly circle count off: {}", n);
    }
}
