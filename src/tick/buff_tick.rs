//! Buff tick：每 tick 呼叫 `BuffStore::tick(dt)` 倒數、移除過期項。
//!
//! 取代舊 `slow_buff_tick`；所有 buff 統一走 `ability_runtime::BuffStore`。
//! 過期 buff 若 payload 含 `move_speed_bonus` 且 target 還活著且是 Creep →
//! 廣播 `creep/S { id, move_speed }` 讓 client 重算 lerp（buff_id 不再限定
//! "slow"，但 slow buff 採單一 instance 設計：buff_id = "slow"，由 payload
//! 內的 `slow_factor` 欄位驅動「強蓋弱」比較，多次命中只 refresh duration）。
//!
//! **DoT (Task 15)**：payload 含 `dot_damage` 的 buff 每秒對 target 扣 HP。
//! 以 1 秒累計槽 (`dot_accum: f32`) 控制頻率，累積到 1s 時觸發一次整批 dot。

use crossbeam_channel::Sender;
use omb_script_abi::stat_keys::StatKey;
use serde_json::json;
use specs::{shred, Read, ReadStorage, SystemData, Write, World};

use crate::ability_runtime::{BuffStore, UnitStats};
use crate::comp::*;
use crate::scripting::{ScriptEvent, ScriptEventQueue};
use crate::transport::OutboundMsg;

/// 位移類 payload key — 任一存在於過期 buff 的 payload 就要重算 creep 移速並廣播 `creep/S`。
/// 對應 Dota MOVESPEED_BONUS_* / MOVESPEED_ABSOLUTE / MIN / MAX / LIMIT。
const MOVESPEED_PAYLOAD_KEYS: &[StatKey] = &[
    StatKey::MoveSpeedBonus,
    StatKey::MoveSpeedBonusEquipment,
    StatKey::MoveSpeedBonusBuff,
    StatKey::MoveSpeedBaseOverride,
    StatKey::MoveSpeedBonusPercentage,
    StatKey::MoveSpeedBonusPercentageUnique,
    StatKey::MoveSpeedBonusPercentageUnique2,
    StatKey::MoveSpeedAbsolute,
    StatKey::MoveSpeedAbsoluteMin,
    StatKey::MoveSpeedLimit,
    StatKey::MoveSpeedMax,
];

#[derive(SystemData)]
pub struct BuffTickData<'a> {
    dt: Read<'a, DeltaTime>,
    buffs: Write<'a, BuffStore>,
    creeps: ReadStorage<'a, Creep>,
    cpropertys: specs::WriteStorage<'a, CProperty>,
    is_buildings: ReadStorage<'a, IsBuilding>,
    mqtx: Write<'a, Vec<Sender<OutboundMsg>>>,
    script_events: Write<'a, ScriptEventQueue>,
}

#[derive(Default)]
pub struct Sys;

impl<'a> System<'a> for Sys {
    type SystemData = BuffTickData<'a>;

    const NAME: &'static str = "buff";

