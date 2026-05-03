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

/// Per-entity SimRng op_kind for game_processor. Phase 1de.2: replaces fastrand
/// for projectile accuracy + attack-stun rolls. Reordering or reusing these
/// constants across systems would invalidate replay determinism.
const OP_PROJECTILE_ACCURACY: u32 = 20;
const OP_PROJECTILE_STUN_ROLL: u32 = 21;

// ============================================================================
// P2 typed-payload helpers — gated behind `kcp`. For non-kcp builds the helpers
// fall back to legacy JSON-only OutboundMsg construction.
// ============================================================================

/// game.explosion
#[inline]
fn make_game_explosion(x: f32, y: f32, radius: f32, duration: f32) -> OutboundMsg {
    #[cfg(feature = "kcp")]
    {
        use crate::state::resource_management::proto_build;
        use crate::transport::TypedOutbound;
        OutboundMsg::new_typed_at(
            "td/all/res", "game", "explosion",
            TypedOutbound::GameExplosion(proto_build::game_explosion(x, y, radius, duration)),
            json!({ "x": x, "y": y, "radius": radius, "duration": duration }),
            x, y,
        )
    }
    #[cfg(not(feature = "kcp"))]
    {
        OutboundMsg::new_s_at("td/all/res", "game", "explosion",
            json!({ "x": x, "y": y, "radius": radius, "duration": duration }), x, y)
    }
}

/// game.lives
#[inline]
fn make_game_lives(lives: i32) -> OutboundMsg {
    #[cfg(feature = "kcp")]
    {
        use crate::state::resource_management::proto_build;
        use crate::transport::TypedOutbound;
        // P5: game-wide event — reach every player.
        OutboundMsg::new_typed_all(
            "td/all/res", "game", "lives",
            TypedOutbound::GameLives(proto_build::game_lives(lives)),
            json!({ "lives": lives }),
        )
    }
    #[cfg(not(feature = "kcp"))]
    {
        OutboundMsg::new_s("td/all/res", "game", "lives", json!({ "lives": lives }))
    }
}

