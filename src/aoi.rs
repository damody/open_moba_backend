//! P5: Per-player AOI (Area-Of-Interest) broadphase.
//!
//! Used by the transport broadcast thread to resolve `BroadcastPolicy::AoiEntity`
//! into a `(x, y)` coordinate without touching specs storage (the transport
//! thread holds no `World`). The grid is a lightweight, cache-friendly bucket
//! lookup keyed by `(cell_x, cell_y)`, rebuilt each tick from the same pre-gather
//! that heartbeat already does in `State::send_heartbeat`.
//!
//! Design notes:
//! - `AOI_CELL_SIZE = 256.0` matches the viewport half-sizes used by omfx
//!   (typical pad region is ~1024x768 so each viewport touches 16-32 cells —
//!   small enough for a linear scan, large enough to keep `cells` shallow).
//! - `rebuild` replaces the grid contents per tick; entity churn (spawn/death)
//!   is naturally handled without separate add/remove bookkeeping.
//! - `positions` is kept as a flat `HashMap<u64, (f32,f32)>` for O(1) lookups
//!   by `entity_id` (the policy fast path). The `cells` map exists for future
//!   radius queries (`query(center, radius, cb)`) which the transport doesn't
//!   currently call but will need for broadphase radius / VFX fan-out in P6.
//!
//! The resource is `Arc<Mutex<AoiGrid>>` so the transport thread (tokio) can
//! share read-only access with the game loop (rayon). Cheap: rebuild is
//! essentially a reallocation of two flat maps.

use hashbrown::HashMap;

/// Side length of a single AOI cell in game-world units.
/// 256 chosen so a 1024x768 viewport touches ≤ 16 cells (4x3 plus padding).
pub const AOI_CELL_SIZE: f32 = 256.0;

/// A single entity's position snapshot inside a cell.
#[derive(Copy, Clone, Debug)]
pub struct AoiEntry {
    pub entity_id: u64,
    pub pos: (f32, f32),
}

/// AOI broadphase grid. Rebuilt per tick from a pre-gathered iterator.
///
/// `cells` bucket entities by integer `(cell_x, cell_y)` for future radius
/// queries. `positions` is a flat entity_id → (x,y) lookup hot path —
/// `BroadcastPolicy::AoiEntity` hits exactly this.
#[derive(Default, Debug)]
pub struct AoiGrid {
    cells: HashMap<(i32, i32), Vec<AoiEntry>>,
    positions: HashMap<u64, (f32, f32)>,
}

impl AoiGrid {
    pub fn new() -> Self {
        Self {
            cells: HashMap::new(),
            positions: HashMap::new(),
        }
    }

    /// Replace grid contents in a single pass. Caller provides all alive
    /// entities with (id, pos) — typically collected during the same iteration
    /// that heartbeat / visibility diff already does, so cost is amortized.
    pub fn rebuild(&mut self, entries: impl IntoIterator<Item = AoiEntry>) {
        self.cells.clear();
        self.positions.clear();
        for entry in entries {
            self.positions.insert(entry.entity_id, entry.pos);
            let key = Self::cell_key(entry.pos);
            self.cells.entry(key).or_insert_with(Vec::new).push(entry);
        }
    }

    /// O(1) position lookup by entity_id. Returns `None` if the entity wasn't
    /// in the last rebuild snapshot (e.g. spawned this tick after rebuild, or
    /// already dead). The transport policy handler treats `None` as "broadcast
    /// to all sessions" — safer than silently dropping the event.
    #[inline]
    pub fn lookup_pos(&self, entity_id: u64) -> Option<(f32, f32)> {
        self.positions.get(&entity_id).copied()
    }

