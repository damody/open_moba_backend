pub mod campaign_manager;
pub mod game_processor;
pub mod mqtt_handler;
pub mod state;
pub use game_processor::GameProcessor;
pub use omoba_core::comp::tick_profile;
pub use omoba_core::comp::{TickPhase, TickProfile};
pub mod attack;
pub mod base;
pub mod campaign;
pub mod circular_vision;
pub mod circular_vision_refactored;
pub mod clock;
pub mod collision_index;
pub mod ecs;
pub mod enemy;
pub mod outcome;
pub mod outcome_system;
pub mod player;
pub mod vision;
pub use collision_index::CollisionIndex;
pub mod tower_template;

pub use omoba_core::runtime::comp::{
    blocked_region, bounty, building, check_point, creep, creep_move_broadcast, damage, facing,
    game_mode, gold, heightmap, hero, inventory, is_base, item_effects, last, lockstep_resources,
    phys, projectile, resources, tower, tower_registry, tower_upgrade_registry,
    tower_upgrade_rules, unit,
};
pub use omoba_core::runtime::comp::{CreepMoveBroadcast, IsBuilding};

pub use omoba_core::runtime::comp::{
    blocked_region::*, bounty::*, building::*, check_point::*, creep::*, creep_move_broadcast::*,
    damage::*, facing::*, game_mode::*, gold::*, heightmap::*, hero::*, inventory::*, is_base::*,
    item_effects::*, last::*, lockstep_resources::*, phys::*, projectile::*, resources::*,
    tower::*, tower_registry::*, tower_upgrade_registry::*, tower_upgrade_rules::*, unit::*,
};

pub use self::{
    attack::*, base::*, campaign::*, circular_vision::*, clock::*, ecs::*, enemy::*, outcome::*,
    player::*, state::*, tower_template::*,
};