/// game.end
#[inline]
fn make_game_end(winner: &str, extra: serde_json::Value) -> OutboundMsg {
    #[cfg(feature = "kcp")]
    {
        use crate::state::resource_management::proto_build;
        use crate::transport::TypedOutbound;
        // P5: game-end broadcasts to every player.
        OutboundMsg::new_typed_all(
            "td/all/res", "game", "end",
            TypedOutbound::GameEnd(proto_build::game_end(winner)),
            extra,
        )
    }
    #[cfg(not(feature = "kcp"))]
    {
        OutboundMsg::new_s("td/all/res", "game", "end", extra)
    }
}

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
        Outcome::GainGold { .. } => "GainGold",
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
                        phys: p, magi: m, real: r, target: t, predeclared: pd, ..
                    } = &out
                    {
                        if let Some(&idx) = first_dmg_idx.get(t) {
                            if let Outcome::Damage {
                                phys: ap, magi: am, real: ar, predeclared: apd, ..
                            } = &mut aggregated[idx]
                            {
                                *ap += *p;
                                *am += *m;
                                *ar += *r;
                                // P7: AND-combine — skip H only if ALL
                                // contributors were pre-declared. Mixed
                                // (predeclared + authoritative) falls back to
                                // emitting H so server stays authoritative.
                                *apd = *apd && *pd;
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
                    Outcome::Damage { pos, phys, magi, real, source, target, predeclared } => {
                        Self::handle_damage(ecs, &mut next_outcomes, pos, phys, magi, real, source, target, predeclared)?;
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
                    Outcome::GainGold { target, amount } => {
                        Self::handle_gold_gain(ecs, target, amount)?;
                    }
                    Outcome::CreepLeaked { ent } => {
                        remove_uids.push(ent);
                        Self::handle_creep_leaked(ecs, mqtx, ent)?;
                    }
                    Outcome::AddBuff { target, buff_id, duration, payload } => {
                        Self::handle_add_buff(ecs, target, buff_id, duration, payload)?;
                    }
                    Outcome::Explosion { pos, radius, duration } => {
                        let _ = mqtx.try_send(make_game_explosion(
                            pos.x.to_f32_for_render(),
                            pos.y.to_f32_for_render(),
                            radius.to_f32_for_render(),
                            duration.to_f32_for_render(),
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

        // Cleanup creep/tower cross-links so the dead entity doesn't leave dangling
        // block_tower / block_creeps references in survivors. Side-effects only —
        // the entity itself is removed by `delete_entities` in process_outcomes.
        // Phase 1.6: 不再廣播 entity.death；omfx sim_runner worker 用
        // SimWorldSnapshot.removed_entity_ids 自行偵測死亡釋放渲染資源。
        {
            let mut creeps = ecs.write_storage::<Creep>();
            let mut towers = ecs.write_storage::<Tower>();
            if let Some(c) = creeps.get_mut(entity) {
                if let Some(bt) = c.block_tower {
                    if let Some(t) = towers.get_mut(bt) {
                        t.block_creeps.retain(|&x| x != entity);
                    }
                }
            } else if let Some(t) = towers.get_mut(entity) {
                let blocked: Vec<Entity> = t.block_creeps.clone();
                for ce in blocked {
                    if let Some(c) = creeps.get_mut(ce) {
                        c.block_tower = None;
                        next_outcomes.push(Outcome::CreepWalk { target: ce });
                    }
                }
            }
        }

        if is_enemy_base {
            // 敵方基地被擊毀 → 玩家勝利
            log::info!("🏆 敵方基地 entity {:?} destroyed — emitting game.end", entity);
            let _ = mqtx.send(make_game_end("player",
                json!({"winner": "player", "base_entity_id": entity.id()})));
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
                // NOTE: bounty proximity is non-deterministic UI hint (faction-scoped); lossy f32 acceptable.
                let (px, py) = p.xy_f32();
                let dpx = dead_pos.x.to_f32_for_render();
                let dpy = dead_pos.y.to_f32_for_render();
                let dx = px - dpx;
                let dy = py - dpy;
                let d2 = dx * dx + dy * dy;
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

        if leveled_up {
            log::info!("🎉 hero entity {:?} 升級！", hero_e);
        }
    }

    fn handle_projectile(
        ecs: &mut World,
        mqtx: &crossbeam_channel::Sender<OutboundMsg>,
        pos: omoba_sim::Vec2,
        source: Option<Entity>,
        target: Option<Entity>
    ) -> Result<(), Error> {
        use omoba_sim::{Fixed64, Vec2 as SimVec2};
        let source_entity = source.ok_or_else(|| failure::err_msg("Missing source entity"))?;
        let target_entity = target.ok_or_else(|| failure::err_msg("Missing target entity"))?;

        // 此 path 只用於非腳本塔（MOBA legacy）；TD 塔走腳本 `spawn_projectile_ex` 直接 spawn
        // 最終 damage 走 UnitStats::final_atk（聚合所有 stat_keys 官方 key）
        // 同時讀取 source 身上任何 buff 的 attack_stun_chance / attack_stun_duration，擲骰
        // 決定此發 projectile 命中後是否暈眩目標（matchlock_gun 的 87% 機率）
        // 另查 `multi_shot_visual` buff 決定是否額外 spawn 視覺子彈（無傷害）
        // Phase 1de.2: deterministic SimRng inputs (master_seed + tick) for the
        // accuracy / stun rolls. Read once at the top of handle_projectile to
        // avoid repeated resource lookups inside the rolls.
        let master_seed: u64 = ecs.read_resource::<MasterSeed>().0;
        let tick: u32 = ecs.read_resource::<Tick>().0 as u32;
        let attacker_id: u32 = source_entity.id();

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
            let mut final_atk: Fixed64 = stats.final_atk(tp.atk_physic.v, source_entity);

            // Accuracy 擲骰：base 命中率 1.0 + sum(accuracy_bonus) buffs；clamp [0,1]。
            // miss → damage=0（projectile 仍飛行，前端可由 0 傷害判定顯示 miss）。
            // Phase 1de.2: deterministic per-(attacker, OP_PROJECTILE_ACCURACY) stream.
            let accuracy_bonus = buff_store
                .sum_add(source_entity, omb_script_abi::stat_keys::StatKey::AccuracyBonus);
            let accuracy: Fixed64 = (Fixed64::ONE + accuracy_bonus).clamp(Fixed64::ZERO, Fixed64::ONE);
            if accuracy < Fixed64::ONE {
                let mut acc_rng = omoba_sim::SimRng::from_master_entity(
                    master_seed, tick, attacker_id, OP_PROJECTILE_ACCURACY,
                );
                let roll: Fixed64 = acc_rng.gen_fixed64_unit();
                // Original semantics: miss iff roll > accuracy. With Fixed64 uniform
                // on the [0,1) grid, `roll >= accuracy` preserves the miss probability
                // (roll == accuracy collides at one out of 1024 buckets — within game tolerance).
                if roll >= accuracy {
                    final_atk = Fixed64::ZERO;
                }
            }

            // 取 source 身上任一 buff 中最強的 attack_stun_chance + 對應 duration
            // Phase 1de.2: still reads f64 from JSON payload (BuffStore wire format
            // accepts both i64 raw and legacy f64 — see buff_store.rs).
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
            // Phase 1de.2: deterministic per-(attacker, OP_PROJECTILE_STUN_ROLL) stream.
            let stun_roll: Fixed64 = if stun_chance > 0.0 && stun_duration > 0.0 {
                let mut stun_rng = omoba_sim::SimRng::from_master_entity(
                    master_seed, tick, attacker_id, OP_PROJECTILE_STUN_ROLL,
                );
                let roll = stun_rng.gen_fixed64_unit().to_f32_for_render();
                if roll < stun_chance {
                    Fixed64::from_raw((stun_duration * omoba_sim::fixed::SCALE as f32) as i64)
                } else {
                    Fixed64::ZERO
                }
            } else {
                Fixed64::ZERO
            };

            // 多發視覺 buff：sum_add 聚合（大絕套 3 → 3 發，也支援多個 buff 相加）
            // N > 1 時主彈正常判傷害，額外 N-1 發 visual-only（無傷害、target=None 到 tpos 自毀）
            let vc = buff_store
                .sum_add(source_entity, StatKey::MultiShotVisual)
                .to_f32_for_render();
            let visual_count = if vc >= 2.0 { vc.round().max(1.0) as u32 } else { 1 };

            (tp.bullet_speed, p2.0, final_atk, stun_roll, visual_count)
        };

        // 命中由 projectile_tick 的距離判定決定（target 接近時 step >= dist 即命中）。
        // time_left 為安全閥：flight_time_s * 3 + 3 秒，允許高速單位拖著子彈移動。
        let initial_dist: Fixed64 = (p2 - pos).length();
        // flight_time math (s) needs f32 — wire format is f32 ms, and we want
        // the .max(0.01) clamp behavior. Compute in f32 only at the wire boundary.
        let move_speed_f = msd.to_f32_for_render();
        let initial_dist_f = initial_dist.to_f32_for_render();
        let flight_time_s: f32 = if move_speed_f > 0.0 {
            (initial_dist_f / move_speed_f).max(0.01)
        } else {
            0.01
        };
        let safety_time_left: Fixed64 = Fixed64::from_raw(((flight_time_s * 3.0 + 3.0) * omoba_sim::fixed::SCALE as f32) as i64);

        // Legacy path (MOBA 英雄 / 非腳本塔)：單體傷害、無 splash、無 slow
        let splash_radius: Fixed64 = Fixed64::ZERO;
        let slow_factor: Fixed64 = Fixed64::ZERO;
        let slow_duration: Fixed64 = Fixed64::ZERO;

        let ntarget = target_entity.id();
        let flight_time_ms: u64 = (flight_time_s * 1000.0).max(1.0) as u64;
        let kind_id: u16 = 0;  // UNSPECIFIED — legacy handler, no template assignment

        // 主彈 + 視覺彈（大絕變身 buff 讓 visual_count=3）：
        // i == 0 為真實子彈（damage = atk_phys、target 追蹤、吃 stun roll）
        // i >= 1 為視覺子彈（damage = 0、target = None、到 tpos 自毀不 hit、起點左右側偏）
        let delta = p2 - pos;
        let dir: SimVec2 = if delta.length_squared() > Fixed64::from_raw(1) {
            delta.normalized()
        } else {
            SimVec2::new(Fixed64::ONE, Fixed64::ZERO)
        };
        let perp: SimVec2 = SimVec2::new(-dir.y, dir.x);
        let lateral_step: Fixed64 = Fixed64::from_i32(24); // 槍口偏移 (pixel)

        for i in 0..visual_count {
            let is_real = i == 0;
            let dmg_phys_this: Fixed64 = if is_real { atk_phys } else { Fixed64::ZERO };
            let stun_this: Fixed64 = if is_real { stun_duration_roll } else { Fixed64::ZERO };
            let target_this = if is_real { target } else { None };
            // (i - half) * lateral_step ; computed in Fixed64: half can be 0.5
            // → encode as raw 512.
            let lateral: Fixed64 = if visual_count > 1 {
                let half_raw: i64 = ((visual_count as i64 - 1) * 512) ; // (n-1)/2 * SCALE
                let i_scaled: i64 = i as i64 * omoba_sim::fixed::SCALE; // i * SCALE
                let diff_raw: i64 = i_scaled - half_raw;
                Fixed64::from_raw(diff_raw) * lateral_step
            } else {
                Fixed64::ZERO
            };
            let start_pos: SimVec2 = pos + perp * lateral;
            let start_x_f = start_pos.x.to_f32_for_render();
            let start_y_f = start_pos.y.to_f32_for_render();
            let p2_x_f = p2.x.to_f32_for_render();
            let p2_y_f = p2.y.to_f32_for_render();

            let e = ecs.create_entity()
                .with(Pos(start_pos))
                .with(Projectile {
                    time_left: safety_time_left,
                    owner: source_entity.clone(),
                    tpos: p2,
                    target: target_this,
                    radius: splash_radius,
                    msd,
                    damage_phys: dmg_phys_this,
                    damage_magi: Fixed64::ZERO,
                    damage_real: Fixed64::ZERO,
                    slow_factor,
                    slow_duration,
                    hit_radius: Fixed64::ZERO,
                    stun_duration: stun_this,
                })
                .build();

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
        let turn_speed_rad_f = cd.turn_speed_deg.to_f32_for_render().to_radians();
        // Creep 統一掛 ScriptUnitTag（預設全單位腳本化）；unit_id = "creep_{name}"
        let unit_id = format!("creep_{}", creep_name);
        let e = ecs.create_entity()
            .with(Pos(pos)) // SimVec2 直接內嵌
            .with(cd.creep)
            .with(cd.cdata)
            .with(faction)
            .with(bounty)
            .with(Facing(omoba_sim::Angle::ZERO))
            .with(FacingBroadcast(None))
            .with(TurnSpeed(omoba_sim::Fixed64::from_raw((turn_speed_rad_f * 1024.0) as i64)))
            .with(crate::scripting::ScriptUnitTag { unit_id: unit_id.clone() })
            .build();
        ecs.write_resource::<crate::scripting::ScriptEventQueue>()
            .push(crate::scripting::ScriptEvent::Spawn { e });
        // Default min-move-speed buff：避免多重 ice 減速把 ms 壓到 ≤ 1 觸發 frontend
        // lerp/extrap fallback 導致 creep 瞬移到下個 waypoint。
        // 用 BuffStore 寫入而非全域 clamp — 不同 creep 類型未來可以有不同下限、
        // 設計上也允許某些 buff 顯式拿掉這個下限（例如「凍結 1 秒」效果）。
        // Phase 1c.3: BuffStore::add now takes Fixed64 — use raw i32::MAX as
        // sentinel "permanent". NOTE: i32::MAX duration is the permanent-buff convention; could be replaced with explicit None/permanent flag in Phase 2.
        ecs.write_resource::<crate::ability_runtime::BuffStore>().add(
            e,
            "creep_min_speed_floor",
            omoba_sim::Fixed64::from_raw(i64::MAX),
            serde_json::json!({ "movespeed_absolute_min": 10.0 }),
        );
        Ok(())
    }

    /// 通用 buff 寫入：把 `Outcome::AddBuff` 對應到 `BuffStore`。若 payload 含
    /// `move_speed_bonus` 且 target 是 Creep，立即廣播 `creep/S` 讓 client 端
    /// lerp 移速變慢（replaces 舊的 handle_apply_slow 專用廣播）。
    fn handle_add_buff(
        ecs: &mut World,
        target: Entity,
        buff_id: String,
        duration: omoba_sim::Fixed64,
        payload: serde_json::Value,
    ) -> Result<(), Error> {
        {
            let mut store = ecs.write_resource::<crate::ability_runtime::BuffStore>();
            store.add(target, &buff_id, duration, payload);
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

        // Phase 1.6: 不再送 entity.death；omfx 用 SimWorldSnapshot.removed_entity_ids
        // 偵測 creep 從 ECS 消失自動釋放 sprite / label slot。

        // 廣播專用的 game/lives 事件（前端 HUD 立即更新），不需要 hero.stats
        let _ = mqtx.try_send(make_game_lives(remaining));

        if remaining <= 0 {
            let _ = mqtx.try_send(make_game_end("defeat",
                json!({ "result": "defeat", "reason": "lives_depleted" })));
            log::warn!("☠️ TD 模式：玩家生命歸零，遊戲結束");
        }
        Ok(())
    }

    /// Tack 塔放射針：沒有 target，飛向固定 end_pos；命中判定在 projectile_tick 逐 tick 掃描。
    fn handle_projectile_directional(
        ecs: &mut World,
        mqtx: &crossbeam_channel::Sender<OutboundMsg>,
        pos: omoba_sim::Vec2,
        source: Option<Entity>,
        end_pos: omoba_sim::Vec2,
    ) -> Result<(), Error> {
        use specs::{Builder, WorldExt};
        use omoba_sim::Fixed64;

        let source_entity = source.ok_or_else(|| failure::err_msg("ProjectileDirectional 缺少 source"))?;

        // 此 path 為 legacy（tower_tick 不再 push ProjectileDirectional；Tack 走腳本
        // spawn_projectile_ex）。保留 handle 作為備用；kind_id 留 0 (UNSPECIFIED)
        let (msd, atk_phys, kind_id): (Fixed64, Fixed64, u16) = {
            let tatks = ecs.read_storage::<TAttack>();
            let tp = tatks.get(source_entity).ok_or_else(|| failure::err_msg("Source attack properties not found"))?;
            (tp.bullet_speed, tp.atk_physic.v, 0)
        };

        let initial_dist: Fixed64 = (end_pos - pos).length();
        let move_speed_f = msd.to_f32_for_render();
        let initial_dist_f = initial_dist.to_f32_for_render();
        let flight_time_s: f32 = if move_speed_f > 0.0 { (initial_dist_f / move_speed_f).max(0.01) } else { 0.01 };
        let safety_time_left: Fixed64 = Fixed64::from_raw(((flight_time_s * 1.5 + 0.5) * omoba_sim::fixed::SCALE as f32) as i64);

        let e = ecs.create_entity()
            .with(Pos(pos))
            .with(Projectile {
                time_left: safety_time_left,
                owner: source_entity,
                tpos: end_pos,
                target: None,
                radius: Fixed64::ZERO,
                msd,
                damage_phys: atk_phys,
                damage_magi: Fixed64::ZERO,
                damage_real: Fixed64::ZERO,
                slow_factor: Fixed64::ZERO,
                slow_duration: Fixed64::ZERO,
                hit_radius: Fixed64::ZERO,
                stun_duration: Fixed64::ZERO,
            })
            .build();

        Ok(())
    }

    fn handle_tower_spawn(ecs: &mut World, mqtx: &crossbeam_channel::Sender<OutboundMsg>, pos: omoba_sim::Vec2, td: TowerData) -> Result<(), Error> {
        ecs.create_entity().with(Pos(pos)).with(Tower::new()).with(td.tpty).with(td.tatk).build();
        ecs.get_mut::<Searcher>().unwrap().tower.mark_dirty();
        Ok(())
    }
    
    fn handle_creep_stop(ecs: &mut World, mqtx: &crossbeam_channel::Sender<OutboundMsg>, source: Entity, target: Entity) -> Result<(), Error> {
        let mut creeps = ecs.write_storage::<Creep>();
        let c = creeps.get_mut(target).ok_or_else(|| failure::err_msg("Creep not found"))?;
        c.block_tower = Some(source);
        c.status = CreepStatus::Stop;

        let positions = ecs.read_storage::<Pos>();
        let pos = positions.get(target).ok_or_else(|| failure::err_msg("Creep position not found"))?;

        // Phase 5.2: legacy 0x02 GameEvent broadcast cut. Lockstep TickBatch
        // (0x10) carries authoritative state; client renders from sim.
        let (_px, _py) = pos.xy_f32();
        let _ = (mqtx, target);
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
        pos: omoba_sim::Vec2,
        phys: omoba_sim::Fixed64,
        magi: omoba_sim::Fixed64,
        real: omoba_sim::Fixed64,
        source: Entity,
        target: Entity,
        // P7: true 當本 damage 全部來自已 pre-declared 的非 AOE projectile
        // (client 已在 ProjectileCreate.damage 收到並排程 local 扣血)，
        // 此時跳過 creep.H 廣播省 bytes。若死亡仍照常發 entity.D。
        predeclared: bool,
    ) -> Result<(), Error> {
        use omoba_sim::Fixed64;
        let mut hp_after = Fixed64::ZERO;
        let mut max_hp = Fixed64::ZERO;
        let mut died = false;

        // damage_taken_bonus 聚合（Task 14）：目標身上所有 buff 的此 key sum_add
        // 例：Ice Embrittlement L3 對被減速 creep +25% 傷害
        let dmg_taken_bonus = {
            let bs = ecs.read_resource::<crate::ability_runtime::BuffStore>();
            bs.sum_add(target, StatKey::DamageTakenBonus)
        };
        let raw_mul = Fixed64::ONE + dmg_taken_bonus;
        let dmg_multiplier = if raw_mul < Fixed64::ZERO { Fixed64::ZERO } else { raw_mul };

        {
            let mut properties = ecs.write_storage::<CProperty>();
            if let Some(target_props) = properties.get_mut(target) {
                let hp_before = target_props.hp;
                let total_damage = (phys + magi + real) * dmg_multiplier;
                target_props.hp = target_props.hp - total_damage;
                hp_after = target_props.hp;
                max_hp = target_props.mhp;

                let (source_name, target_name) = Self::get_entity_names(ecs, source, target);

                // NOTE: log uses f32 boundary — Fixed64 has no Display.
                let damage_parts = {
                    let mut parts = Vec::new();
                    if phys > Fixed64::ZERO { parts.push(format!("Phys {:.1}", phys.to_f32_for_render())); }
                    if magi > Fixed64::ZERO { parts.push(format!("Magi {:.1}", magi.to_f32_for_render())); }
                    if real > Fixed64::ZERO { parts.push(format!("Pure {:.1}", real.to_f32_for_render())); }
                    if parts.is_empty() {
                        parts.push(format!("Total {:.1}", total_damage.to_f32_for_render()));
                    }
                    parts.join(", ")
                };

                log::debug!("⚔️ {} 攻擊 {} | {} damage | HP: {:.1} → {:.1}/{:.1}",
                    source_name, target_name, damage_parts,
                    hp_before.to_f32_for_render(), hp_after.to_f32_for_render(), target_props.mhp.to_f32_for_render()
                );

                if target_props.hp <= Fixed64::ZERO {
                    target_props.hp = Fixed64::ZERO;
                    hp_after = Fixed64::ZERO;
                    died = true;
                    // [DEBUG-STRESS] 死亡關鍵診斷：印 max_hp / hp_before / total_damage / source
                    // 篩 mhp > 100 跳過 1HP 塔本身的死亡（目前只關心 creep 怎麼死）
                    if max_hp > Fixed64::from_i32(100) {
                        log::info!("💀 {} died | max_hp={} hp_before={} dmg={:.1} (×{:.2}) source={}",
                            target_name,
                            max_hp.to_f32_for_render(),
                            hp_before.to_f32_for_render(),
                            total_damage.to_f32_for_render(),
                            dmg_multiplier.to_f32_for_render(),
                            source_name);
                    }
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

    fn handle_heal(ecs: &mut World, target: Entity, amount: omoba_sim::Fixed64) -> Result<(), Error> {
        let mut properties = ecs.write_storage::<CProperty>();
        if let Some(target_props) = properties.get_mut(target) {
            let summed = target_props.hp + amount;
            target_props.hp = if summed > target_props.mhp { target_props.mhp } else { summed };
        }
        Ok(())
    }

    fn handle_attack_update(ecs: &mut World, target: Entity, asd_count: Option<omoba_sim::Fixed64>, cooldown_reset: bool) -> Result<(), Error> {
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

    fn handle_gold_gain(ecs: &mut World, target: Entity, amount: i32) -> Result<(), Error> {
        if amount == 0 { return Ok(()); }
        let mut golds = ecs.write_storage::<Gold>();
        match golds.get_mut(target) {
            Some(g) => {
                g.0 = g.0.saturating_add(amount);
            }
            None => {
                let _ = golds.insert(target, Gold(amount.max(0)));
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