//! 戰鬥傷害綜合測試。
//!
//! 第 4-5 階段（鎖步清理）回歸覆蓋率。該用戶舉報
//! 「即使塔正在開火，蠕變 HP 條仍保持滿」。此測試引腳
//! 權威的損害路徑：「Outcome::Damage」排隊到
//! ECS `Vec<Outcome>` 資源，透過
//! `GameProcessor::process_outcomes`，必須遞減目標的
//! `CProperty.hp` 並在 HP 達到 0 時發出後續的 `Outcome::Death`。
//!
//! 避免載入腳本 DLL（這需要 `base_content.dll`
//! 已上演並填入「omoba_template_ids」註冊表）。只是構造
//! 透過 `StateInitializer::setup_campaign_ecs_world` 產生一個最小的世界
//! 具有“CProperty”的來源/目標實體以及練習
//! 直接“process_outcomes”。這隔離了*損壞應用程式*
//! 邏輯 — 在第 1.4 階段中，handle_damage 刪除的部分可能已經損壞。

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
    // get_entity_names 路徑查看 TowerTemplateRegistry；在生產中
    // 登錄由 load_scripts 填入。對於損壞路徑單元測試，我們
    // 只需隱藏一個空的預設值 - `name_of` 會變成「未知」。
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
    // 第一次呼叫：傷害將 HP 限制為 0 並排隊 Outcome::Death in
    // 下一個結果。死亡結果將在下一次呼叫時處理。
    GameProcessor::process_outcomes(&mut world, &tx).expect("first process_outcomes");
    world.maintain();
    GameProcessor::process_outcomes(&mut world, &tx).expect("second process_outcomes");
    world.maintain();

    // 第二遍之後，實體應該被刪除（handle_death Pushed
    // 它到remove_uids）。
    assert!(
        !world.is_alive(target),
        "target entity should be dead/removed after fatal damage",
    );
}
