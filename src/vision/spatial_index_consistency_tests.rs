//! 跨 spatial-index impl 的等價性 + stress 測試。
//!
//! 四個 impl 在同樣的 obstacles + 同樣的 query 下，必須回傳完全相同的 id 集合
//! （順序可不同，用 BTreeSet 比對）。
//!
//! 這份測試是 SAH BVH / SHG / SAP 實作正確性的最強保證 — 直接拿 QuadTree 當 oracle。

#![cfg(test)]

use std::collections::BTreeSet;
use vek::Vec2;

use crate::comp::circular_vision::{ObstacleInfo, ObstacleProperties, ObstacleType};
use super::spatial_index::{Bounds, Entry, SpatialIndex, SpatialIndexParams, build_spatial_index};

const KINDS: &[&str] = &["quadtree", "hash_grid", "bvh", "sap"];

fn obs(x: f32, y: f32, r: f32) -> ObstacleInfo {
    ObstacleInfo {
        position: Vec2::new(x, y),
        obstacle_type: ObstacleType::Circular { radius: r },
        height: 10.0,
        properties: ObstacleProperties {
            blocks_completely: true,
            opacity: 1.0,
            shadow_multiplier: 1.0,
        },
    }
}

fn make_entry(id: &str, x: f32, y: f32, r: f32) -> Entry<String, ObstacleInfo> {
    Entry::new(id.to_string(), obs(x, y, r), Vec2::new(x, y), r)
}

fn world_bounds() -> Bounds {
    Bounds::new(Vec2::new(-2000.0, -2000.0), Vec2::new(2000.0, 2000.0))
}

fn id_set(results: &[Entry<String, ObstacleInfo>]) -> BTreeSet<String> {
    results.iter().map(|e| e.id.clone()).collect()
}

fn build(kind: &str) -> Box<dyn SpatialIndex<String, ObstacleInfo>> {
    build_spatial_index(kind, SpatialIndexParams::default())
}

/// 簡單 deterministic LCG，避免拉 rand crate
struct Lcg(u64);
impl Lcg {
    fn new(seed: u64) -> Self { Self(seed.max(1)) }
    fn next_u32(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 32) as u32
    }
    fn next_f32_range(&mut self, lo: f32, hi: f32) -> f32 {
        let u = (self.next_u32() as f32) / (u32::MAX as f32);
        lo + (hi - lo) * u
    }
    fn next_usize(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next_u32() as usize) % n }
    }
}

#[test]
fn four_impls_agree_on_static_scene() {
    let entries: Vec<Entry<String, ObstacleInfo>> = (0..30).map(|i| {
        let mut g = Lcg::new(1234 + i as u64);
        let x = g.next_f32_range(-1500.0, 1500.0);
        let y = g.next_f32_range(-1500.0, 1500.0);
        let r = g.next_f32_range(20.0, 100.0);
        make_entry(&format!("o{}", i), x, y, r)
    }).collect();

    let queries: Vec<(Vec2<f32>, f32)> = (0..20).map(|i| {
        let mut g = Lcg::new(9000 + i as u64);
        let cx = g.next_f32_range(-1500.0, 1500.0);
        let cy = g.next_f32_range(-1500.0, 1500.0);
        let r = g.next_f32_range(50.0, 500.0);
        (Vec2::new(cx, cy), r)
    }).collect();

    let mut sets_per_query: Vec<Vec<BTreeSet<String>>> = vec![Vec::new(); queries.len()];
    for &kind in KINDS {
        let mut idx = build(kind);
        idx.initialize(world_bounds(), entries.clone());
        for (qi, &(c, r)) in queries.iter().enumerate() {
            let s = id_set(&idx.query_in_range(c, r));
            sets_per_query[qi].push(s);
        }
    }

    for (qi, sets) in sets_per_query.iter().enumerate() {
        for w in sets.windows(2) {
            assert_eq!(w[0], w[1],
                "impls disagree on query #{}: {} = {:?}, other = {:?}",
                qi, KINDS[0], sets[0], w[1]);
        }
    }
}

