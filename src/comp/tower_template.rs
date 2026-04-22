//! TD 塔 spawn 輔助 + SlowBuff component。
//!
//! 塔的所有靜態屬性（cost/atk/range/footprint/label/...）由腳本 `tower_metadata()`
//! 回報、host 在 `load_scripts()` 結束時填 `TowerTemplateRegistry` resource。
//! 這支模組只負責「拿 unit_id 到 registry 查 template → 建 entity」。
//!
//! 舊的硬編 `TowerKind` enum 與 `TowerTemplate` 已在 PR-5 移除：
//! 之後新增第 5 種塔只要寫新腳本 + 重 build DLL，host 不用改。

use serde::{Deserialize, Serialize};
use specs::{Builder, Component, Entity, VecStorage, World, WorldExt};
use vek::Vec2;

use super::*;

/// 減速 debuff：命中的 creep 在 `remaining` 秒內，移動速度乘上 `factor`。
/// 由 `projectile_tick` 在命中有 slow_factor>0 的 projectile 時 push `Outcome::ApplySlow`，
/// GameProcessor 再附加這個 Component；`slow_buff_tick` 每 tick 扣 remaining，歸零移除。
#[derive(Clone, Copy, Debug)]
pub struct SlowBuff {
    pub factor: f32,
    pub remaining: f32,
}

impl Component for SlowBuff {
    type Storage = VecStorage<Self>;
}

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

    let tprop = TProperty::new(tpl.hp, 0, 120.0);
    let tatk = TAttack::new(tpl.atk, tpl.asd_interval, tpl.range, tpl.bullet_speed);
    let faction = Faction::new(FactionType::Player, 0);
    let vision = CircularVision::new(tpl.range + 100.0, 40.0).with_precision(120);
    let cprop = CProperty {
        hp: tpl.hp,
        mhp: tpl.hp,
        msd: 0.0,
        def_physic: 0.0,
        def_magic: 0.0,
    };

    let entity = world
        .create_entity()
        .with(Pos(pos))
        .with(Tower::new())
        .with(tprop)
        .with(cprop)
        .with(tatk)
        .with(faction)
        .with(vision)
        .with(Facing(0.0))
        .with(TurnSpeed(tpl.turn_speed_deg.to_radians()))
        .with(CollisionRadius(tpl.footprint))
        .with(crate::scripting::ScriptUnitTag { unit_id: unit_id.to_string() })
        .build();

    // 排入 Spawn 事件，讓腳本 on_spawn 初始化 stats（atk/asd/range 等）
    world.write_resource::<crate::scripting::ScriptEventQueue>()
        .push(crate::scripting::ScriptEvent::Spawn { e: entity });

    Some(entity)
}
