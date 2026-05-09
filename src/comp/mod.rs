pub mod campaign_manager;
pub mod game_processor;
pub mod mqtt_handler;
pub mod phys;
pub mod resources;
pub mod state;
pub use game_processor::GameProcessor;
pub mod tick_profile;
pub use tick_profile::{Phase as TickPhase, TickProfile};
pub mod attack;
pub mod base;
pub mod campaign;
pub mod check_point;
pub mod circular_vision;
pub mod circular_vision_refactored;
pub mod clock;
pub mod collision_index;
pub mod creep;
pub mod damage;
pub mod ecs;
pub mod enemy;
pub mod gold;
pub mod heightmap;
pub mod hero;
pub mod last;
pub mod outcome;
pub mod outcome_system;
pub mod player;
pub mod projectile;
pub mod tower;
pub mod unit;
pub mod vision;
pub use collision_index::CollisionIndex;
pub mod blocked_region;
pub mod bounty;
pub mod building;
pub mod facing;
pub mod game_mode;
pub mod inventory;
pub mod is_base;
pub mod item_effects;
pub mod tower_registry;
pub mod tower_template;
pub mod tower_upgrade_registry;
pub mod tower_upgrade_rules;
pub use building::IsBuilding;
pub mod creep_move_broadcast;
pub use creep_move_broadcast::CreepMoveBroadcast;
pub mod lockstep_resources;
pub use lockstep_resources::{
    PendingAbilityCast, PendingAbilityCastQueue, PendingAbilityUpgrade, PendingAbilityUpgradeQueue,
    PendingItemUse, PendingItemUseQueue, PendingMoveQueue, PendingMoveTo, PendingPlayerInputs,
    PendingTowerSell, PendingTowerSellQueue, PendingTowerSpawn, PendingTowerSpawnQueue,
    PendingTowerUpgrade, PendingTowerUpgradeQueue, SnapshotStore,
};

pub use self::{
    attack::*,
    base::*,
    blocked_region::*,
    bounty::*,
    campaign::*,
    check_point::*,
    circular_vision::*,
    clock::*,
    creep::*,
    damage::*,
    ecs::*,
    enemy::*,
    facing::*,
    game_mode::*,
    // tower_registry 不 glob export，避免與 tower_template::TowerTemplate 命名衝突
    gold::*,
    heightmap::*,
    hero::*,
    inventory::*,
    is_base::*,
    item_effects::*,
    last::*,
    outcome::*,
    phys::*,
    player::*,
    projectile::*,
    resources::*,
    state::*,
    tower::*,
    tower_template::*,
    unit::*,
};