#[test]
fn four_impls_agree_under_random_mutations() {
    let mut g = Lcg::new(42);
    let mut indices: Vec<Box<dyn SpatialIndex<String, ObstacleInfo>>> =
        KINDS.iter().map(|k| build(k)).collect();
    for idx in indices.iter_mut() {
        idx.initialize(world_bounds(), vec![]);
    }

    let mut active_ids: BTreeSet<String> = BTreeSet::new();

    // 初始 100 個 insert
    for i in 0..100 {
        let id = format!("o{}", i);
        let entry = make_entry(
            &id,
            g.next_f32_range(-1500.0, 1500.0),
            g.next_f32_range(-1500.0, 1500.0),
            g.next_f32_range(20.0, 80.0),
        );
        for idx in indices.iter_mut() {
            idx.insert(entry.clone());
        }
        active_ids.insert(id);
    }

    // 隨機 50 個 mutation
    for _ in 0..50 {
        match g.next_usize(3) {
            0 => {
                let id = format!("new{}", g.next_u32());
                let entry = make_entry(
                    &id,
                    g.next_f32_range(-1500.0, 1500.0),
                    g.next_f32_range(-1500.0, 1500.0),
                    g.next_f32_range(20.0, 80.0),
                );
                for idx in indices.iter_mut() {
                    idx.insert(entry.clone());
                }
                active_ids.insert(id);
            }
            1 => {
                if !active_ids.is_empty() {
                    let id = active_ids.iter().nth(g.next_usize(active_ids.len())).cloned().unwrap();
                    let mut removed_flags = Vec::new();
                    for idx in indices.iter_mut() {
                        removed_flags.push(idx.remove(&id));
                    }
                    assert!(removed_flags.windows(2).all(|w| w[0] == w[1]),
                        "remove({}) returned {:?} across impls", id, removed_flags);
                    active_ids.remove(&id);
                }
            }
            _ => {
                if !active_ids.is_empty() {
                    let id = active_ids.iter().nth(g.next_usize(active_ids.len())).cloned().unwrap();
                    let entry = make_entry(
                        &id,
                        g.next_f32_range(-1500.0, 1500.0),
                        g.next_f32_range(-1500.0, 1500.0),
                        g.next_f32_range(20.0, 80.0),
                    );
                    for idx in indices.iter_mut() {
                        idx.update(entry.clone());
                    }
                }
            }
        }
    }

    for q in 0..20 {
        let center = Vec2::new(g.next_f32_range(-1500.0, 1500.0), g.next_f32_range(-1500.0, 1500.0));
        let radius = g.next_f32_range(50.0, 500.0);
        let sets: Vec<BTreeSet<String>> = indices.iter()
            .map(|idx| id_set(&idx.query_in_range(center, radius)))
            .collect();
        for (i, w) in sets.windows(2).enumerate() {
            assert_eq!(w[0], w[1],
                "post-mutation query #{} (center={:?} r={}) disagree:\n  {} = {:?}\n  {} = {:?}",
                q, center, radius,
                indices[i].name(), sets[i],
                indices[i + 1].name(), w[1]);
        }
    }
}

#[test]
fn stress_1000_obstacles_four_impls_agree() {
    let mut g = Lcg::new(7);
    let entries: Vec<Entry<String, ObstacleInfo>> = (0..1000).map(|i| {
        let x = g.next_f32_range(-1900.0, 1900.0);
        let y = g.next_f32_range(-1900.0, 1900.0);
        let r = g.next_f32_range(10.0, 60.0);
        make_entry(&format!("s{}", i), x, y, r)
    }).collect();

    let mut indices: Vec<Box<dyn SpatialIndex<String, ObstacleInfo>>> =
        KINDS.iter().map(|k| build(k)).collect();
    for idx in indices.iter_mut() {
        idx.initialize(world_bounds(), entries.clone());
    }

    for q in 0..50 {
        let center = Vec2::new(g.next_f32_range(-1900.0, 1900.0), g.next_f32_range(-1900.0, 1900.0));
        let radius = g.next_f32_range(100.0, 800.0);
        let sets: Vec<BTreeSet<String>> = indices.iter()
            .map(|idx| id_set(&idx.query_in_range(center, radius)))
            .collect();
        for (i, w) in sets.windows(2).enumerate() {
            assert_eq!(w[0].len(), w[1].len(),
                "stress query #{} size mismatch: {} = {}, {} = {}",
                q,
                indices[i].name(), sets[i].len(),
                indices[i + 1].name(), w[1].len());
            assert_eq!(w[0], w[1], "stress query #{} id-set mismatch", q);
        }
    }
}

/// 對 (Entity-style Id=u64, Item=()) 也跑一次基本一致性，確認 generic 化後對非 String/ObstacleInfo
/// 的 (Id, Item) 配對也正常 — 這是 collision pre-detection 的 smoke test。
#[test]
fn entity_keyed_indexes_basic_consistency() {
    use super::spatial_index::build_entity_index;
    use specs::{World, WorldExt, Builder};

    let mut world = World::new();
    let mut entries: Vec<Entry<specs::Entity, ()>> = Vec::new();
    let mut g = Lcg::new(123);
    for i in 0..50 {
        let e = world.create_entity().build();
        let x = g.next_f32_range(-1500.0, 1500.0);
        let y = g.next_f32_range(-1500.0, 1500.0);
        entries.push(Entry::new(e, (), Vec2::new(x, y), 5.0));
        let _ = i;
    }

    let mut indices: Vec<Box<dyn SpatialIndex<specs::Entity, ()>>> = KINDS.iter()
        .map(|k| build_entity_index(k, SpatialIndexParams::default()))
        .collect();
    for idx in indices.iter_mut() {
        idx.initialize(world_bounds(), entries.clone());
    }

    // 5 query 對齊
    for q in 0..5 {
        let center = Vec2::new(g.next_f32_range(-1500.0, 1500.0), g.next_f32_range(-1500.0, 1500.0));
        let radius = g.next_f32_range(100.0, 800.0);
        let sets: Vec<BTreeSet<specs::Entity>> = indices.iter()
            .map(|idx| idx.query_in_range(center, radius).iter().map(|e| e.id).collect())
            .collect();
        for (i, w) in sets.windows(2).enumerate() {
            assert_eq!(w[0], w[1],
                "entity-keyed query #{} disagree: {} vs {}",
                q, indices[i].name(), indices[i + 1].name());
        }
    }
}
