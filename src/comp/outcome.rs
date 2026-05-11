pub use omoba_core::runtime::comp::{
    AttackCancelFx, AttackCancelFxQueue, AttackCancelPhase, AttackPhaseFx, AttackPhaseFxQueue,
    CreepData, DisIndex, ExplosionFx, ExplosionFxQueue, Outcome, RemovedEntitiesQueue, Searcher,
    TowerData, TowerFireFx, TowerFireFxQueue,
};
use omoba_core::runtime::SpatialIndexParams;

pub fn searcher_from_config() -> Searcher {
    use crate::config::vision_config::COLLISION_CONFIG;

    let cfg = &*COLLISION_CONFIG;
    let params = SpatialIndexParams {
        quadtree_max_depth: cfg.QUADTREE_MAX_DEPTH,
        quadtree_max_per_node: cfg.QUADTREE_MAX_PER_NODE,
        hash_grid_cell_size: cfg.SHG_CELL_SIZE,
        bvh_max_leaf: cfg.BVH_MAX_LEAF,
    };
    Searcher::from_index_kinds(
        &cfg.SPATIAL_INDEX_TOWER,
        &cfg.SPATIAL_INDEX_CREEP,
        &cfg.SPATIAL_INDEX_HERO,
        &cfg.SPATIAL_INDEX_REGION,
        params,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tower_fire_fx_queue_drains_once() {
        let mut queue = TowerFireFxQueue::default();
        queue.pending.push(TowerFireFx {
            entity_id: 7,
            entity_gen: 1,
            spawn_tick: 42,
            dir_rad: 0.25,
        });

        let drained = std::mem::take(&mut queue.pending);
        assert_eq!(drained.len(), 1);
        assert!(queue.pending.is_empty());
        let drained_again = std::mem::take(&mut queue.pending);
        assert!(drained_again.is_empty());
    }

    #[test]
    fn attack_phase_fx_queue_drains_once_and_keeps_sequence_resource() {
        let mut queue = AttackPhaseFxQueue::default();
        queue.next_seq = 5;
        queue.pending.push(AttackPhaseFx {
            entity_id: 7,
            entity_gen: 1,
            spawn_tick: 42,
            attack_seq: 4,
            is_critical: false,
            windup_ms: 120,
            impact_at_ms: 120,
            backswing_ms: 240,
            dir_rad: 0.25,
            target_entity_id: Some(99),
            target_pos_x: None,
            target_pos_y: None,
        });

        let drained = std::mem::take(&mut queue.pending);
        assert_eq!(drained.len(), 1);
        assert!(queue.pending.is_empty());
        assert_eq!(queue.next_seq, 5);
        let drained_again = std::mem::take(&mut queue.pending);
        assert!(drained_again.is_empty());
    }

    #[test]
    fn attack_cancel_fx_queue_drains_once() {
        let mut queue = AttackCancelFxQueue::default();
        queue.pending.push(AttackCancelFx {
            entity_id: 7,
            entity_gen: 1,
            spawn_tick: 42,
            attack_seq: 4,
            phase: AttackCancelPhase::Windup,
            impact_committed: false,
        });

        let drained = std::mem::take(&mut queue.pending);
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].phase, AttackCancelPhase::Windup);
        assert!(!drained[0].impact_committed);
        assert!(queue.pending.is_empty());
        let drained_again = std::mem::take(&mut queue.pending);
        assert!(drained_again.is_empty());
    }
}