    /// Radius query: call `cb(entity_id)` for every entity within `radius`
    /// of `center`. Used for VFX fan-out / AOI splash policies (reserved for
    /// P6; not currently invoked by the broadcast thread).
    pub fn query<F: FnMut(u64)>(&self, center: (f32, f32), radius: f32, mut cb: F) {
        let r2 = radius * radius;
        let (min_cx, min_cy) = Self::cell_key((center.0 - radius, center.1 - radius));
        let (max_cx, max_cy) = Self::cell_key((center.0 + radius, center.1 + radius));
        for cx in min_cx..=max_cx {
            for cy in min_cy..=max_cy {
                let Some(bucket) = self.cells.get(&(cx, cy)) else { continue };
                for e in bucket {
                    let dx = e.pos.0 - center.0;
                    let dy = e.pos.1 - center.1;
                    if dx * dx + dy * dy <= r2 {
                        cb(e.entity_id);
                    }
                }
            }
        }
    }

    /// Current entity count. Useful for diagnostics / tests.
    pub fn len(&self) -> usize {
        self.positions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }

    #[inline]
    fn cell_key(pos: (f32, f32)) -> (i32, i32) {
        let cx = (pos.0 / AOI_CELL_SIZE).floor() as i32;
        let cy = (pos.1 / AOI_CELL_SIZE).floor() as i32;
        (cx, cy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u64, x: f32, y: f32) -> AoiEntry {
        AoiEntry { entity_id: id, pos: (x, y) }
    }

    #[test]
    fn rebuild_inserts_positions() {
        let mut g = AoiGrid::new();
        g.rebuild([
            entry(1, 10.0, 20.0),
            entry(2, 300.0, 400.0),
        ]);
        assert_eq!(g.lookup_pos(1), Some((10.0, 20.0)));
        assert_eq!(g.lookup_pos(2), Some((300.0, 400.0)));
        assert_eq!(g.lookup_pos(99), None);
        assert_eq!(g.len(), 2);
    }

    #[test]
    fn rebuild_replaces_previous_contents() {
        let mut g = AoiGrid::new();
        g.rebuild([entry(1, 10.0, 20.0)]);
        g.rebuild([entry(2, 50.0, 60.0)]);
        assert_eq!(g.lookup_pos(1), None, "stale entry must be evicted");
        assert_eq!(g.lookup_pos(2), Some((50.0, 60.0)));
    }

    #[test]
    fn query_hits_entities_within_radius() {
        let mut g = AoiGrid::new();
        g.rebuild([
            entry(1, 100.0, 100.0),   // inside radius from (120,120)
            entry(2, 130.0, 130.0),   // inside
            entry(3, 500.0, 500.0),   // outside
            entry(4, 120.0, 120.0),   // at center
        ]);
        let mut hits: Vec<u64> = Vec::new();
        g.query((120.0, 120.0), 50.0, |id| hits.push(id));
        hits.sort();
        assert_eq!(hits, vec![1, 2, 4]);
    }

    #[test]
    fn query_empty_grid_yields_nothing() {
        let g = AoiGrid::new();
        let mut hits: Vec<u64> = Vec::new();
        g.query((0.0, 0.0), 1000.0, |id| hits.push(id));
        assert!(hits.is_empty());
    }

    #[test]
    fn cell_size_partitions_correctly() {
        // Cell key for pos (0,0) vs (AOI_CELL_SIZE, AOI_CELL_SIZE) must differ.
        let mut g = AoiGrid::new();
        g.rebuild([
            entry(1, 0.0, 0.0),
            entry(2, AOI_CELL_SIZE + 1.0, AOI_CELL_SIZE + 1.0),
        ]);
        // A tight radius around (0,0) only finds entity 1.
        let mut hits: Vec<u64> = Vec::new();
        g.query((0.0, 0.0), 10.0, |id| hits.push(id));
        assert_eq!(hits, vec![1]);
    }

    #[test]
    fn negative_coordinates_are_supported() {
        let mut g = AoiGrid::new();
        g.rebuild([
            entry(1, -300.0, -300.0),
            entry(2, 300.0, 300.0),
        ]);
        assert_eq!(g.lookup_pos(1), Some((-300.0, -300.0)));
        let mut hits: Vec<u64> = Vec::new();
        g.query((-290.0, -290.0), 100.0, |id| hits.push(id));
        assert_eq!(hits, vec![1]);
    }
}
