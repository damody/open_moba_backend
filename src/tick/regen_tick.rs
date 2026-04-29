//! Regen tick：每 tick 對有 CProperty 的單位算 HP regen（mana 儲存另案）。
//!
//! 讀 `UnitStats::hp_regen(base=0, e)` 取所有 buff 加總的 regen 值（Dota
//! HEALTH_REGEN_CONSTANT + HEALTH_REGEN_PERCENTAGE × HP_REGEN_AMPLIFY_PERCENTAGE）。
//! DISABLE_HEALING buff 存在時直接 0。
//! 建築物（IsBuilding）仍參與 HP regen（只有位移、復活、視野等 key 跳過）。
//! 只對 `cp.hp > 0` 的單位 tick；死亡後 regen 暫停。

use crossbeam_channel::Sender;
use rayon::prelude::*;
use serde_json::json;
use specs::{shred, Entities, Join, Read, ReadStorage, SystemData, Write, WriteStorage, World};

use crate::ability_runtime::{BuffStore, UnitStats};
use crate::comp::*;
use crate::transport::OutboundMsg;
use omb_script_abi::stat_keys::StatKey;

#[derive(SystemData)]
pub struct RegenTickData<'a> {
    _entities: Entities<'a>,
    dt: Read<'a, DeltaTime>,
    buffs: Read<'a, BuffStore>,
    is_buildings: ReadStorage<'a, IsBuilding>,
    cpropertys: WriteStorage<'a, CProperty>,
    creeps: ReadStorage<'a, Creep>,
    heroes: ReadStorage<'a, Hero>,
    mqtx: Write<'a, Vec<Sender<OutboundMsg>>>,
}

#[derive(Default)]
pub struct Sys {
    /// 累積 dt 達 0.25s 才觸發一次 regen 計算（4 Hz），降低每 frame 跑 1000 creep 的壓力。
    /// 觸發時用累積值當 effective dt，總回血量不變。
    dt_acc: f32,
}

const REGEN_INTERVAL: f32 = 0.25;

impl<'a> System<'a> for Sys {
    type SystemData = RegenTickData<'a>;

    const NAME: &'static str = "regen";

    fn run(job: &mut Job<Self>, mut data: Self::SystemData) {
        job.own.dt_acc += data.dt.0;
        if job.own.dt_acc < REGEN_INTERVAL {
            return;
        }
        let dt = std::mem::replace(&mut job.own.dt_acc, 0.0);
        let tx = data.mqtx.get(0).cloned();

        // 候選 entity：身上至少有一條 buff 含 HP regen 相關 key（任一）。
        // stress map 預期空集合 → 整個 system 跳過。
        use std::collections::HashSet;
        let candidates: HashSet<specs::Entity> = [
            StatKey::HealthRegenConstant.as_str(),
            StatKey::HealthRegenPercentage.as_str(),
            StatKey::HpRegenAmplifyPercentage.as_str(),
        ]
        .iter()
        .flat_map(|k| data.buffs.entities_with_key(k))
        .collect();

        if candidates.is_empty() {
            return;
        }

        // 平行計算（read-only） + 序列寫回。Stress map 走不到這（candidates 已 early return）。
        const PAR_MIN: usize = 32;

        // 顯式捕獲 borrows，讓 closure 是 Send 並可在 par_iter 中使用。
        // `cpropertys` 在這階段只讀（writeback 在下一階段做）。
        let cp_storage = &data.cpropertys;
        let creeps = &data.creeps;
        let heroes = &data.heroes;
        let is_buildings = &data.is_buildings;
        let buffs: &BuffStore = &data.buffs;

        let compute = |&e: &specs::Entity| -> Option<(specs::Entity, f32, f32)> {
            // 確認 entity 有 CProperty（creep / hero / 召喚物都有）
            let cp = cp_storage.get(e)?;
            if cp.hp <= 0.0 {
                return None;
            }
            // creep / hero 之外的 entity（例：純塔）不算 regen
            let is_creep = creeps.get(e).is_some();
            let is_hero = heroes.get(e).is_some();
            if !is_creep && !is_hero {
                return None;
            }
            let stats = UnitStats::from_refs(buffs, is_buildings.get(e).is_some());
            let regen = stats.hp_regen(0.0, e);
            if regen.abs() < 0.0001 {
                return None;
            }
            let eff_max = cp.mhp + stats.max_hp_bonus(e);
            let new_hp = (cp.hp + regen * dt).clamp(0.0, eff_max);
            if (new_hp - cp.hp).abs() > 0.01 {
                Some((e, new_hp, eff_max))
            } else {
                None
            }
        };

        let candidates_vec: Vec<specs::Entity> = candidates.into_iter().collect();
        let hp_updates: Vec<(specs::Entity, f32, f32)> = if candidates_vec.len() >= PAR_MIN {
            candidates_vec.par_iter().filter_map(compute).collect()
        } else {
            candidates_vec.iter().filter_map(compute).collect()
        };

        // 寫回 CProperty
        for (e, new_hp, _mhp) in &hp_updates {
            if let Some(cp) = data.cpropertys.get_mut(*e) {
                cp.hp = *new_hp;
            }
        }

        // 廣播 HP 更新給前端（少量時才每 tick 送；太頻繁可改 throttling）
        if let Some(ref t) = tx {
            for (e, new_hp, mhp) in hp_updates {
                let _ = t.try_send(make_hp_update(e.id(), new_hp, mhp));
            }
        }
    }
}

/// Build an entity.H OutboundMsg (prost CreepHp under kcp).
#[inline]
fn make_hp_update(id: u32, hp: f32, max_hp: f32) -> OutboundMsg {
    #[cfg(feature = "kcp")]
    {
        use crate::state::resource_management::proto_build;
        use crate::transport::TypedOutbound;
        // P5: HP regen without position — AoiGrid resolves entity_id → pos.
        OutboundMsg::new_typed_aoi_entity(
            "td/all/res", "entity", "H",
            TypedOutbound::CreepHp(proto_build::creep_hp(id, hp)),
            json!({ "id": id, "hp": hp, "max_hp": max_hp }),
            id as u64,
        )
    }
    #[cfg(not(feature = "kcp"))]
    {
        OutboundMsg::new_s("td/all/res", "entity", "H",
            json!({ "id": id, "hp": hp, "max_hp": max_hp }))
    }
}
