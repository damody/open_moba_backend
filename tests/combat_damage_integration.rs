//! Combat damage integration test.
//!
//! Phase 4-5 (lockstep cleanup) regression coverage. The user reported
//! "creep HP bars stay full even though towers are firing". This test pins
//! the authoritative damage path: an `Outcome::Damage` enqueued into the
//! ECS `Vec<Outcome>` resource, drained through
//! `GameProcessor::process_outcomes`, must decrement the target's
//! `CProperty.hp` and emit a follow-up `Outcome::Death` once HP reaches 0.
//!
//! Avoids loading the script DLL (which would require `base_content.dll`
//! staged and the `omoba_template_ids` registry populated). Just constructs
//! a minimal World via `StateInitializer::setup_campaign_ecs_world`, spawns
//! source / target entities with `CProperty`, and exercises
//! `process_outcomes` directly. This isolates the *damage application*
//! logic — the part that handle_damage cuts in Phase 1.4 could have broken.

use crossbeam_channel::unbounded;
use omobab::comp::{CProperty, GameProcessor, Pos};
use omobab::transport::OutboundMsg;
use omobab::Outcome;
use omoba_sim::Fixed64;
use rayon::ThreadPoolBuilder;
use specs::{Builder, World, WorldExt};
use std::sync::Arc;

fn build_world() -> World {
    let pool = Arc::new(
        ThreadPoolBuilder::new()
            .num_threads(2)
            .build()
            .expect("rayon pool"),
    );
    let mut world = omobab::state::StateInitializer::setup_campaign_ecs_world(&pool);
    // get_entity_names path peeks at TowerTemplateRegistry; in production the
    // registry is populated by load_scripts. For damage-path unit testing we
    // just stash an empty default — `name_of` falls through to "Unknown".
    world.insert(omobab::comp::tower_registry::TowerTemplateRegistry::default());
    world
}

fn spawn_target(world: &mut World, hp: f32) -> specs::Entity {
    let hp_fx = Fixed64::from_raw((hp * 1024.0) as i64);
    let cprop = CProperty {
        hp: hp_fx,
        mhp: hp_fx,
        msd: Fixed64::ZERO,
        def_physic: Fixed64::ZERO,
        def_magic: Fixed64::ZERO,
    };
    world
        .create_entity()
        .with(Pos::from_xy_f32(0.0, 0.0))
        .with(cprop)
        .build()
}

fn spawn_source(world: &mut World) -> specs::Entity {
    world
        .create_entity()
        .with(Pos::from_xy_f32(100.0, 0.0))
        .build()
}

fn read_hp(world: &World, e: specs::Entity) -> f32 {
    world
        .read_storage::<CProperty>()
        .get(e)
        .map(|c| c.hp.to_f32_for_render())
        .unwrap_or(-1.0)
}

#[test]
fn damage_outcome_decrements_hp() {
    let mut world = build_world();
    let source = spawn_source(&mut world);
    let target = spawn_target(&mut world, 500.0);

    assert!((read_hp(&world, target) - 500.0).abs() < 0.5, "HP should start at 500");

    let phys = Fixed64::from_raw((30.0 * 1024.0) as i64);
    {
        let mut outcomes = world.write_resource::<Vec<Outcome>>();
        outcomes.push(Outcome::Damage {
            pos: omoba_sim::Vec2::ZERO,
            phys,
            magi: Fixed64::ZERO,
            real: Fixed64::ZERO,
            source,
            target,
            predeclared: false,
        });
    }

    let (tx, _rx) = unbounded::<OutboundMsg>();
    GameProcessor::process_outcomes(&mut world, &tx).expect("process_outcomes");
    world.maintain();

    let hp_after = read_hp(&world, target);
    assert!(
        (hp_after - 470.0).abs() < 0.5,
        "expected HP=470 after 30 damage, got {}",
        hp_after,
    );
}

#[test]
fn damage_aggregation_multiple_sources_one_target() {
    let mut world = build_world();
    let s1 = spawn_source(&mut world);
    let s2 = spawn_source(&mut world);
    let target = spawn_target(&mut world, 500.0);

    let phys_a = Fixed64::from_raw((20.0 * 1024.0) as i64);
    let phys_b = Fixed64::from_raw((35.0 * 1024.0) as i64);
    {
        let mut outcomes = world.write_resource::<Vec<Outcome>>();
        outcomes.push(Outcome::Damage {
            pos: omoba_sim::Vec2::ZERO,
            phys: phys_a,
            magi: Fixed64::ZERO,
            real: Fixed64::ZERO,
            source: s1,
            target,
            predeclared: false,
        });
        outcomes.push(Outcome::Damage {
            pos: omoba_sim::Vec2::ZERO,
            phys: phys_b,
            magi: Fixed64::ZERO,
            real: Fixed64::ZERO,
            source: s2,
            target,
            predeclared: false,
        });
    }

    let (tx, _rx) = unbounded::<OutboundMsg>();
    GameProcessor::process_outcomes(&mut world, &tx).expect("process_outcomes");
    world.maintain();

    let hp_after = read_hp(&world, target);
    assert!(
        (hp_after - 445.0).abs() < 0.5,
        "expected HP=445 after 20+35=55 damage, got {}",
        hp_after,
    );
}

#[test]
fn fatal_damage_emits_death_and_removes_entity() {
    let mut world = build_world();
    let source = spawn_source(&mut world);
    let target = spawn_target(&mut world, 50.0);

    let phys = Fixed64::from_raw((100.0 * 1024.0) as i64);
    {
        let mut outcomes = world.write_resource::<Vec<Outcome>>();
        outcomes.push(Outcome::Damage {
            pos: omoba_sim::Vec2::ZERO,
            phys,
            magi: Fixed64::ZERO,
            real: Fixed64::ZERO,
            source,
            target,
            predeclared: false,
        });
    }

    let (tx, _rx) = unbounded::<OutboundMsg>();
    // First call: damage clamps HP to 0 and queues Outcome::Death in
    // next_outcomes. The Death outcome is processed on the next call.
    GameProcessor::process_outcomes(&mut world, &tx).expect("first process_outcomes");
    world.maintain();
    GameProcessor::process_outcomes(&mut world, &tx).expect("second process_outcomes");
    world.maintain();

    // After second pass, the entity should be deleted (handle_death pushed
    // it onto remove_uids).
    assert!(
        !world.is_alive(target),
        "target entity should be dead/removed after fatal damage",
    );
}
