//! TD 塔 spawn 輔助。
//!
//! 塔的所有靜態屬性（cost/atk/range/footprint/label/...）由腳本 `tower_metadata()`
//! 回報、host 在 `load_scripts()` 結束時填 `TowerTemplateRegistry` resource。
//! 這支模組只負責「拿 unit_id 到 registry 查 template → 建 entity」。
//!
//! 舊的硬編 `TowerKind` enum 與 `TowerTemplate` 已在 PR-5 移除。
//! `SlowBuff` component 已被統一 `BuffStore` 取代（buff_id="slow"）。

use serde::{Deserialize, Serialize};
use specs::{Builder, Component, Entity, World, WorldExt};
use vek::Vec2;

use super::*;

/// 在指定位置依 `unit_id` spawn 一座 TD 塔。
/// 從 `TowerTemplateRegistry` resource 查 template；找不到就回 None（log warning）。
/// 同時 push `ScriptEvent::Spawn` 讓腳本的 `on_spawn` 下一個 tick 跑到。
pub fn spawn_td_tower(world: &mut World, pos: Vec2<f32>, unit_id: &str) -> Option<Entity> {
    let tpl = {
        let reg = world.read_resource::<crate::comp::tower_registry::TowerTemplateRegistry>();
        reg.get(unit_id).cloned()
    };
    let Some(tpl) = tpl else {
        log::warn!("spawn_td_tower: unknown unit_id '{}'", unit_id);
        return None;
    };

    // Phase 1c.4: TProperty / TAttack / CProperty 全 Fixed32（Phase 1c.2）。
    // TowerTemplate 仍 f32（Phase 1d）。Bridge once per spawn.
    use omoba_sim::Fixed32;
    let f32_to_fx = |v: f32| Fixed32::from_raw((v * omoba_sim::fixed::SCALE as f32) as i32);
    let tpl_hp = f32_to_fx(tpl.hp);
    let tprop = TProperty::new(tpl_hp, 0, Fixed32::from_i32(120));
    let tatk = TAttack::new(
        f32_to_fx(tpl.atk),
        f32_to_fx(tpl.asd_interval),
        f32_to_fx(tpl.range),
        f32_to_fx(tpl.bullet_speed),
    );
    let faction = Faction::new(FactionType::Player, 0);
    let vision = CircularVision::new(tpl.range + 100.0, 40.0).with_precision(120);
    let cprop = CProperty {
        hp: tpl_hp,
        mhp: tpl_hp,
        msd: Fixed32::ZERO,
        def_physic: Fixed32::ZERO,
        def_magic: Fixed32::ZERO,
    };

    let entity = world
        .create_entity()
        .with(Pos::from_xy_f32(pos.x, pos.y))
        .with(Tower::new())
        .with(IsBuilding)
        .with(tprop)
        .with(cprop)
        .with(tatk)
        .with(faction)
        .with(vision)
        .with(Facing(omoba_sim::Angle::ZERO))
        .with(crate::comp::FacingBroadcast(None))
        // PHASE 2: tower template metadata still uses turn_speed_deg f32; redesign in Phase 2 KCP tag rework.
        .with(TurnSpeed(omoba_sim::Fixed32::from_raw((tpl.turn_speed_deg.to_radians() * 1024.0) as i32)))
        .with(CollisionRadius(omoba_sim::Fixed32::from_raw((tpl.footprint * 1024.0) as i32)))
        .with(crate::scripting::ScriptUnitTag { unit_id: unit_id.to_string() })
        .build();

    // 排入 Spawn 事件，讓腳本 on_spawn 初始化 stats（atk/asd/range 等）
    world.write_resource::<crate::scripting::ScriptEventQueue>()
        .push(crate::scripting::ScriptEvent::Spawn { e: entity });

    // 標記 Searcher.tower 索引髒污 → 下個 nearby_tick 重建。
    // 否則 collision system 看不到塔，creep / hero 會穿過塔。
    world.write_resource::<crate::comp::outcome::Searcher>().tower.mark_dirty();

    Some(entity)
}
