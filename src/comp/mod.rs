pub mod phys;
pub mod resources;
pub mod state;
pub mod campaign_manager;
pub mod mqtt_handler;
pub mod game_processor;
pub use game_processor::GameProcessor;
pub mod tick_profile;
pub use tick_profile::{TickProfile, Phase as TickPhase};
pub mod last;
pub mod outcome;
pub mod projectile;
pub mod attack;
pub mod ecs;
pub mod base;
pub mod tower;
pub mod clock;
pub mod creep;
pub mod check_point;
pub mod player;
pub mod hero;
pub mod enemy;
pub mod campaign;
pub mod unit;
pub mod damage;
pub mod heightmap;
pub mod circular_vision;
pub mod circular_vision_refactored;
pub mod vision;
pub mod outcome_system;
pub mod gold;
pub mod inventory;
pub mod item_effects;
pub mod is_base;
pub mod bounty;
pub mod facing;
pub mod blocked_region;
pub mod game_mode;
pub mod tower_template;
pub mod tower_registry;

pub use self::{
    blocked_region::*,
    game_mode::*,
    tower_template::*,
    // tower_registry 不 glob export，避免與 tower_template::TowerTemplate 命名衝突
    gold::*,
    inventory::*,
    item_effects::*,
    is_base::*,
    bounty::*,
    facing::*,
    phys::*,
    resources::*,
    state::*,
    last::*,
    outcome::*,
    projectile::*,
    attack::*,
    player::*,
    ecs::*,
    base::*,
    tower::*,
    creep::*,
    clock::*,
    check_point::*,
    hero::*,
    enemy::*,
    campaign::*,
    unit::*,
    damage::*,
    heightmap::*,
    circular_vision::*,
};