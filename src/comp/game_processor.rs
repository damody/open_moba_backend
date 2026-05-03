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
                        // Phase 4.2: route legacy `make_game_explosion` mqtx
                        // emit through the deterministic snapshot pipeline.
                        // Push into ExplosionFxQueue (non-state resource —
                        // sim never reads back, so determinism is unaffected);
                        // the omfx sim_runner extractor drains it each tick
                        // and the render thread spawns the ring scene node
                        // with omfx-wall-clock lifecycle.
                        let current_tick = ecs.read_resource::<Tick>().0 as u32;
                        let duration_ms = (duration.to_f32_for_render() * 1000.0)
                            .clamp(0.0, u32::MAX as f32)
                            as u32;
                        let mut q = ecs.write_resource::<ExplosionFxQueue>();
                        q.pending.push(ExplosionFx {
                            pos_x: pos.x.to_f32_for_render(),
                            pos_y: pos.y.to_f32_for_render(),
                            radius: radius.to_f32_for_render(),
                            duration_ms,
                            spawn_tick: current_tick,
                        });
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

    /// Phase 2.1: lockstep `PlayerInputEnum::TowerPlace` handler.
    ///
    /// Called from `drain_pending_tower_spawns` after `PendingTowerSpawnQueue`
    /// is filled by `tick::player_input_tick::Sys`. Maps `kind_id` (proto
    /// `TowerPlace.tower_kind_id` = `omoba_template_ids::TowerId.0` as u32) to
    /// the `unit_id` string the existing `tower_template::spawn_td_tower`
    /// expects, and delegates the actual entity construction + ScriptEvent::
    /// Spawn push there.
    ///
    /// Runs deterministically on both host (omb) and replica (omfx sim_runner)
    /// because the queue is filled identically on both sides from the same
    /// `TickBatch.inputs` and drained at the same dispatch boundary.
    pub fn handle_tower_spawn_from_input(
        world: &mut World,
        kind_id: u32,
        pos: omoba_sim::Vec2,
        owner_pid: u32,
    ) -> Result<specs::Entity, Error> {
        let tid = omoba_template_ids::TowerId(kind_id as u16);
        let unit_id = omoba_template_ids::tower_id_str(tid);
        if unit_id.is_empty() || unit_id == "?" {
            return Err(failure::err_msg(format!(
                "TowerPlace: unknown tower_kind_id {} (pid={})",
                kind_id, owner_pid
            )));
        }
        // spawn_td_tower expects Vec2<f32>; bridge through to_f32_for_render.
        let pos_f32 = vek::Vec2::new(pos.x.to_f32_for_render(), pos.y.to_f32_for_render());
        let entity = crate::comp::tower_template::spawn_td_tower(world, pos_f32, unit_id)
            .ok_or_else(|| failure::err_msg(format!(
                "spawn_td_tower returned None for unit_id='{}'", unit_id
            )))?;
        log::info!(
            "TowerPlace ok pid={} kind_id={} unit_id='{}' pos=({:.1},{:.1}) entity={:?}",
            owner_pid, kind_id, unit_id, pos_f32.x, pos_f32.y, entity
        );
        Ok(entity)
    }

    /// Phase 2.1: drain `PendingTowerSpawnQueue` and spawn each requested tower.
    /// Must be called after the dispatcher's `player_input_tick::Sys` has
    /// populated the queue but before the next snapshot extract — both host
    /// `state::core::tick` and replica `omfx sim_runner` invoke this.
    pub fn drain_pending_tower_spawns(world: &mut World) {
        let drained: Vec<crate::comp::PendingTowerSpawn> = {
            let mut q = world.write_resource::<crate::comp::PendingTowerSpawnQueue>();
            std::mem::take(&mut q.requests)
        };
        for req in drained {
            if let Err(e) = Self::handle_tower_spawn_from_input(world, req.kind_id, req.pos, req.owner_pid) {
                log::warn!(
                    "TowerPlace failed pid={} kind_id={}: {}",
                    req.owner_pid, req.kind_id, e
                );
            }
        }
    }

    /// Phase 2.2: lockstep `PlayerInputEnum::TowerSell` handler.
    ///
    /// Called from `drain_pending_tower_sells` after `PendingTowerSellQueue`
    /// is filled by `tick::player_input_tick::Sys`. Mirrors the existing
    /// MQTT/JSON `state::resource_management::sell_tower` convention:
    ///   * 85% base cost refund + 75% per upgrade level refund (read from
    ///     `TowerTemplateRegistry` + `TowerUpgradeRegistry` via the entity's
    ///     `ScriptUnitTag.unit_id`).
    ///   * Refund credited to the first `Hero` entity with `Faction == Player`
    ///     (TD mode = single-player wallet).
    ///   * Clear `BuffStore` for the doomed entity to prevent leaked buffs
    ///     (e.g. upgrade_* with f32::MAX duration).
    ///   * `world.entities().delete(...)` — Phase 1.6 snapshot diff
    ///     auto-removes from render via `removed_entity_ids`.
    ///
    /// Runs deterministically on both host (omb) and replica (omfx sim_runner)
    /// because the queue is filled identically on both sides from the same
    /// `TickBatch.inputs` and drained at the same dispatch boundary.
    pub fn handle_tower_sell_from_input(
        world: &mut World,
        tower_entity_id: u32,
        owner_pid: u32,
    ) -> Result<(), Error> {
        use specs::Join;

        // Resolve `Entity` from raw u32 id by joining over live entities. Specs
        // doesn't expose a stable `Entity::from_id` for non-test code; the
        // existing `mqtt_handler::sell_tower` site uses the same pattern.
        let target_entity = {
            let entities = world.entities();
            let towers = world.read_storage::<Tower>();
            let mut found = None;
            for (e, _t) in (&entities, &towers).join() {
                if e.id() == tower_entity_id {
                    found = Some(e);
                    break;
                }
            }
            found
        };
        let target_entity = match target_entity {
            Some(e) => e,
            None => {
                return Err(failure::err_msg(format!(
                    "TowerSell: tower entity id={} not found / not a Tower (pid={})",
                    tower_entity_id, owner_pid
                )));
            }
        };

        // Ownership check: TD only has FactionType::Player towers in the
        // single-player slot. If multi-player slots get added later this
        // check should also compare a per-tower owner_pid marker.
        {
            let factions = world.read_storage::<Faction>();
            match factions.get(target_entity) {
                Some(f) if f.faction_id == FactionType::Player => {}
                Some(f) => {
                    return Err(failure::err_msg(format!(
                        "TowerSell: tower id={} not Player-owned (faction={:?}, pid={})",
                        tower_entity_id, f.faction_id, owner_pid
                    )));
                }
                None => {
                    return Err(failure::err_msg(format!(
                        "TowerSell: tower id={} has no Faction component (pid={})",
                        tower_entity_id, owner_pid
                    )));
                }
            }
        }

        // Compute refund: 85% base + 75% per upgrade level. Mirrors
        // `state::resource_management::sell_tower` so the lockstep path stays
        // consistent with the legacy MQTT path.
        let refund = {
            let tags = world.read_storage::<crate::scripting::ScriptUnitTag>();
            let reg = world.read_resource::<crate::comp::tower_registry::TowerTemplateRegistry>();
            let towers = world.read_storage::<Tower>();
            let ureg = world.read_resource::<crate::comp::tower_upgrade_registry::TowerUpgradeRegistry>();
            let base_refund = tags
                .get(target_entity)
                .and_then(|t| reg.get(&t.unit_id))
                .map(|tpl| (tpl.cost as f32 * 0.85) as i32)
                .unwrap_or(0);
            let upgrade_refund = if let (Some(t), Some(tag)) = (towers.get(target_entity), tags.get(target_entity)) {
                let mut total = 0i32;
                for path in 0..3u8 {
                    for level in 1..=t.upgrade_levels[path as usize] {
                        if let Some(def) = ureg.get(&tag.unit_id, path, level) {
                            total += (def.cost as f32 * 0.75) as i32;
                        }
                    }
                }
                total
            } else {
                0
            };
            base_refund + upgrade_refund
        };

        // Find the player's hero (TD wallet). Single Player-faction Hero in
        // current TD mode; pick first match. Mirrors `sell_tower` lookup.
        let hero_entity = {
            let entities = world.entities();
            let heroes = world.read_storage::<Hero>();
            let factions = world.read_storage::<Faction>();
            let mut found = None;
            for (e, _h, f) in (&entities, &heroes, &factions).join() {
                if f.faction_id == FactionType::Player {
                    found = Some(e);
                    break;
                }
            }
            found
        };
        if let Some(hero_entity) = hero_entity {
            let mut golds = world.write_storage::<Gold>();
            if let Some(g) = golds.get_mut(hero_entity) {
                g.0 += refund;
            }
        }

        // Clear BuffStore residue (upgrade_* f32::MAX permanent buffs would
        // leak otherwise — see `state::resource_management::sell_tower`).
        {
            let mut store = world.write_resource::<crate::ability_runtime::BuffStore>();
            store.remove_all_for(target_entity);
        }

        // Delete entity. Phase 1.6 snapshot diff captures this in
        // `removed_entity_ids` so omfx render auto-cleans.
        world.entities().delete(target_entity).ok();

        log::info!(
            "TowerSell ok pid={} entity_id={} refund={}",
            owner_pid, tower_entity_id, refund
        );
        Ok(())
    }

    /// Phase 2.2: drain `PendingTowerSellQueue` and process each sell request.
    /// Must be called after the dispatcher's `player_input_tick::Sys` has
    /// populated the queue but before the next snapshot extract — both host
    /// `state::core::tick` and replica `omfx sim_runner` invoke this.
    pub fn drain_pending_tower_sells(world: &mut World) {
        let drained: Vec<crate::comp::PendingTowerSell> = {
            let mut q = world.write_resource::<crate::comp::PendingTowerSellQueue>();
            std::mem::take(&mut q.requests)
        };
        for req in drained {
            if let Err(e) = Self::handle_tower_sell_from_input(world, req.tower_entity_id, req.owner_pid) {
                log::warn!(
                    "TowerSell failed pid={} entity_id={}: {}",
                    req.owner_pid, req.tower_entity_id, e
                );
            }
        }
    }

    /// Phase 2.3: lockstep `PlayerInputEnum::TowerUpgrade` handler.
    ///
    /// Mirrors the gameplay logic of
    /// `state::resource_management::upgrade_tower` (the legacy MQTT entry):
    ///   * Resolve `Entity` from `tower_entity_id` (Tower-storage join +
    ///     `id() == tower_entity_id`).
    ///   * Validate `Faction == Player`.
    ///   * Compute target level = `tower.upgrade_levels[path] + 1`. The
    ///     `level` field on the proto is treated as a hint only — it would
    ///     otherwise force the omfx UI (Phase 4.3 hasn't yet exposed
    ///     `upgrade_levels` via snapshot) to know the current level. Using
    ///     the entity-side state guarantees correctness whatever the client
    ///     sent.
    ///   * Run `tower_upgrade_rules::validate_upgrade` on the current levels +
    ///     path; reject if the rules say no.
    ///   * Look up `UpgradeDef` from `TowerUpgradeRegistry`; reject if none.
    ///   * Find Player-faction Hero (TD wallet); check Gold ≥ cost; deduct.
    ///   * Apply the def's `effects`:
    ///       - `BehaviorFlag` → push to `tower.upgrade_flags` if absent.
    ///       - `StatMod`      → add a `BuffStore` entry with key
    ///                          `upgrade_<path>_<level>_<i>` and a sentinel
    ///                          `Fixed64::from_raw(i64::MAX)` duration so it
    ///                          never expires (matches legacy convention).
    ///   * Increment `tower.upgrade_levels[path]`.
    ///
    /// Runs deterministically on both host (omb) and replica (omfx sim_runner)
    /// because the queue is filled identically on both sides from the same
    /// `TickBatch.inputs` and drained at the same dispatch boundary.
    pub fn handle_tower_upgrade_from_input(
        world: &mut World,
        tower_entity_id: u32,
        path: u8,
        _level_hint: u8,
        owner_pid: u32,
    ) -> Result<(), Error> {
        use specs::Join;
        use omoba_core::tower_meta::UpgradeEffect;

        if path >= 3 {
            return Err(failure::err_msg(format!(
                "TowerUpgrade: invalid path={} (must be 0..=2) pid={}",
                path, owner_pid
            )));
        }

        // Resolve target tower entity + capture levels + unit_id.
        let target = {
            let entities = world.entities();
            let towers = world.read_storage::<Tower>();
            let tags = world.read_storage::<crate::scripting::ScriptUnitTag>();
            let mut found: Option<(specs::Entity, [u8; 3], String)> = None;
            for (e, t, tag) in (&entities, &towers, &tags).join() {
                if e.id() == tower_entity_id {
                    found = Some((e, t.upgrade_levels, tag.unit_id.clone()));
                    break;
                }
            }
            found
        };
        let (target_entity, levels, unit_id) = match target {
            Some(t) => t,
            None => {
                return Err(failure::err_msg(format!(
                    "TowerUpgrade: tower id={} not found / not a Tower (pid={})",
                    tower_entity_id, owner_pid
                )));
            }
        };

        // Ownership check (mirror handle_tower_sell_from_input).
        {
            let factions = world.read_storage::<Faction>();
            match factions.get(target_entity) {
                Some(f) if f.faction_id == FactionType::Player => {}
                Some(f) => {
                    return Err(failure::err_msg(format!(
                        "TowerUpgrade: tower id={} not Player-owned (faction={:?}, pid={})",
                        tower_entity_id, f.faction_id, owner_pid
                    )));
                }
                None => {
                    return Err(failure::err_msg(format!(
                        "TowerUpgrade: tower id={} has no Faction component (pid={})",
                        tower_entity_id, owner_pid
                    )));
                }
            }
        }

        // Rule validation (already-maxed / two-primary / two-secondary / etc.).
        if let Err(rej) = crate::comp::tower_upgrade_rules::validate_upgrade(levels, path) {
            return Err(failure::err_msg(format!(
                "TowerUpgrade: rule rejection eid={} path={} levels={:?} → {:?} (pid={})",
                tower_entity_id, path, levels, rej, owner_pid
            )));
        }
        let next_level = levels[path as usize] + 1;

        // Look up UpgradeDef (clone out to release the borrow on the
        // registry resource before we take other borrows).
        let def = {
            let reg = world.read_resource::<crate::comp::tower_upgrade_registry::TowerUpgradeRegistry>();
            reg.get(&unit_id, path, next_level).cloned()
        };
        let def = match def {
            Some(d) => d,
            None => {
                return Err(failure::err_msg(format!(
                    "TowerUpgrade: no UpgradeDef for kind={} path={} level={} (pid={})",
                    unit_id, path, next_level, owner_pid
                )));
            }
        };

        // Find player's hero (TD wallet) — mirror handle_tower_sell_from_input.
        let hero_entity = {
            let entities = world.entities();
            let heroes = world.read_storage::<Hero>();
            let factions = world.read_storage::<Faction>();
            let mut found = None;
            for (e, _h, f) in (&entities, &heroes, &factions).join() {
                if f.faction_id == FactionType::Player {
                    found = Some(e);
                    break;
                }
            }
            found
        };
        let hero_entity = match hero_entity {
            Some(e) => e,
            None => {
                return Err(failure::err_msg(format!(
                    "TowerUpgrade: no Player-faction Hero entity (pid={})",
                    owner_pid
                )));
            }
        };

        // Gold check (read).
        let has_gold = {
            let golds = world.read_storage::<Gold>();
            golds.get(hero_entity).map(|g| g.0).unwrap_or(0) >= def.cost
        };
        if !has_gold {
            return Err(failure::err_msg(format!(
                "TowerUpgrade: insufficient gold (need {}) for kind={} path={} level={} (pid={})",
                def.cost, unit_id, path, next_level, owner_pid
            )));
        }

        // Deduct gold.
        {
            let mut golds = world.write_storage::<Gold>();
            if let Some(g) = golds.get_mut(hero_entity) {
                g.0 -= def.cost;
            }
        }

        // Sort effects into flag adds + stat-mod buff entries (so we can
        // collect them without holding overlapping borrows on storages).
        let mut flags_to_add: Vec<String> = Vec::new();
        let mut stat_mods: Vec<(String, serde_json::Value)> = Vec::new();
        for (effect_idx, effect) in def.effects.iter().enumerate() {
            match effect {
                UpgradeEffect::BehaviorFlag { flag } => flags_to_add.push(flag.clone()),
                UpgradeEffect::StatMod { key, value, op: _ } => {
                    let buff_id = format!("upgrade_{}_{}_{}", path, next_level, effect_idx);
                    stat_mods.push((buff_id, json!({ key: *value })));
                }
            }
        }
        for (buff_id, payload) in stat_mods {
            let mut store = world.write_resource::<crate::ability_runtime::BuffStore>();
            // Sentinel "permanent" via raw i64::MAX, matches the legacy
            // upgrade_tower convention (BuffStore::add takes Fixed64).
            store.add(target_entity, &buff_id, omoba_sim::Fixed64::from_raw(i64::MAX), payload);
        }

        // Increment upgrade_levels[path] + dedupe upgrade_flags.
        {
            let mut towers = world.write_storage::<Tower>();
            if let Some(t) = towers.get_mut(target_entity) {
                for flag in flags_to_add {
                    if !t.upgrade_flags.iter().any(|f| f == &flag) {
                        t.upgrade_flags.push(flag);
                    }
                }
                t.upgrade_levels[path as usize] = next_level;
            }
        }

        log::info!(
            "TowerUpgrade ok pid={} eid={} kind={} path={} level={} cost={}",
            owner_pid, tower_entity_id, unit_id, path, next_level, def.cost
        );
        Ok(())
    }

    /// Phase 2.3: drain `PendingTowerUpgradeQueue` and process each upgrade.
    /// Must be called after the dispatcher's `player_input_tick::Sys` has
    /// populated the queue but before the next snapshot extract — both host
    /// `state::core::tick` and replica `omfx sim_runner` invoke this.
    pub fn drain_pending_tower_upgrades(world: &mut World) {
        let drained: Vec<crate::comp::PendingTowerUpgrade> = {
            let mut q = world.write_resource::<crate::comp::PendingTowerUpgradeQueue>();
            std::mem::take(&mut q.requests)
        };
        for req in drained {
            if let Err(e) = Self::handle_tower_upgrade_from_input(
                world, req.tower_entity_id, req.path, req.level, req.owner_pid,
            ) {
                log::warn!(
                    "TowerUpgrade failed pid={} eid={} path={}: {}",
                    req.owner_pid, req.tower_entity_id, req.path, e
                );
            }
        }
    }

    /// Phase 2.4: lockstep `PlayerInputEnum::ItemUse` handler.
    ///
    /// Mirrors the gameplay logic of
    /// `state::resource_management::use_item` (the legacy MQTT entry):
    ///   * Validate `slot < INVENTORY_SLOTS`.
    ///   * Find the Player-faction Hero entity (TD single-player wallet);
    ///     same lookup pattern as TowerSell / TowerUpgrade.
    ///   * Read the slot, look up `ItemConfig` via `ItemRegistry`.
    ///   * Reject if no item / cooldown not ready / no `active` effect.
    ///   * Apply the active effect to the hero's `CProperty` (Shield → HP up
    ///     to `mhp`, SprintBuff → `msd += bonus`, others log-only for now —
    ///     matches the legacy MVP).
    ///   * Set `item.cooldown_remaining = cfg.cooldown`.
    ///
    /// `target_pos` / `target_entity` from the proto are accepted but not
    /// used by the current effect set; they are forwarded for future
    /// targeted-active items.
    ///
    /// Runs deterministically on both host (omb) and replica (omfx sim_runner)
    /// because the queue is filled identically on both sides from the same
    /// `TickBatch.inputs` and drained at the same dispatch boundary.
    pub fn handle_item_use_from_input(
        world: &mut World,
        item_slot: u32,
        _target_pos: Option<omoba_sim::Vec2>,
        _target_entity: Option<u32>,
        owner_pid: u32,
    ) -> Result<(), Error> {
        use specs::Join;
        use crate::comp::inventory::INVENTORY_SLOTS;

        let slot_i = item_slot as usize;
        if slot_i >= INVENTORY_SLOTS {
            return Err(failure::err_msg(format!(
                "ItemUse: invalid slot={} (max {}) pid={}",
                slot_i, INVENTORY_SLOTS, owner_pid
            )));
        }

        // Find the Player-faction hero (TD single-player wallet — same
        // pattern as handle_tower_sell_from_input). Multi-player support
        // would compare a per-player marker instead of the first hit.
        let hero_entity = {
            let entities = world.entities();
            let heroes = world.read_storage::<Hero>();
            let factions = world.read_storage::<Faction>();
            let mut found = None;
            for (e, _h, f) in (&entities, &heroes, &factions).join() {
                if f.faction_id == FactionType::Player {
                    found = Some(e);
                    break;
                }
            }
            found
        };
        let hero_entity = match hero_entity {
            Some(e) => e,
            None => {
                return Err(failure::err_msg(format!(
                    "ItemUse: no Player-faction Hero entity (pid={})",
                    owner_pid
                )));
            }
        };

        // Look up the slot's ItemConfig + readiness.
        let (item_cfg, can_use) = {
            let invs = world.read_storage::<crate::comp::Inventory>();
            let reg = world.read_resource::<crate::item::ItemRegistry>();
            if let Some(inv) = invs.get(hero_entity) {
                if let Some(Some(inst)) = inv.slots.get(slot_i) {
                    let cfg = reg.get(&inst.item_id);
                    let ready = inst.cooldown_remaining <= 0.0;
                    (cfg, ready)
                } else {
                    (None, false)
                }
            } else {
                (None, false)
            }
        };
        let cfg = match item_cfg {
            Some(c) => c,
            None => {
                return Err(failure::err_msg(format!(
                    "ItemUse: empty slot={} or unknown item (pid={})",
                    slot_i, owner_pid
                )));
            }
        };
        if !can_use {
            return Err(failure::err_msg(format!(
                "ItemUse: slot={} on cooldown (pid={})",
                slot_i, owner_pid
            )));
        }
        let active = match &cfg.active {
            Some(a) => a.clone(),
            None => {
                return Err(failure::err_msg(format!(
                    "ItemUse: slot={} item has no active effect (pid={})",
                    slot_i, owner_pid
                )));
            }
        };

        // Apply effect to hero CProperty. Mirrors the MVP from
        // state::resource_management::use_item — Shield + SprintBuff actually
        // mutate stats; the others log only (gameplay TBD, no buff system).
        {
            let mut props = world.write_storage::<CProperty>();
            if let Some(p) = props.get_mut(hero_entity) {
                match &active {
                    crate::item::ActiveEffect::Shield { amount, .. } => {
                        let amt_fx = omoba_sim::Fixed64::from_raw((*amount * 1024.0) as i64);
                        let summed = p.hp + amt_fx;
                        p.hp = if summed > p.mhp { p.mhp } else { summed };
                        log::info!("ItemUse Shield +{} HP pid={}", amount, owner_pid);
                    }
                    crate::item::ActiveEffect::RestoreMana { amount } => {
                        log::info!("ItemUse RestoreMana +{} MP pid={} (mp not wired in MVP)", amount, owner_pid);
                    }
                    crate::item::ActiveEffect::SprintBuff { ms_bonus, duration } => {
                        let bonus_fx = omoba_sim::Fixed64::from_raw((*ms_bonus * 1024.0) as i64);
                        p.msd += bonus_fx;
                        log::info!("ItemUse SprintBuff +{} ms {}s pid={} (MVP no expiry)", ms_bonus, duration, owner_pid);
                    }
                    crate::item::ActiveEffect::DamageReduce { percent, duration } => {
                        log::info!("ItemUse DamageReduce {}% {}s pid={} (buff pipeline TBD)", percent * 100.0, duration, owner_pid);
                    }
                    crate::item::ActiveEffect::HeadshotNext { bonus_damage } => {
                        log::info!("ItemUse HeadshotNext +{} dmg pid={} (projectile hook TBD)", bonus_damage, owner_pid);
                    }
                }
            }
        }

        // Start cooldown on the slot.
        {
            let mut invs = world.write_storage::<crate::comp::Inventory>();
            if let Some(inv) = invs.get_mut(hero_entity) {
                if let Some(Some(inst)) = inv.slots.get_mut(slot_i) {
                    inst.cooldown_remaining = cfg.cooldown;
                }
            }
        }

        log::info!(
            "ItemUse ok pid={} slot={} item={} cooldown={}s",
            owner_pid, slot_i, cfg.id, cfg.cooldown
        );
        Ok(())
    }

    /// Phase 2.4: drain `PendingItemUseQueue` and process each request.
    /// Must be called after the dispatcher's `player_input_tick::Sys` has
    /// populated the queue but before the next snapshot extract — both host
    /// `state::core::tick` and replica `omfx sim_runner` invoke this.
    pub fn drain_pending_item_uses(world: &mut World) {
        let drained: Vec<crate::comp::PendingItemUse> = {
            let mut q = world.write_resource::<crate::comp::PendingItemUseQueue>();
            std::mem::take(&mut q.requests)
        };
        for req in drained {
            if let Err(e) = Self::handle_item_use_from_input(
                world, req.item_slot, req.target_pos, req.target_entity, req.owner_pid,
            ) {
                log::warn!(
                    "ItemUse failed pid={} slot={}: {}",
                    req.owner_pid, req.item_slot, e
                );
            }
        }
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