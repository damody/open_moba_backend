use std::collections::BTreeMap;
use std::time::Instant;
use failure::Error;
use omb_script_abi::stat_keys::StatKey;
use serde_json::json;
use specs::{World, WorldExt, Entity, Builder, storage::{WriteStorage, ReadStorage}};

use crate::comp::*;
use crate::transport::OutboundMsg;
use crate::Outcome;
use crate::Projectile;

fn outcome_kind(o: &Outcome) -> &'static str {
    match o {
        Outcome::Damage { .. } => "Damage",
        Outcome::ProjectileLine2 { .. } => "ProjectileLine2",
        Outcome::Death { .. } => "Death",
        Outcome::Creep { .. } => "Creep",
        Outcome::CreepStop { .. } => "CreepStop",
        Outcome::CreepWalk { .. } => "CreepWalk",
        Outcome::Tower { .. } => "Tower",
        Outcome::Heal { .. } => "Heal",
        Outcome::UpdateAttack { .. } => "UpdateAttack",
        Outcome::GainExperience { .. } => "GainExperience",
        Outcome::SpawnUnit { .. } => "SpawnUnit",
        Outcome::CreepLeaked { .. } => "CreepLeaked",
        Outcome::AddBuff { .. } => "AddBuff",
        Outcome::Explosion { .. } => "Explosion",
        Outcome::ProjectileDirectional { .. } => "ProjectileDirectional",
    }
}

pub struct GameProcessor;