    fn run(_job: &mut Job<Self>, mut data: Self::SystemData) {
        // TODO Phase 1[c]: drop conversion when BuffStore goes Fixed32-native.
        let dt = data.dt.0.to_f32_for_render();
        let expired = data.buffs.tick(dt);
        let tx = data.mqtx.get(0).cloned();

        // DoT (Task 15)：連續扣血，每 tick dot_damage * dt，達 dot/s 持續傷害
        // 累積到單次廣播避免每 tick 刷 creep/H。
        // 用 entities_by_key 反向索引取候選，避免對全表 entity 都呼 sum_add。
        let dot_targets: Vec<(specs::Entity, f32)> = data
            .buffs
            .entities_with_key(StatKey::DotDamage.as_str())
            .filter_map(|e| {
                let d = data.buffs.sum_add(e, StatKey::DotDamage);
                if d > 0.0 { Some((e, d)) } else { None }
            })
            .collect();
        for (entity, dot) in dot_targets {
            if let Some(cp) = data.cpropertys.get_mut(entity) {
                cp.hp = (cp.hp - dot * dt).max(0.0);
                if let Some(t) = tx.as_ref() {
                    let msg_type = if data.creeps.get(entity).is_some() { "creep" } else { "entity" };
                    let _ = t.try_send(make_hp_update(msg_type, entity.id(), cp.hp, cp.mhp));
                }
            }
        }

        for (entity, _buff_id, payload) in expired {
            // 每條過期 buff push ModifierRemoved 事件，讓腳本的 on_modifier_removed
            // 能 hook 到（例：某 stacking debuff 過期時補一個 refresh buff）。
            data.script_events.push(ScriptEvent::ModifierRemoved {
                e: entity,
                modifier_id: _buff_id.clone(),
            });

            // 若 payload 任一 key 屬於位移類 → 對 creep 重算 effective 並廣播 creep/S。
            // 用 UnitStats 套完整 Dota 公式（而非舊的 clamp 0.01-1.0）。
            let touches_movespeed = MOVESPEED_PAYLOAD_KEYS
                .iter()
                .any(|k| payload.get(k.as_str()).is_some());
            if touches_movespeed {
                let is_creep = data.creeps.get(entity).is_some();
                if is_creep {
                    if let (Some(ref t), Some(cp)) = (&tx, data.cpropertys.get(entity)) {
                        let stats = UnitStats::from_refs(
                            &*data.buffs,
                            data.is_buildings.get(entity).is_some(),
                        );
                        let effective = stats.final_move_speed(cp.msd, entity);
                        let _ = t.try_send(make_creep_slow(entity.id(), effective));
                    }
                }
            }
        }
    }
}

/// Build an HP-update OutboundMsg (creep/entity/hero/unit.H) — prost CreepHp under kcp.
/// `msg_type` is preserved on the wire via `GameEvent.msg_type` for shim routing.
#[inline]
fn make_hp_update(msg_type: &str, id: u32, hp: f32, max_hp: f32) -> OutboundMsg {
    #[cfg(feature = "kcp")]
    {
        use crate::state::resource_management::proto_build;
        use crate::transport::TypedOutbound;
        // P9: stamp the kind so client shim routes ("hero"/"creep"/"unit"/"entity", "H").
        let entity_kind = match msg_type {
            "hero" => proto_build::EntityKind::Hero,
            "unit" => proto_build::EntityKind::Unit,
            "tower" => proto_build::EntityKind::Tower,
            "creep" => proto_build::EntityKind::Creep,
            _ => proto_build::EntityKind::Entity,
        };
        // P5: DoT HP ticks use AoiEntity so only players seeing the creep pay bandwidth.
        OutboundMsg::new_typed_aoi_entity(
            "td/all/res", msg_type, "H",
            TypedOutbound::CreepHp(proto_build::creep_hp_with_kind(id, hp, entity_kind)),
            json!({ "id": id, "hp": hp, "max_hp": max_hp }),
            id as u64,
        )
    }
    #[cfg(not(feature = "kcp"))]
    {
        OutboundMsg::new_s("td/all/res", msg_type, "H",
            json!({ "id": id, "hp": hp, "max_hp": max_hp }))
    }
}

/// Build a creep.S OutboundMsg (prost CreepSlow under kcp).
#[inline]
fn make_creep_slow(id: u32, move_speed: f32) -> OutboundMsg {
    #[cfg(feature = "kcp")]
    {
        use crate::state::resource_management::proto_build;
        use crate::transport::TypedOutbound;
        OutboundMsg::new_typed_aoi_entity(
            "td/all/res", "creep", "S",
            TypedOutbound::CreepSlow(proto_build::creep_slow(id, move_speed)),
            json!({ "id": id, "move_speed": move_speed }),
            id as u64,
        )
    }
    #[cfg(not(feature = "kcp"))]
    {
        OutboundMsg::new_s("td/all/res", "creep", "S",
            json!({ "id": id, "move_speed": move_speed }))
    }
}
