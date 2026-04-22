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

    // 3. 頂點角落圓：沿內角平分線偏移，填補邊緣取樣跳過端點造成的角落空洞
    //    放 **3 層** 圓（近、中、遠），保證頂點附近任何方向的 hero 逼近都會被擋。
    //    位移距離 d = SMALL_R × factor / sin(θ/2) 讓圓內切兩條相鄰邊。
    for i in 0..n {
        let v = poly[i];
        let prev = poly[(i + n - 1) % n];
        let next = poly[(i + 1) % n];
        let pv_len = (v - prev).magnitude();
        let vn_len = (next - v).magnitude();
        if pv_len < f32::EPSILON || vn_len < f32::EPSILON { continue; }
        let u = (v - prev) / pv_len;           // 從 prev 指向 V
        let w = (next - v) / vn_len;           // 從 V 指向 next
        // 內角 θ：cos(θ) = -u·w  ⇒  sin(θ/2) = sqrt((1 + u·w)/2)
        let dot = u.x * w.x + u.y * w.y;
        let half = ((1.0 + dot) * 0.5).max(0.01); // 避免 ÷0（極銳角）
        let s = half.sqrt();
        // 內角平分線 = 兩條邊左法線之和的方向
        let n_u = Vec2::new(-u.y, u.x);
        let n_w = Vec2::new(-w.y, w.x);
        let bi = n_u + n_w;
        let bi_len = bi.magnitude();
        if bi_len < f32::EPSILON { continue; } // 兩邊共線（180° 頂點）
        let bisector = bi / bi_len;
        // 先決定 bisector 正負（哪個方向是 polygon 內部）
        let test_d = (BLOCKER_SMALL_RADIUS / s).min(BLOCKER_SMALL_RADIUS * 2.5);
        let inward = if point_in_polygon(v + bisector * test_d, poly) { bisector }
                     else if point_in_polygon(v - bisector * test_d, poly) { -bisector }
                     else { continue };
        // 3 層：靠近頂點（~0.5×）→ 內切（1.0×）→ 略遠（1.5×）
        for factor in [0.55_f32, 1.0, 1.5] {
            let d = (BLOCKER_SMALL_RADIUS * factor / s).min(BLOCKER_SMALL_RADIUS * 3.0);
            let center = v + inward * d;
            if !point_in_polygon(center, poly) { continue; }
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

    /// 一個銳角三角形（最銳角約 30°），驗證銳角頂點仍會被 corner 圓覆蓋
    fn acute_triangle() -> Vec<Vec2<f32>> {
        vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(400.0, 0.0),
            Vec2::new(50.0, 200.0), // 銳角頂點
        ]
    }

    /// 實際 MVP_1 map 的 region_0 polygon
    fn mvp1_region0() -> Vec<Vec2<f32>> {
        vec![
            Vec2::new(-596.0, 1264.0),  // V0
            Vec2::new(-480.0, 832.0),   // V1
            Vec2::new(1340.0, 1144.0),  // V2（右側）
            Vec2::new(1084.0, 1876.0),  // V3（右側）
        ]
    }

    #[test]
    fn mvp1_all_four_corners_have_circle() {
        // 4 個頂點每個都必須有一個 circle 放在距該頂點 < BLOCKER_SMALL_RADIUS × 3 範圍內
        let poly = mvp1_region0();
        let circles = blocker_circles_for_polygon(&poly);
        let thresh = BLOCKER_SMALL_RADIUS * 3.0;
        let mut missing: Vec<usize> = Vec::new();
        for (i, v) in poly.iter().enumerate() {
            let has_nearby = circles.iter().any(|(c, _)| (*c - *v).magnitude() < thresh);
            if !has_nearby { missing.push(i); }
        }
        assert!(missing.is_empty(),
            "頂點 {:?} 沒有 corner circle 在 {} 範圍內（總 circles = {}）",
            missing, thresh, circles.len());
    }

    #[test]
    fn mvp1_every_edge_point_blocks_hero() {
        // 沿 polygon 邊界每 20 單位取一個點，把它往內推 hero_r = 30 單位
        // （模擬「英雄中心剛進入 polygon 30 單位內」的位置），驗證每個都被 blocker 擋住
        let poly = mvp1_region0();
        let circles = blocker_circles_for_polygon(&poly);
        let hero_r = 30.0_f32;
        let n = poly.len();
        let mut failures: Vec<(Vec2<f32>, f32)> = Vec::new();
        for i in 0..n {
            let a = poly[i];
            let b = poly[(i + 1) % n];
            let edge = b - a;
            let len = edge.magnitude();
            let dir = edge / len;
            let normal_a = Vec2::new(-dir.y, dir.x);
            let normal_b = -normal_a;
            let mut t = 0.0_f32;
            while t <= len {
                let p = a + dir * t;
                // 測兩個法線哪個在內部
                let cand_a = p + normal_a * hero_r;
                let cand_b = p + normal_b * hero_r;
                let hero_pos = if point_in_polygon(cand_a, &poly) { cand_a }
                               else if point_in_polygon(cand_b, &poly) { cand_b }
                               else { t += 20.0; continue };
                let hit = circles.iter().any(|(c, r)| {
                    let d2 = (*c - hero_pos).magnitude_squared();
                    let touch = hero_r + r;
                    d2 < touch * touch
                });
                if !hit {
                    // 找最近的 blocker 看差多少
                    let min_margin = circles.iter()
                        .map(|(c, r)| {
                            let d = (*c - hero_pos).magnitude();
                            (hero_r + r) - d  // 正=重疊；負=差距
                        })
                        .fold(f32::NEG_INFINITY, f32::max);
                    failures.push((hero_pos, min_margin));
                }
                t += 20.0;
            }
        }
        assert!(failures.is_empty(),
            "{} 個 edge-inner probe 未被任何 blocker 擋住：{:?}",
            failures.len(),
            failures.iter().take(5).collect::<Vec<_>>()
        );
    }

    #[test]
    fn mvp1_boundary_transition_blocks_hero() {
        // 找到 polygon 邊界上對 V2/V3 最近的進入路徑，驗證英雄從外推進到邊界時
        // 至少有一個 blocker 讓 `circle_hits_units` 觸發（模擬實際碰撞檢查）。
        let circles = blocker_circles_for_polygon(&mvp1_region0());
        let hero_r = 30.0_f32;
        // 對 V2/V3 各做一個放射狀掃描：從頂點外側逼近，每步 5 單位，
        // 一旦進入 polygon 就要被 blocker 擋住
        let poly = mvp1_region0();
        let targets = [
            ("V2", Vec2::new(1340.0_f32, 1144.0)),
            ("V3", Vec2::new(1084.0_f32, 1876.0)),
        ];
        for (name, vertex) in targets {
            // 逼近方向：從外部沿內角平分線反方向走到 vertex，再往內走
            // 取 polygon 重心作為「內部方向」參考
            let centroid = Vec2::new(
                (poly.iter().map(|p| p.x).sum::<f32>()) / poly.len() as f32,
                (poly.iter().map(|p| p.y).sum::<f32>()) / poly.len() as f32,
            );
            let inward = (centroid - vertex).normalized();
            for step in 0..30 {
                let offset = step as f32 * 5.0;
                let hero_pos = vertex + inward * offset;
                if !point_in_polygon(hero_pos, &poly) { continue; }
                // 距 vertex < hero_r*2 的範圍屬「英雄碰到頂點」之內，應該被擋
                let d_vertex = (hero_pos - vertex).magnitude();
                if d_vertex > hero_r * 2.0 { break; }
                let hit = circles.iter().any(|(c, r)| {
                    let d2 = (*c - hero_pos).magnitude_squared();
                    let touch = hero_r + r;
                    d2 < touch * touch
                });
                assert!(hit,
                    "{} 附近 hero@({:.0},{:.0}) 離頂點 {:.0} 單位 卻無 blocker 覆蓋",
                    name, hero_pos.x, hero_pos.y, d_vertex);
            }
        }
    }

    #[test]
    fn mvp1_right_vertices_block_hero() {
        // 用戶回報：右側兩個頂點（V2/V3）可以跑進去。
        // 對 V2 的每個「剛進入 polygon 邊界」接近位置測試。
        let circles = blocker_circles_for_polygon(&mvp1_region0());
        let hero_r = 30.0_f32;

        // V2 = (1340, 1144) 的內角方向接近 (-0.863, 0.508)（基於計算）
        // 測試英雄從外往內走，進入 polygon 第一格（最靠近 V2 的 polygon 內部點）
        let probes_v2 = [
            Vec2::new(1280.0_f32, 1150.0),
            Vec2::new(1290.0_f32, 1160.0),
            Vec2::new(1300.0_f32, 1150.0),
            Vec2::new(1310.0_f32, 1145.0),
        ];
        let probes_v3 = [
            Vec2::new(1050.0_f32, 1820.0),
            Vec2::new(1070.0_f32, 1830.0),
            Vec2::new(1080.0_f32, 1850.0),
            Vec2::new(1060.0_f32, 1800.0),
        ];
        for hero_pos in probes_v2.iter().chain(probes_v3.iter()) {
            // 只對 polygon 內部的 probe 做驗證
            if !point_in_polygon(*hero_pos, &mvp1_region0()) { continue; }
            let hit = circles.iter().any(|(c, r)| {
                let d2 = (*c - *hero_pos).magnitude_squared();
                let touch = hero_r + r;
                d2 < touch * touch
            });
            assert!(hit, "hero@({},{}) 在 V2/V3 邊界內部無 blocker 覆蓋 ({} circles total)",
                hero_pos.x, hero_pos.y, circles.len());
        }
    }

    #[test]
    fn corner_blocks_hero() {
        // 英雄 (r=30) 站在 polygon 內角附近應該與某 blocker 圓重疊（collision）
        // 這是這次用戶回報「頂點處穿透」的 regression test
        for (poly, hero) in [
            (square_100(), Vec2::new(3.0_f32, 3.0)),
            (square_100(), Vec2::new(97.0_f32, 3.0)),
            (rect_300(), Vec2::new(5.0_f32, 5.0)),
            (rect_300(), Vec2::new(295.0_f32, 5.0)),
            (rect_300(), Vec2::new(5.0_f32, 295.0)),
            (rect_300(), Vec2::new(295.0_f32, 295.0)),
            (acute_triangle(), Vec2::new(55.0_f32, 195.0)), // 近銳角頂點
        ] {
            let circles = blocker_circles_for_polygon(&poly);
            let hero_r = 30.0_f32;
            let hit = circles.iter().any(|(c, r)| {
                let d2 = (*c - hero).magnitude_squared();
                let touch = hero_r + r;
                d2 < touch * touch
            });
            assert!(hit, "hero @({},{}) 在角落沒碰到任何 blocker (circles={})",
                hero.x, hero.y, circles.len());
        }
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