impl GameProcessor {
    pub fn process_outcomes(ecs: &mut World, mqtx: &crossbeam_channel::Sender<OutboundMsg>) -> Result<(), Error> {
        let mut remove_uids = vec![];
        let mut next_outcomes = vec![];
        let mut variant_timings: Vec<(&'static str, u128)> = Vec::new();

        {
            let mut ocs = ecs.get_mut::<Vec<Outcome>>().unwrap();
            let mut raw_outcomes = vec![];
            raw_outcomes.append(ocs);

            let damage_merge_start = Instant::now();
            let mut merged_damage_count: u64 = 0;
            let outcomes = {
                let mut first_dmg_idx: std::collections::HashMap<Entity, usize> =
                    std::collections::HashMap::new();
                let mut aggregated: Vec<Outcome> = Vec::with_capacity(raw_outcomes.len());
                for out in raw_outcomes {
                    if let Outcome::Damage {
                        phys: p, magi: m, real: r, target: t, ..
                    } = &out
                    {
                        if let Some(&idx) = first_dmg_idx.get(t) {
                            if let Outcome::Damage {
                                phys: ap, magi: am, real: ar, ..
                            } = &mut aggregated[idx]
                            {
                                *ap += *p;
                                *am += *m;
                                *ar += *r;
                                merged_damage_count += 1;
                                continue;
                            }
                        }
                        first_dmg_idx.insert(*t, aggregated.len());
                    }
                    aggregated.push(out);
                }
                aggregated
            };
            if merged_damage_count > 0 {
                variant_timings.push((
                    "DamageMerge",
                    damage_merge_start.elapsed().as_nanos(),
                ));
            }

            for out in outcomes {
                let kind = outcome_kind(&out);
                let t0 = Instant::now();
                match out {
                    Outcome::Death { pos: p, ent: e } => {
                        remove_uids.push(e);
                        Self::handle_death(ecs, &mut next_outcomes, mqtx, e)?;
                    }
                    Outcome::ProjectileLine2 { pos, source, target } => {
                        Self::handle_projectile(ecs, mqtx, pos, source, target)?;
                    }
                    Outcome::ProjectileDirectional { pos, source, end_pos } => {
                        Self::handle_projectile_directional(ecs, mqtx, pos, source, end_pos)?;
                    }
                    Outcome::Creep { cd } => {
                        Self::handle_creep_spawn(ecs, mqtx, cd)?;
                    }
                    Outcome::Tower { pos, td } => {
                        Self::handle_tower_spawn(ecs, mqtx, pos, td)?;
                    }
                    Outcome::CreepStop { source, target } => {
                        Self::handle_creep_stop(ecs, mqtx, source, target)?;
                    }
                    Outcome::CreepWalk { target } => {
                        Self::handle_creep_walk(ecs, target)?;
                    }
                    Outcome::Damage { pos, phys, magi, real, source, target } => {
                        Self::handle_damage(ecs, &mut next_outcomes, pos, phys, magi, real, source, target)?;
                    }
                    Outcome::Heal { pos, target, amount } => {
                        Self::handle_heal(ecs, target, amount)?;
                    }
                    Outcome::UpdateAttack { target, asd_count, cooldown_reset } => {
                        Self::handle_attack_update(ecs, target, asd_count, cooldown_reset)?;
                    }
                    Outcome::GainExperience { target, amount } => {
                        Self::handle_experience_gain(ecs, target, amount as u32)?;
                    }
                    Outcome::CreepLeaked { ent } => {
                        remove_uids.push(ent);
                        Self::handle_creep_leaked(ecs, mqtx, ent)?;
                    }
                    Outcome::AddBuff { target, buff_id, duration, payload } => {
                        Self::handle_add_buff(ecs, target, buff_id, duration, payload)?;
                    }
                    Outcome::Explosion { pos, radius, duration } => {
                        let _ = mqtx.try_send(OutboundMsg::new_s_at(
                            "td/all/res", "game", "explosion",
                            json!({
                                "x": pos.x,
                                "y": pos.y,
                                "radius": radius,
                                "duration": duration,
                            }),
                            pos.x, pos.y,
                        ));
                    }
                    _ => {}
                }
                variant_timings.push((kind, t0.elapsed().as_nanos()));
            }
        }

        ecs.delete_entities(&remove_uids[..]);
        ecs.write_resource::<Vec<Outcome>>().clear();
        ecs.write_resource::<Vec<Outcome>>().append(&mut next_outcomes);

        {
            let mut profile = ecs.write_resource::<TickProfile>();
            for (kind, ns) in variant_timings {
                profile.record_variant(kind, ns);
            }
        }

        Ok(())
    }
    
    fn handle_death(
        ecs: &mut World,
        next_outcomes: &mut Vec<Outcome>,
        mqtx: &crossbeam_channel::Sender<OutboundMsg>,
        entity: Entity
    ) -> Result<(), Error> {
        // 只有敵方基地死亡才算玩家勝（我方基地雖有 IsBase，不觸發勝負）
        let is_enemy_base = {
            let is_bases = ecs.read_storage::<IsBase>();
            let factions = ecs.read_storage::<Faction>();
            is_bases.get(entity).is_some()
                && factions
                    .get(entity)
                    .map(|f| f.faction_id == FactionType::Enemy)
                    .unwrap_or(false)
        };

        // 若死者有 Bounty → 將金錢/經驗分給最近的友方英雄
        Self::distribute_bounty(ecs, next_outcomes, mqtx, entity);

        let mut creeps = ecs.write_storage::<Creep>();
        let mut towers = ecs.write_storage::<Tower>();
        let mut projs = ecs.write_storage::<Projectile>();

        let entity_type = if let Some(c) = creeps.get_mut(entity) {
            if let Some(bt) = c.block_tower {
                if let Some(t) = towers.get_mut(bt) {
                    t.block_creeps.retain(|&x| x != entity);
                }
            }
            "creep"
        } else if let Some(t) = towers.get_mut(entity) {
            for ce in t.block_creeps.iter() {
                if let Some(c) = creeps.get_mut(*ce) {
                    c.block_tower = None;
                    next_outcomes.push(Outcome::CreepWalk { target: ce.clone() });
                }
            }
            "tower"
        } else if let Some(_p) = projs.get_mut(entity) {
            "projectile"
        } else {
            ""
        };

        if !entity_type.is_empty() {
            mqtx.send(OutboundMsg::new_s("td/all/res", entity_type, "D", json!({"id": entity.id()})));
        }

        if is_enemy_base {
            // 敵方基地被擊毀 → 玩家勝利
            log::info!("🏆 敵方基地 entity {:?} destroyed — emitting game.end", entity);
            mqtx.send(OutboundMsg::new_s(
                "td/all/res",
                "game",
                "end",
                json!({"winner": "player", "base_entity_id": entity.id()}),
            ));
        }
        Ok(())
    }
    
    /// 將 Bounty 分配給最近的友方英雄（MVP 以玩家陣營為友方）
    fn distribute_bounty(
        ecs: &mut World,
        next_outcomes: &mut Vec<Outcome>,
        mqtx: &crossbeam_channel::Sender<OutboundMsg>,
        dead: Entity,
    ) {
        use serde_json::json;
        let bounty = match ecs.read_storage::<Bounty>().get(dead).copied() {
            Some(b) => b,
            None => return,
        };
        let dead_pos = match ecs.read_storage::<Pos>().get(dead).map(|p| p.0) {
            Some(p) => p,
            None => return,
        };
        // 取出死者陣營（用於敵友判定，避免誤給自己人死亡獎勵）
        let dead_faction = ecs.read_storage::<Faction>().get(dead).cloned();

        // 找出最近、與死者敵對的 Player 英雄
        let (hero_e, _) = {
            let entities = ecs.entities();
            let heroes = ecs.read_storage::<Hero>();
            let factions = ecs.read_storage::<Faction>();
            let positions = ecs.read_storage::<Pos>();
            use specs::Join;
            let mut best: Option<(Entity, f32)> = None;
            for (e, _h, f, p) in (&entities, &heroes, &factions, &positions).join() {
                if f.faction_id != FactionType::Player {
                    continue;
                }
                if let Some(ref df) = dead_faction {
                    if !f.is_hostile_to(df) {
                        continue; // 同隊死亡不給賞金
                    }
                }
                let d2 = (p.0 - dead_pos).magnitude_squared();
                if d2 > 1200.0 * 1200.0 {
                    continue;
                }
                if best.map(|(_, d)| d2 < d).unwrap_or(true) {
                    best = Some((e, d2));
                }
            }
            match best {
                Some(x) => x,
                None => return,
            }
        };

        // 加金錢
        {
            let mut golds = ecs.write_storage::<Gold>();
            if let Some(g) = golds.get_mut(hero_e) {
                g.0 += bounty.gold;
            }
        }
        // 給經驗（透過 Hero::add_experience 處理升級與技能點）
        let leveled_up = {
            let mut heroes = ecs.write_storage::<Hero>();
            if let Some(h) = heroes.get_mut(hero_e) {
                let before = h.level;
                let _ = h.add_experience(bounty.exp);
                h.level != before
            } else {
                false
            }
        };

        // 廣播 hero.stats
        let lives = ecs.read_resource::<PlayerLives>().0;
        let hero_stats_payload = {
            let heroes = ecs.read_storage::<Hero>();
            let golds = ecs.read_storage::<Gold>();
            let props = ecs.read_storage::<CProperty>();
            let atks = ecs.read_storage::<TAttack>();
            let positions = ecs.read_storage::<Pos>();
            let buff_store = ecs.read_resource::<crate::ability_runtime::BuffStore>();
            let h = heroes.get(hero_e);
            let g = golds.get(hero_e).map(|g| g.0).unwrap_or(0);
            let prop = props.get(hero_e);
            let (hp, mhp) = prop.map(|p| (p.hp, p.mhp)).unwrap_or((0.0, 0.0));
            let (armor_b, mres_b, msd_b) = prop.map(|p| (p.def_physic, p.def_magic, p.msd)).unwrap_or((0.0, 0.0, 0.0));
            let (atk_dmg_b, atk_int_b, atk_rng_b, bullet_spd) = atks.get(hero_e)
                .map(|a| (a.atk_physic.v, a.asd.v, a.range.v, a.bullet_speed))
                .unwrap_or((0.0, 0.0, 0.0, 0.0));
            let p = positions.get(hero_e).map(|p| p.0).unwrap_or(vek::Vec2::zero());
            h.map(|h| {
                (
                    crate::state::resource_management::build_hero_stats_payload(
                        hero_e, h, g, hp, mhp, armor_b, mres_b, msd_b,
                        atk_dmg_b, atk_int_b, atk_rng_b, bullet_spd, lives, &buff_store,
                    ),
                    p,
                )
            })
        };
        if let Some((payload, pos)) = hero_stats_payload {
            let _ = mqtx.send(OutboundMsg::new_s_at(
                "td/all/res", "hero", "stats", payload, pos.x, pos.y,
            ));
        }
        if leveled_up {
            log::info!("🎉 hero entity {:?} 升級！", hero_e);
        }
    }

    fn handle_projectile(
        ecs: &mut World, 
        mqtx: &crossbeam_channel::Sender<OutboundMsg>, 
        pos: vek::Vec2<f32>, 
        source: Option<Entity>, 
        target: Option<Entity>
    ) -> Result<(), Error> {
        let source_entity = source.ok_or_else(|| failure::err_msg("Missing source entity"))?;
        let target_entity = target.ok_or_else(|| failure::err_msg("Missing target entity"))?;

        // 此 path 只用於非腳本塔（MOBA legacy）；TD 塔走腳本 `spawn_projectile_ex` 直接 spawn
        // 最終 damage 走 UnitStats::final_atk（聚合所有 stat_keys 官方 key）
        // 同時讀取 source 身上任何 buff 的 attack_stun_chance / attack_stun_duration，擲骰
        // 決定此發 projectile 命中後是否暈眩目標（matchlock_gun 的 87% 機率）
        // 另查 `multi_shot_visual` buff 決定是否額外 spawn 視覺子彈（無傷害）
        let (msd, p2, atk_phys, stun_duration_roll, visual_count) = {
            let positions = ecs.read_storage::<Pos>();
            let tproperty = ecs.read_storage::<TAttack>();
            let buff_store = ecs.read_resource::<crate::ability_runtime::BuffStore>();
            let is_buildings = ecs.read_storage::<IsBuilding>();

            let _p1 = positions.get(source_entity).ok_or_else(|| failure::err_msg("Source position not found"))?;
            let p2 = positions.get(target_entity).ok_or_else(|| failure::err_msg("Target position not found"))?;
            let tp = tproperty.get(source_entity).ok_or_else(|| failure::err_msg("Source attack properties not found"))?;
            let is_b = is_buildings.get(source_entity).is_some();
            let stats = crate::ability_runtime::UnitStats::from_refs(&*buff_store, is_b);
            let mut final_atk = stats.final_atk(tp.atk_physic.v, source_entity);

            // Accuracy 擲骰：base 命中率 1.0 + sum(accuracy_bonus) buffs；clamp [0,1]。
            // miss → damage=0（projectile 仍飛行，前端可由 0 傷害判定顯示 miss）。
            let accuracy = (1.0 + buff_store.sum_add(source_entity, omb_script_abi::stat_keys::StatKey::AccuracyBonus)).clamp(0.0, 1.0);
            if accuracy < 1.0 && fastrand::f32() > accuracy {
                final_atk = 0.0;
            }

            // 取 source 身上任一 buff 中最強的 attack_stun_chance + 對應 duration
            let mut stun_chance = 0.0f32;
            let mut stun_duration = 0.0f32;
            for (_, entry) in buff_store.iter_for(source_entity) {
                let c = entry.payload.get("attack_stun_chance").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
                let d = entry.payload.get("attack_stun_duration").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
                if c > stun_chance {
                    stun_chance = c;
                    stun_duration = d;
                }
            }
            let stun_roll = if stun_chance > 0.0 && stun_duration > 0.0 && fastrand::f32() < stun_chance {
                stun_duration
            } else {
                0.0
            };

            // 多發視覺 buff：sum_add 聚合（大絕套 3 → 3 發，也支援多個 buff 相加）
            // N > 1 時主彈正常判傷害，額外 N-1 發 visual-only（無傷害、target=None 到 tpos 自毀）
            let vc = buff_store.sum_add(source_entity, StatKey::MultiShotVisual);
            let visual_count = if vc >= 2.0 { vc.round().max(1.0) as u32 } else { 1 };

            (tp.bullet_speed, p2.0, final_atk, stun_roll, visual_count)
        };

        // 命中由 projectile_tick 的距離判定決定（target 接近時 step >= dist 即命中）。
        // time_left 為安全閥：flight_time_s * 3 + 3 秒，允許高速單位拖著子彈移動。
        let move_speed = msd as f32;
        let initial_dist = (p2 - pos).magnitude();
        let flight_time_s: f32 = if move_speed > 0.0 {
            (initial_dist / move_speed).max(0.01)
        } else {
            0.01
        };
        let safety_time_left = flight_time_s * 3.0 + 3.0;

        // Legacy path (MOBA 英雄 / 非腳本塔)：單體傷害、無 splash、無 slow
        let (splash_radius, slow_factor, slow_duration): (f32, f32, f32) = (0.0, 0.0, 0.0);

        let ntarget = target_entity.id();
        let flight_time_ms: u64 = (flight_time_s * 1000.0).max(1.0) as u64;
        let kind_key = "";

        // 主彈 + 視覺彈（大絕變身 buff 讓 visual_count=3）：
        // i == 0 為真實子彈（damage = atk_phys、target 追蹤、吃 stun roll）
        // i >= 1 為視覺子彈（damage = 0、target = None、到 tpos 自毀不 hit、起點左右側偏）
        let dir = if (p2 - pos).magnitude_squared() > 0.0001 {
            (p2 - pos).normalized()
        } else {
            vek::Vec2::new(1.0_f32, 0.0)
        };
        let perp = vek::Vec2::new(-dir.y, dir.x);
        let lateral_step = 24.0_f32; // 槍口偏移 (pixel)

        for i in 0..visual_count {
            let is_real = i == 0;
            let dmg_phys_this = if is_real { atk_phys } else { 0.0 };
            let stun_this = if is_real { stun_duration_roll } else { 0.0 };
            let target_this = if is_real { target } else { None };
            let lateral = if visual_count > 1 {
                let half = (visual_count as f32 - 1.0) * 0.5;
                (i as f32 - half) * lateral_step
            } else {
                0.0
            };
            let start_pos = pos + perp * lateral;

            let e = ecs.create_entity()
                .with(Pos(start_pos))
                .with(Projectile {
                    time_left: safety_time_left,
                    owner: source_entity.clone(),
                    tpos: p2,
                    target: target_this,
                    radius: splash_radius,
                    msd: msd,
                    damage_phys: dmg_phys_this,
                    damage_magi: 0.0,
                    damage_real: 0.0,
                    slow_factor,
                    slow_duration,
                    hit_radius: 0.0,
                    stun_duration: stun_this,
                })
                .build();

            let pjs = json!({
                "id": e.id(),
                "target_id": ntarget,
                "start_pos": { "x": start_pos.x, "y": start_pos.y },
                "end_pos":   { "x": p2.x, "y": p2.y },
                "move_speed": move_speed,
                "flight_time_ms": flight_time_ms,
                "kind": kind_key,
            });
            mqtx.try_send(OutboundMsg::new_s_at("td/all/res", "projectile", "C", pjs, start_pos.x, start_pos.y));
        }
        Ok(())
    }
    
    fn handle_creep_spawn(ecs: &mut World, mqtx: &crossbeam_channel::Sender<OutboundMsg>, cd: CreepData) -> Result<(), Error> {
        let display_name = cd.creep.label.clone().unwrap_or_else(|| cd.creep.name.clone());
        let creep_name = cd.creep.name.clone();
        let hp = cd.cdata.hp;
        let mhp = cd.cdata.mhp;
        let msd = cd.cdata.msd;
        let pos = cd.pos;
        // 依 creep 名稱決定獎勵（MVP 簡版）
        let bounty = match creep_name.as_str() {
            "melee_minion" => Bounty { gold: 18, exp: 55 },
            "ranged_minion" => Bounty { gold: 15, exp: 45 },
            "siege_minion" => Bounty { gold: 40, exp: 110 },
            // 我方 creep 死亡不給 bounty
            n if n.starts_with("ally_") => Bounty { gold: 0, exp: 0 },
            _ => Bounty { gold: 10, exp: 25 },
        };
        // 陣營：依 CreepData.faction_name 決定，空字串 → Enemy（舊 map 相容）
        let faction = match cd.faction_name.as_str() {
            "Player" | "player" => Faction::new(FactionType::Player, 0),
            _ => Faction::new(FactionType::Enemy, 1),
        };
        let turn_speed_rad = cd.turn_speed_deg.to_radians();
        // Creep 統一掛 ScriptUnitTag（預設全單位腳本化）；unit_id = "creep_{name}"
        let unit_id = format!("creep_{}", creep_name);
        let e = ecs.create_entity()
            .with(Pos(cd.pos))
            .with(cd.creep)
            .with(cd.cdata)
            .with(faction)
            .with(bounty)
            .with(Facing(0.0))
            .with(TurnSpeed(turn_speed_rad))
            .with(crate::scripting::ScriptUnitTag { unit_id: unit_id.clone() })
            .build();
        ecs.write_resource::<crate::scripting::ScriptEventQueue>()
            .push(crate::scripting::ScriptEvent::Spawn { e });
        // Payload shape matches client expectations (top-level position/hp/max_hp)
        let payload = json!({
            "entity_id": e.id(),
            "id": e.id(),
            "name": display_name,
            "position": { "x": pos.x, "y": pos.y },
            "hp": hp,
            "max_hp": mhp,
            "move_speed": msd,
        });
        mqtx.try_send(OutboundMsg::new_s_at("td/all/res", "creep", "C", payload, pos.x, pos.y));
        Ok(())
    }

    /// 通用 buff 寫入：把 `Outcome::AddBuff` 對應到 `BuffStore`。若 payload 含
    /// `move_speed_bonus` 且 target 是 Creep，立即廣播 `creep/S` 讓 client 端
    /// lerp 移速變慢（replaces 舊的 handle_apply_slow 專用廣播）。
    fn handle_add_buff(
        ecs: &mut World,
        target: Entity,
        buff_id: String,
        duration: f32,
        payload: serde_json::Value,
    ) -> Result<(), Error> {
        let has_move_speed_bonus = payload.get(StatKey::MoveSpeedBonus.as_str()).and_then(|v| v.as_f64()).is_some();
        {
            let mut store = ecs.write_resource::<crate::ability_runtime::BuffStore>();
            store.add(target, &buff_id, duration, payload);
        }
        // 只針對有移速影響、且是 creep 的目標廣播（hero 走 hero_move_tick 每幀發位置，不需要離散更新）
        if has_move_speed_bonus {
            let is_creep = ecs.read_storage::<Creep>().get(target).is_some();
            if is_creep {
                let msd = ecs.read_storage::<CProperty>()
                    .get(target).map(|c| c.msd).unwrap_or(0.0);
                let sum = {
                    let store = ecs.read_resource::<crate::ability_runtime::BuffStore>();
                    store.sum_add(target, StatKey::MoveSpeedBonus)
                };
                let effective = msd * (1.0 + sum).clamp(0.01, 1.0);
                if let Some(tx) = ecs.read_resource::<Vec<crossbeam_channel::Sender<OutboundMsg>>>().get(0) {
                    let _ = tx.try_send(OutboundMsg::new_s("td/all/res", "creep", "S", json!({
                        "id": target.id(),
                        "move_speed": effective,
                    })));
                }
            }
        }
        Ok(())
    }

    /// TD 模式：小兵漏怪到終點。扣 `PlayerLives` 1、廣播 `game/lives` 與 entity delete。
    /// 若生命歸零再廣播 `game/end`（遊戲結束）。
    fn handle_creep_leaked(
        ecs: &mut World,
        mqtx: &crossbeam_channel::Sender<OutboundMsg>,
        entity: Entity,
    ) -> Result<(), Error> {
        let remaining = {
            let mut lives = ecs.write_resource::<PlayerLives>();
            lives.0 = (lives.0 - 1).max(0);
            lives.0
        };
        log::info!("💔 小兵漏網！玩家生命 {} (entity={:?})", remaining, entity);

        // 廣播 entity delete 讓前端移除 creep 圖
        let _ = mqtx.try_send(OutboundMsg::new_s(
            "td/all/res",
            "creep",
            "D",
            json!({ "id": entity.id() }),
        ));

        // 廣播專用的 game/lives 事件（前端 HUD 立即更新），不需要 hero.stats
        let _ = mqtx.try_send(OutboundMsg::new_s(
            "td/all/res",
            "game",
            "lives",
            json!({ "lives": remaining }),
        ));

        if remaining <= 0 {
            let _ = mqtx.try_send(OutboundMsg::new_s(
                "td/all/res",
                "game",
                "end",
                json!({ "result": "defeat", "reason": "lives_depleted" }),
            ));
            log::warn!("☠️ TD 模式：玩家生命歸零，遊戲結束");
        }
        Ok(())
    }

    /// Tack 塔放射針：沒有 target，飛向固定 end_pos；命中判定在 projectile_tick 逐 tick 掃描。
    fn handle_projectile_directional(
        ecs: &mut World,
        mqtx: &crossbeam_channel::Sender<OutboundMsg>,
        pos: vek::Vec2<f32>,
        source: Option<Entity>,
        end_pos: vek::Vec2<f32>,
    ) -> Result<(), Error> {
        use specs::{Builder, WorldExt};

        let source_entity = source.ok_or_else(|| failure::err_msg("ProjectileDirectional 缺少 source"))?;

        // 此 path 為 legacy（tower_tick 不再 push ProjectileDirectional；Tack 走腳本
        // spawn_projectile_ex）。保留 handle 作為備用；kind_key 留空字串
        let (msd, atk_phys, kind_key): (f32, f32, &str) = {
            let tatks = ecs.read_storage::<TAttack>();
            let tp = tatks.get(source_entity).ok_or_else(|| failure::err_msg("Source attack properties not found"))?;
            (tp.bullet_speed, tp.atk_physic.v, "")
        };

        let initial_dist = (end_pos - pos).magnitude();
        let flight_time_s: f32 = if msd > 0.0 { (initial_dist / msd).max(0.01) } else { 0.01 };
        let safety_time_left = flight_time_s * 1.5 + 0.5;

        let e = ecs.create_entity()
            .with(Pos(pos))
            .with(Projectile {
                time_left: safety_time_left,
                owner: source_entity,
                tpos: end_pos,
                target: None,
                radius: 0.0,
                msd,
                damage_phys: atk_phys,
                damage_magi: 0.0,
                damage_real: 0.0,
                slow_factor: 0.0,
                slow_duration: 0.0,
                hit_radius: 0.0,
                stun_duration: 0.0,
            })
            .build();

        // 前端渲染用：沒 target_id 時用 end_pos 做 end 位置
        let flight_time_ms: u64 = (flight_time_s * 1000.0).max(1.0) as u64;
        let pjs = json!({
            "id": e.id(),
            "target_id": 0, // 0 = 無 target（directional）
            "start_pos": { "x": pos.x, "y": pos.y },
            "end_pos":   { "x": end_pos.x, "y": end_pos.y },
            "move_speed": msd,
            "flight_time_ms": flight_time_ms,
            "kind": kind_key,
            "directional": true,
            "hit_radius": 80.0_f32, // legacy directional path 用預設
        });
        mqtx.try_send(OutboundMsg::new_s_at("td/all/res", "projectile", "C", pjs, pos.x, pos.y));
        Ok(())
    }

    fn handle_tower_spawn(ecs: &mut World, mqtx: &crossbeam_channel::Sender<OutboundMsg>, pos: vek::Vec2<f32>, td: TowerData) -> Result<(), Error> {
        let mut cjs = json!(td);
        let e = ecs.create_entity().with(Pos(pos)).with(Tower::new()).with(td.tpty).with(td.tatk).build();
        cjs.as_object_mut().unwrap().insert("id".to_owned(), json!(e.id()));
        cjs.as_object_mut().unwrap().insert("pos".to_owned(), json!(pos));
        mqtx.try_send(OutboundMsg::new_s_at("td/all/res", "tower", "C", cjs, pos.x, pos.y));
        ecs.get_mut::<Searcher>().unwrap().tower.needsort = true;
        Ok(())
    }
    
    fn handle_creep_stop(ecs: &mut World, mqtx: &crossbeam_channel::Sender<OutboundMsg>, source: Entity, target: Entity) -> Result<(), Error> {
        let mut creeps = ecs.write_storage::<Creep>();
        let c = creeps.get_mut(target).ok_or_else(|| failure::err_msg("Creep not found"))?;
        c.block_tower = Some(source);
        c.status = CreepStatus::Stop;
        
        let positions = ecs.read_storage::<Pos>();
        let pos = positions.get(target).ok_or_else(|| failure::err_msg("Creep position not found"))?;
        
        mqtx.try_send(OutboundMsg::new_s_at("td/all/res", "creep", "M", json!({
            "id": target.id(),
            "x": pos.0.x,
            "y": pos.0.y,
        }), pos.0.x, pos.0.y));
        Ok(())
    }
    
    fn handle_creep_walk(ecs: &mut World, target: Entity) -> Result<(), Error> {
        let mut creeps = ecs.write_storage::<Creep>();
        let creep = creeps.get_mut(target).ok_or_else(|| failure::err_msg("Creep not found"))?;
        creep.status = CreepStatus::PreWalk;
        Ok(())
    }
    
    fn handle_damage(
        ecs: &mut World,
        next_outcomes: &mut Vec<Outcome>,
        pos: vek::Vec2<f32>,
        phys: f32,
        magi: f32,
        real: f32,
        source: Entity,
        target: Entity
    ) -> Result<(), Error> {
        let mut hp_after = 0.0f32;
        let mut max_hp = 0.0f32;
        let mut died = false;

        // damage_taken_bonus 聚合（Task 14）：目標身上所有 buff 的此 key sum_add
        // 例：Ice Embrittlement L3 對被減速 creep +25% 傷害
        let dmg_taken_bonus = {
            let bs = ecs.read_resource::<crate::ability_runtime::BuffStore>();
            bs.sum_add(target, StatKey::DamageTakenBonus)
        };
        let dmg_multiplier = (1.0 + dmg_taken_bonus).max(0.0);

        {
            let mut properties = ecs.write_storage::<CProperty>();
            if let Some(target_props) = properties.get_mut(target) {
                let hp_before = target_props.hp;
                let total_damage = (phys + magi + real) * dmg_multiplier;
                target_props.hp -= total_damage;
                hp_after = target_props.hp;
                max_hp = target_props.mhp;

                let (source_name, target_name) = Self::get_entity_names(ecs, source, target);

                let damage_parts = {
                    let mut parts = Vec::new();
                    if phys > 0.0 { parts.push(format!("Phys {:.1}", phys)); }
                    if magi > 0.0 { parts.push(format!("Magi {:.1}", magi)); }
                    if real > 0.0 { parts.push(format!("Pure {:.1}", real)); }
                    if parts.is_empty() {
                        parts.push(format!("Total {:.1}", total_damage));
                    }
                    parts.join(", ")
                };

                log::debug!("⚔️ {} 攻擊 {} | {} damage | HP: {:.1} → {:.1}/{:.1}",
                    source_name, target_name, damage_parts, hp_before, hp_after, target_props.mhp
                );

                if target_props.hp <= 0.0 {
                    target_props.hp = 0.0;
                    hp_after = 0.0;
                    died = true;
                    log::debug!("💀 {} died from damage!", target_name);
                }
            }
        }

        // Broadcast HP update to frontend
        let target_pos = ecs.read_storage::<Pos>().get(target).map(|p| p.0);
        if let Some(tp) = target_pos {
            // Determine entity type for the broadcast
            let entity_type = {
                let heroes = ecs.read_storage::<Hero>();
                let creeps = ecs.read_storage::<Creep>();
                let units = ecs.read_storage::<Unit>();
                if heroes.get(target).is_some() { "hero" }
                else if creeps.get(target).is_some() { "creep" }
                else if units.get(target).is_some() { "unit" }
                else { "entity" }
            };
            let mqtx_list = ecs.read_resource::<Vec<crossbeam_channel::Sender<OutboundMsg>>>();
            if let Some(tx) = mqtx_list.get(0) {
                let total = phys + magi + real;
                if total <= 0.0 {
                    // Miss 廣播：accuracy 擲骰失敗導致 0 傷害 → 前端顯示 "Miss"
                    let _ = tx.send(OutboundMsg::new_s_at(
                        "td/all/res",
                        entity_type,
                        "Miss",
                        json!({ "id": target.id() }),
                        tp.x, tp.y,
                    ));
                } else {
                    // HP-only update. Action "H" keeps this separate from real move events;
                    // the position in `new_s_at` is only used for viewport filtering (not the payload).
                    let _ = tx.send(OutboundMsg::new_s_at("td/all/res", entity_type, "H", json!({
                        "id": target.id(),
                        "hp": hp_after,
                        "max_hp": max_hp,
                    }), tp.x, tp.y));
                }
            }
        }

        if died {
            next_outcomes.push(Outcome::Death {
                pos: pos,
                ent: target
            });
        }

        Ok(())
    }
    
    fn handle_heal(ecs: &mut World, target: Entity, amount: f32) -> Result<(), Error> {
        let mut properties = ecs.write_storage::<CProperty>();
        if let Some(target_props) = properties.get_mut(target) {
            target_props.hp = (target_props.hp + amount).min(target_props.mhp);
        }
        Ok(())
    }
    
    fn handle_attack_update(ecs: &mut World, target: Entity, asd_count: Option<f32>, cooldown_reset: bool) -> Result<(), Error> {
        let mut attacks = ecs.write_storage::<TAttack>();
        if let Some(attack) = attacks.get_mut(target) {
            if let Some(new_count) = asd_count {
                attack.asd_count = new_count;
            }
            if cooldown_reset {
                attack.asd_count = attack.asd.v;
            }
        }
        Ok(())
    }
    
    fn handle_experience_gain(ecs: &mut World, target: Entity, amount: u32) -> Result<(), Error> {
        let mut heroes = ecs.write_storage::<Hero>();
        if let Some(hero) = heroes.get_mut(target) {
            let leveled_up = hero.add_experience(amount as i32);
            if leveled_up {
                log::info!("Hero '{}' gained {} experience and leveled up!", hero.name, amount);
            } else {
                log::info!("Hero '{}' gained {} experience", hero.name, amount);
            }
        }
        Ok(())
    }
    
    fn get_entity_names(ecs: &World, source: Entity, target: Entity) -> (String, String) {
        let creeps = ecs.read_storage::<Creep>();
        let heroes = ecs.read_storage::<Hero>();
        let units = ecs.read_storage::<Unit>();
        let towers = ecs.read_storage::<Tower>();
        let script_tags = ecs.read_storage::<crate::scripting::ScriptUnitTag>();
        let registry = ecs.read_resource::<crate::comp::tower_registry::TowerTemplateRegistry>();

        // 優先 creep / hero / unit；其次依 ScriptUnitTag 查 TowerTemplateRegistry label（TD 塔）；
        // 再其次泛用 Tower；都沒有就 Unknown。
        let name_of = |ent: Entity| -> String {
            if let Some(creep) = creeps.get(ent) {
                return creep.name.clone();
            }
            if let Some(hero) = heroes.get(ent) {
                return hero.name.clone();
            }
            if let Some(unit) = units.get(ent) {
                return unit.name.clone();
            }
            if let Some(tag) = script_tags.get(ent) {
                if let Some(tpl) = registry.get(&tag.unit_id) {
                    return tpl.label.clone();
                }
            }
            if towers.get(ent).is_some() {
                return "Tower".to_string();
            }
            "Unknown".to_string()
        };

        (name_of(source), name_of(target))
    }
}