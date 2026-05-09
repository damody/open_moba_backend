use failure::Error;
use omb_script_abi::stat_keys::StatKey;
use serde_json::json;
use specs::{
    storage::{ReadStorage, WriteStorage},
    Builder, Entity, World, WorldExt,
};
use std::collections::BTreeMap;
use std::time::Instant;

use crate::comp::*;
use crate::transport::OutboundMsg;
use crate::Outcome;
use crate::Projectile;

/// game_processor 的每實體 SimRng op_kind。階段 1de.2：取代 fastrand
/// 彈體精準度+攻擊眩暈擲骰。重新排序或重複使用這些
/// 跨系統的常數將使重播決定論失效。
const OP_PROJECTILE_ACCURACY: u32 = 20;
const OP_PROJECTILE_STUN_ROLL: u32 = 21;

// ============================================================================
// P2 類型有效負載助手 — 在「kcp」後面進行門控。對於非 kcp 建置助手
// 回退到傳統的僅 JSON OutboundMsg 構造。
// ============================================================================

/// 遊戲.生活
#[inline]
fn make_game_lives(lives: i32) -> OutboundMsg {
    #[cfg(feature = "kcp")]
    {
        use crate::state::resource_management::proto_build;
        use crate::transport::TypedOutbound;
        // P5：全遊戲範圍的事件－觸及每位玩家。
        OutboundMsg::new_typed_all(
            "td/all/res",
            "game",
            "lives",
            TypedOutbound::GameLives(proto_build::game_lives(lives)),
            json!({ "lives": lives }),
        )
    }
    #[cfg(not(feature = "kcp"))]
    {
        OutboundMsg::new_s("td/all/res", "game", "lives", json!({ "lives": lives }))
    }
}

/// 遊戲結束
#[inline]
fn make_game_end(winner: &str, extra: serde_json::Value) -> OutboundMsg {
    #[cfg(feature = "kcp")]
    {
        use crate::state::resource_management::proto_build;
        use crate::transport::TypedOutbound;
        // P5：遊戲結束向所有玩家廣播。
        OutboundMsg::new_typed_all(
            "td/all/res",
            "game",
            "end",
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
        Outcome::EntityRemoved { .. } => "EntityRemoved",
    }
}

pub struct GameProcessor;

impl GameProcessor {
    pub fn process_outcomes(
        ecs: &mut World,
        mqtx: &crossbeam_channel::Sender<OutboundMsg>,
    ) -> Result<(), Error> {
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
                        phys: p,
                        magi: m,
                        real: r,
                        target: t,
                        predeclared: pd,
                        ..
                    } = &out
                    {
                        if let Some(&idx) = first_dmg_idx.get(t) {
                            if let Outcome::Damage {
                                phys: ap,
                                magi: am,
                                real: ar,
                                predeclared: apd,
                                ..
                            } = &mut aggregated[idx]
                            {
                                *ap += *p;
                                *am += *m;
                                *ar += *r;
                                // P7：與組合 — 僅當 ALL 時才跳過 H
                                // contributors were pre-declared.混合
                                // （預先聲明+權威）回落到
                                // 發出 H，以便伺服器保持權威。
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
                variant_timings.push(("DamageMerge", damage_merge_start.elapsed().as_nanos()));
            }

            for out in outcomes {
                let kind = outcome_kind(&out);
                let t0 = Instant::now();
                match out {
                    Outcome::Death { pos: p, ent: e } => {
                        remove_uids.push(e);
                        Self::handle_death(ecs, &mut next_outcomes, mqtx, e)?;
                    }
                    Outcome::ProjectileLine2 {
                        pos,
                        source,
                        target,
                    } => {
                        Self::handle_projectile(ecs, mqtx, pos, source, target)?;
                    }
                    Outcome::ProjectileDirectional {
                        pos,
                        source,
                        end_pos,
                    } => {
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
                    Outcome::Damage {
                        pos,
                        phys,
                        magi,
                        real,
                        source,
                        target,
                        predeclared,
                    } => {
                        Self::handle_damage(
                            ecs,
                            &mut next_outcomes,
                            pos,
                            phys,
                            magi,
                            real,
                            source,
                            target,
                            predeclared,
                        )?;
                    }
                    Outcome::Heal {
                        pos,
                        target,
                        amount,
                    } => {
                        Self::handle_heal(ecs, target, amount)?;
                    }
                    Outcome::UpdateAttack {
                        target,
                        asd_count,
                        cooldown_reset,
                    } => {
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
                    Outcome::AddBuff {
                        target,
                        buff_id,
                        duration,
                        payload,
                    } => {
                        Self::handle_add_buff(ecs, target, buff_id, duration, payload)?;
                    }
                    Outcome::Explosion {
                        pos,
                        radius,
                        duration,
                    } => {
                        // 階段 4.2：路由遺留 `make_game_explosion` mqtx
                        // 透過確定性快照管道發出。
                        // 推入 ExplosionFxQueue（非狀態資源 —
                        // sim 從不回讀，因此決定論不受影響）；
                        // omfx sim_runner 提取器在每個週期都會耗盡它
                        // 渲染線程產生環形場景節點
                        // 具有 omfx-wall-clock 生命週期。
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
                    Outcome::EntityRemoved { entity } => {
                        // Phase 1b: 唯一的 entity-deletion entry point in
                        // omb sim 路徑。 (a) 將id記錄到
                        // 已刪除實體佇列，因此下一個快照的
                        // returned_entity_ids 覆蓋它； (b) 原子標誌
                        // 透過規格實體刪除－實際存儲
                        // 清理工作在 world.maintain() 處運行
                        // 調度程序刻度線邊界。兩個步驟都運行在
                        // 這個 fn body — 伺服器和客戶端都到達
                        // process_outcomes 在同一邏輯點
                        // 他們各自的勾選，所以 StateHash 同意。
                        let mut q = ecs.write_resource::<crate::comp::RemovedEntitiesQueue>();
                        q.pending.push(entity.id());
                        let _ = ecs.entities().delete(entity);
                    }
                    _ => {}
                }
                variant_timings.push((kind, t0.elapsed().as_nanos()));
            }
        }

        ecs.delete_entities(&remove_uids[..]);
        ecs.write_resource::<Vec<Outcome>>().clear();
        ecs.write_resource::<Vec<Outcome>>()
            .append(&mut next_outcomes);

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
        entity: Entity,
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

        // 清理蠕變/塔交叉鏈接，這樣死亡的實體就不會懸空
        // 倖存者中的 block_tower / block_creeps 引用。僅副作用——
        // 實體本身被 process_outcomes 中的「delete_entities」刪除。
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
            log::info!(
                "🏆 敵方基地 entity {:?} destroyed — emitting game.end",
                entity
            );
            let _ = mqtx.send(make_game_end(
                "player",
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
                // 注意：賞金接近度是不確定的 UI 提示（派系範圍）；有損 f32 可接受。
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
        target: Option<Entity>,
    ) -> Result<(), Error> {
        use omoba_sim::{Fixed64, Vec2 as SimVec2};
        let source_entity = source.ok_or_else(|| failure::err_msg("Missing source entity"))?;
        let target_entity = target.ok_or_else(|| failure::err_msg("Missing target entity"))?;

        // 此 path 只用於非腳本塔（MOBA legacy）；TD 塔走腳本 `spawn_projectile_ex` 直接 spawn
        // 最終 damage 走 UnitStats::final_atk（聚合所有 stat_keys 官方 key）
        // 同時讀取 source 身上任何 buff 的 attack_stun_chance / attack_stun_duration，擲骰
        // 決定此發 projectile 命中後是否暈眩目標（matchlock_gun 的 87% 機率）
        // 另查 `multi_shot_visual` buff 決定是否額外 spawn 視覺子彈（無傷害）
        // 階段 1de.2：確定性 SimRng 輸入（master_seed + tick）
        // 準確度/眩暈擲骰。在handle_projectile的頂部讀一次
        // 避免在卷內重複進行資源查找。
        let master_seed: u64 = ecs.read_resource::<MasterSeed>().0;
        let tick: u32 = ecs.read_resource::<Tick>().0 as u32;
        let attacker_id: u32 = source_entity.id();

        let (msd, p2, atk_phys, stun_duration_roll, visual_count) = {
            let positions = ecs.read_storage::<Pos>();
            let tproperty = ecs.read_storage::<TAttack>();
            let buff_store = ecs.read_resource::<crate::ability_runtime::BuffStore>();
            let is_buildings = ecs.read_storage::<IsBuilding>();

            let _p1 = positions
                .get(source_entity)
                .ok_or_else(|| failure::err_msg("Source position not found"))?;
            let p2 = positions
                .get(target_entity)
                .ok_or_else(|| failure::err_msg("Target position not found"))?;
            let tp = tproperty
                .get(source_entity)
                .ok_or_else(|| failure::err_msg("Source attack properties not found"))?;
            let is_b = is_buildings.get(source_entity).is_some();
            let stats = crate::ability_runtime::UnitStats::from_refs(&*buff_store, is_b);
            let mut final_atk: Fixed64 = stats.final_atk(tp.atk_physic.v, source_entity);

            // Accuracy 擲骰：base 命中率 1.0 + sum(accuracy_bonus) buffs；clamp [0,1]。
            // miss → damage=0（projectile 仍飛行，前端可由 0 傷害判定顯示 miss）。
            // 階段 1de.2：確定性每個（攻擊者、OP_PROJECTILE_ACCURACY）流。
            let accuracy_bonus = buff_store.sum_add(
                source_entity,
                omb_script_abi::stat_keys::StatKey::AccuracyBonus,
            );
            let accuracy: Fixed64 =
                (Fixed64::ONE + accuracy_bonus).clamp(Fixed64::ZERO, Fixed64::ONE);
            if accuracy < Fixed64::ONE {
                let mut acc_rng = omoba_sim::SimRng::from_master_entity(
                    master_seed,
                    tick,
                    attacker_id,
                    OP_PROJECTILE_ACCURACY,
                );
                let roll: Fixed64 = acc_rng.gen_fixed64_unit();
                // 原始語意：miss iff roll > 準確性。與Fixed64統一
                // 在 [0,1) 網格上，`roll>=accuracy` 保留了丟失機率
                // （滾動 == 準確度在 1024 個桶中的一個發生碰撞 - 在遊戲容差範圍內）。
                if roll >= accuracy {
                    final_atk = Fixed64::ZERO;
                }
            }

            // 取 source 身上任一 buff 中最強的 attack_stun_chance + 對應 duration
            // 階段 1de.2：仍從 JSON 有效負載讀取 f64（BuffStore 有線格式）
            // 接受 i64 raw 和傳統 f64 — 請參閱 buff_store.rs）。
            let mut stun_chance = 0.0f32;
            let mut stun_duration = 0.0f32;
            for (_, entry) in buff_store.iter_for(source_entity) {
                let c = entry
                    .payload
                    .get("attack_stun_chance")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0) as f32;
                let d = entry
                    .payload
                    .get("attack_stun_duration")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0) as f32;
                if c > stun_chance {
                    stun_chance = c;
                    stun_duration = d;
                }
            }
            // 階段 1de.2：確定性每個（攻擊者、OP_PROJECTILE_STUN_ROLL）流。
            let stun_roll: Fixed64 = if stun_chance > 0.0 && stun_duration > 0.0 {
                let mut stun_rng = omoba_sim::SimRng::from_master_entity(
                    master_seed,
                    tick,
                    attacker_id,
                    OP_PROJECTILE_STUN_ROLL,
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
            let visual_count = if vc >= 2.0 {
                vc.round().max(1.0) as u32
            } else {
                1
            };

            (tp.bullet_speed, p2.0, final_atk, stun_roll, visual_count)
        };

        // 命中由 projectile_tick 的距離判定決定（target 接近時 step >= dist 即命中）。
        // time_left 為安全閥：flight_time_s * 3 + 3 秒，允許高速單位拖著子彈移動。
        let initial_dist: Fixed64 = (p2 - pos).length();
        // Flight_time math (s) 需要 f32 — 線路格式為 f32 ms，我們想要
        // .max(0.01) 箝位行為。僅在連線邊界處以 f32 進行計算。
        let move_speed_f = msd.to_f32_for_render();
        let initial_dist_f = initial_dist.to_f32_for_render();
        let flight_time_s: f32 = if move_speed_f > 0.0 {
            (initial_dist_f / move_speed_f).max(0.01)
        } else {
            0.01
        };
        let safety_time_left: Fixed64 = Fixed64::from_raw(
            ((flight_time_s * 3.0 + 3.0) * omoba_sim::fixed::SCALE as f32) as i64,
        );

        // Legacy path (MOBA 英雄 / 非腳本塔)：單體傷害、無 splash、無 slow
        let splash_radius: Fixed64 = Fixed64::ZERO;
        let slow_factor: Fixed64 = Fixed64::ZERO;
        let slow_duration: Fixed64 = Fixed64::ZERO;

        let ntarget = target_entity.id();
        let flight_time_ms: u64 = (flight_time_s * 1000.0).max(1.0) as u64;
        let kind_id: u16 = 0; // UNSPECIFIED — legacy handler, no template assignment

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
            let stun_this: Fixed64 = if is_real {
                stun_duration_roll
            } else {
                Fixed64::ZERO
            };
            let target_this = if is_real { target } else { None };
            // (i - 半) * 橫向步長;以Fixed64計算：一半可以是0.5
            // → 編碼為原始 512。
            let lateral: Fixed64 = if visual_count > 1 {
                let half_raw: i64 = ((visual_count as i64 - 1) * 512); // (n-1)/2 * SCALE
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

            let e = ecs
                .create_entity()
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

    fn handle_creep_spawn(
        ecs: &mut World,
        mqtx: &crossbeam_channel::Sender<OutboundMsg>,
        cd: CreepData,
    ) -> Result<(), Error> {
        let display_name = cd
            .creep
            .label
            .clone()
            .unwrap_or_else(|| cd.creep.name.clone());
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
        let e = ecs
            .create_entity()
            .with(Pos(pos)) // SimVec2 直接內嵌
            .with(cd.creep)
            .with(cd.cdata)
            .with(faction)
            .with(bounty)
            .with(Facing(omoba_sim::Angle::ZERO))
            .with(FacingBroadcast(None))
            .with(TurnSpeed(omoba_sim::Fixed64::from_raw(
                (turn_speed_rad_f * 1024.0) as i64,
            )))
            .with(crate::scripting::ScriptUnitTag {
                unit_id: unit_id.clone(),
            })
            .build();
        ecs.write_resource::<crate::scripting::ScriptEventQueue>()
            .push(crate::scripting::ScriptEvent::Spawn { e });
        // Default min-move-speed buff：避免多重 ice 減速把 ms 壓到 ≤ 1 觸發 frontend
        // lerp/extrap fallback 導致 creep 瞬移到下個 waypoint。
        // 用 BuffStore 寫入而非全域 clamp — 不同 creep 類型未來可以有不同下限、
        // 設計上也允許某些 buff 顯式拿掉這個下限（例如「凍結 1 秒」效果）。
        // 階段 1c.3：BuffStore::add 現在採用 Fix64 — 使用原始 i32::MAX 作為
        // 「永久」哨兵。注意：i32::MAX 持續時間是永久 buff 約定；可在第 2 階段以明確的 None/permanent 標誌取代。
        ecs.write_resource::<crate::ability_runtime::BuffStore>()
            .add(
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
            let _ = mqtx.try_send(make_game_end(
                "defeat",
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
        pos: omoba_sim::Vec2,
        source: Option<Entity>,
        end_pos: omoba_sim::Vec2,
    ) -> Result<(), Error> {
        use omoba_sim::Fixed64;
        use specs::{Builder, WorldExt};

        let source_entity =
            source.ok_or_else(|| failure::err_msg("ProjectileDirectional 缺少 source"))?;

        // 此 path 為 legacy（tower_tick 不再 push ProjectileDirectional；Tack 走腳本
        // spawn_projectile_ex）。保留 handle 作為備用；kind_id 留 0 (UNSPECIFIED)
        let (msd, atk_phys, kind_id): (Fixed64, Fixed64, u16) = {
            let tatks = ecs.read_storage::<TAttack>();
            let tp = tatks
                .get(source_entity)
                .ok_or_else(|| failure::err_msg("Source attack properties not found"))?;
            (tp.bullet_speed, tp.atk_physic.v, 0)
        };

        let initial_dist: Fixed64 = (end_pos - pos).length();
        let move_speed_f = msd.to_f32_for_render();
        let initial_dist_f = initial_dist.to_f32_for_render();
        let flight_time_s: f32 = if move_speed_f > 0.0 {
            (initial_dist_f / move_speed_f).max(0.01)
        } else {
            0.01
        };
        let safety_time_left: Fixed64 = Fixed64::from_raw(
            ((flight_time_s * 1.5 + 0.5) * omoba_sim::fixed::SCALE as f32) as i64,
        );

        let e = ecs
            .create_entity()
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

    fn handle_tower_spawn(
        ecs: &mut World,
        mqtx: &crossbeam_channel::Sender<OutboundMsg>,
        pos: omoba_sim::Vec2,
        td: TowerData,
    ) -> Result<(), Error> {
        ecs.create_entity()
            .with(Pos(pos))
            .with(Tower::new())
            .with(td.tpty)
            .with(td.tatk)
            .build();
        ecs.get_mut::<Searcher>().unwrap().tower.mark_dirty();
        Ok(())
    }

    /// 階段 2.1：鎖定步驟 `PlayerInputEnum::TowerPlace` 處理程序。
    ///
    /// 在“PendingTowerSpawnQueue”之後從“drain_pending_tower_spawns”調用
    /// 由 `tick::player_input_tick::Sys` 填滿。映射`kind_id`（原型
    /// `TowerPlace.tower_kind_id` = `omoba_template_ids::TowerId.0` 作為 u32) 到
    /// `unit_id` 字串現有的 `tower_template::spawn_td_tower`
    /// 期望並委託實際的實體建構 + ScriptEvent::
    /// 產卵推到那裡。
    ///
    /// 在主機 (omb) 和副本 (omfx sim_runner) 上確定性地運行
    /// 因為隊列兩邊的填充量相同
    /// `TickBatch.inputs` 並在同一調度邊界處耗盡。
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
        // spawn_td_tower 需要 Vec2<f32>；經由 to_f32_for_render 橋接。
        let pos_f32 = vek::Vec2::new(pos.x.to_f32_for_render(), pos.y.to_f32_for_render());
        let entity = crate::comp::tower_template::spawn_td_tower(world, pos_f32, unit_id)
            .ok_or_else(|| {
                failure::err_msg(format!(
                    "spawn_td_tower returned None for unit_id='{}'",
                    unit_id
                ))
            })?;
        log::info!(
            "TowerPlace ok pid={} kind_id={} unit_id='{}' pos=({:.1},{:.1}) entity={:?}",
            owner_pid,
            kind_id,
            unit_id,
            pos_f32.x,
            pos_f32.y,
            entity
        );
        Ok(entity)
    }

    /// 階段 2.1：排空 `PendingTowerSpawnQueue` 並產生每個請求的塔。
    /// 必須在調度程序的“player_input_tick::Sys”完成後調用
    /// 填充隊列，但在下一個快照提取之前 - 兩個主機
    /// `state::core::tick` 和副本 `omfx sim_runner` 呼叫它。
    pub fn drain_pending_tower_spawns(world: &mut World) {
        let drained: Vec<crate::comp::PendingTowerSpawn> = {
            let mut q = world.write_resource::<crate::comp::PendingTowerSpawnQueue>();
            std::mem::take(&mut q.requests)
        };
        for req in drained {
            if let Err(e) =
                Self::handle_tower_spawn_from_input(world, req.kind_id, req.pos, req.owner_pid)
            {
                log::warn!(
                    "TowerPlace failed pid={} kind_id={}: {}",
                    req.owner_pid,
                    req.kind_id,
                    e
                );
            }
        }
    }

    /// 階段 2.2：鎖定步驟 `PlayerInputEnum::TowerSell` 處理程序。
    ///
    /// 在“PendingTowerSellQueue”之後從“drain_pending_tower_sells”調用
    /// 由 `tick::player_input_tick::Sys` 填滿。反映現有的
    /// MQTT/JSON `state::resource_management::sell_tower` 約定：
    /// * 85% 基本成本退款 + 每個升級等級退款 75%（讀自
    /// `TowerTemplateRegistry` + `TowerUpgradeRegistry` 透過實體的
    /// `ScriptUnitTag.unit_id`)。
    /// * 退款記入第一個「Faction == Player」的「Hero」實體
    /// （TD模式=單人錢包）。
    /// * 清除注定實體的「BuffStore」以防止增益洩漏
    /// （例如，upgrade_* 的 f32::MAX 持續時間）。
    /// * `world.entities().delete(...)` — 階段 1.6 快照差異
    /// 透過「removed_entity_ids」自動從渲染中刪除。
    ///
    /// 在主機 (omb) 和副本 (omfx sim_runner) 上確定性地運行
    /// 因為隊列兩邊的填充量相同
    /// `TickBatch.inputs` 並在同一調度邊界處耗盡。
    pub fn handle_tower_sell_from_input(
        world: &mut World,
        tower_entity_id: u32,
        owner_pid: u32,
    ) -> Result<(), Error> {
        use specs::Join;

        // 透過加入活動實體​​，從原始 u32 id 解析「實體」。規格
        // 不會為非測試程式碼公開穩定的“Entity::from_id”；這
        // 現有的「mqtt_handler::sell_tower」網站使用相同的模式。
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

        // 所有權檢查：TD 中只有 FactionType::Player 塔
        // 單人老虎機。如果稍後再增加多人老虎機
        // 檢查也應該比較每個塔的owner_pid 標記。
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

        // 計算退款：85% 基礎 + 每個升級等級 75%。鏡子
        // `state::resource_management::sell_tower` 因此鎖步路徑保持不變
        // 與傳統 MQTT 路徑一致。
        let refund = {
            let tags = world.read_storage::<crate::scripting::ScriptUnitTag>();
            let reg = world.read_resource::<crate::comp::tower_registry::TowerTemplateRegistry>();
            let towers = world.read_storage::<Tower>();
            let ureg =
                world.read_resource::<crate::comp::tower_upgrade_registry::TowerUpgradeRegistry>();
            let base_refund = tags
                .get(target_entity)
                .and_then(|t| reg.get(&t.unit_id))
                .map(|tpl| (tpl.cost as f32 * 0.85) as i32)
                .unwrap_or(0);
            let upgrade_refund = if let (Some(t), Some(tag)) =
                (towers.get(target_entity), tags.get(target_entity))
            {
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

        // 找到玩家的英雄（TD錢包）。單人陣營英雄
        // 當前TD模式；選擇第一個匹配項。鏡像 `sell_tower` 查找。
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

        // 清除 BuffStore 殘留物（upgrade_* f32::MAX 永久增益將
        // 否則洩漏 - 請參閱“state::resource_management::sell_tower”）。
        {
            let mut store = world.write_resource::<crate::ability_runtime::BuffStore>();
            store.remove_all_for(target_entity);
        }

        // 階段 1.6：入隊 Outcome::EntityRemoved — process_outcomes
        // （在同一個tick中在drain_pending_*之後執行）處理
        // 實際的Entity().delete()+RemovedEntitiesQueue推送，以及
        // omfx 渲染透過 snapshot.removed_entity_ids 自動清理。
        world
            .write_resource::<Vec<Outcome>>()
            .push(Outcome::EntityRemoved {
                entity: target_entity,
            });

        log::info!(
            "TowerSell ok pid={} entity_id={} refund={}",
            owner_pid,
            tower_entity_id,
            refund
        );
        Ok(())
    }

    /// 階段 2.2：排空 `PendingTowerSellQueue` 並處理每個銷售請求。
    /// 必須在調度程序的“player_input_tick::Sys”完成後調用
    /// 填充隊列，但在下一個快照提取之前 - 兩個主機
    /// `state::core::tick` 和副本 `omfx sim_runner` 呼叫它。
    pub fn drain_pending_tower_sells(world: &mut World) {
        let drained: Vec<crate::comp::PendingTowerSell> = {
            let mut q = world.write_resource::<crate::comp::PendingTowerSellQueue>();
            std::mem::take(&mut q.requests)
        };
        for req in drained {
            if let Err(e) =
                Self::handle_tower_sell_from_input(world, req.tower_entity_id, req.owner_pid)
            {
                log::warn!(
                    "TowerSell failed pid={} entity_id={}: {}",
                    req.owner_pid,
                    req.tower_entity_id,
                    e
                );
            }
        }
    }

    /// 階段 2.3：鎖定步驟 `PlayerInputEnum::TowerUpgrade` 處理程序。
    ///
    /// 反映了遊戲的邏輯
    /// `state::resource_management::upgrade_tower`（舊版 MQTT 條目）：
    /// * 從`tower_entity_id`解析`Entity`（塔式儲存連線+
    /// `id() == tower_entity_id`)。
    /// * 驗證`派系==玩家`。
    /// * 計算目標等級 = `tower.upgrade_levels[path] + 1`。這
    /// 原型上的“level”字段僅被視為提示 - 它會
    /// 否則強制使用 omfx UI（階段 4.3 尚未公開
    /// `upgrade_levels` 透過快照）來了解目前層級。使用
    /// 無論客戶端如何，實體端狀態都保證正確性
    /// 發送。
    /// * 在目前層級執行 `tower_upgrade_rules::validate_upgrade` +
    /// 小路;如果規則不允許，則拒絕。
    /// * 從`TowerUpgradeRegistry`尋找`UpgradeDef`；如果沒有則拒絕。
    /// * 尋找玩家陣營英雄（TD錢包）；檢查金幣≥成本；扣除額。
    /// * 應用 def 的 `effects`:
    /// - `BehaviorFlag` → 如果不存在則推送到 `tower.upgrade_flags`。
    /// - `StatMod` → 新增一個有金鑰的 `BuffStore` 條目
    /// `upgrade_<path>_<level>_<i>` 和哨兵
    /// `Fixed64::from_raw(i64::MAX)` 持續時間所以它
    /// 永不過期（符合傳統約定）。
    /// * 增加 `tower.upgrade_levels[path]`。
    ///
    /// 在主機 (omb) 和副本 (omfx sim_runner) 上確定性地運行
    /// 因為隊列兩邊的填充量相同
    /// `TickBatch.inputs` 並在同一調度邊界處耗盡。
    pub fn handle_tower_upgrade_from_input(
        world: &mut World,
        tower_entity_id: u32,
        path: u8,
        _level_hint: u8,
        owner_pid: u32,
    ) -> Result<(), Error> {
        use omoba_core::tower_meta::UpgradeEffect;
        use specs::Join;

        if path >= 3 {
            return Err(failure::err_msg(format!(
                "TowerUpgrade: invalid path={} (must be 0..=2) pid={}",
                path, owner_pid
            )));
        }

        // 解析目標塔實體+捕獲等級+unit_id。
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

        // 所有權檢查（鏡像handle_tower_sell_from_input）。
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

        // 規則驗證（已最大化/兩個主要/兩個輔助/等）。
        if let Err(rej) = crate::comp::tower_upgrade_rules::validate_upgrade(levels, path) {
            return Err(failure::err_msg(format!(
                "TowerUpgrade: rule rejection eid={} path={} levels={:?} → {:?} (pid={})",
                tower_entity_id, path, levels, rej, owner_pid
            )));
        }
        let next_level = levels[path as usize] + 1;

        // 尋找 UpgradeDef（克隆出來以釋放對
        // 在我們借用其他資源之前先註冊資源）。
        let def = {
            let reg =
                world.read_resource::<crate::comp::tower_upgrade_registry::TowerUpgradeRegistry>();
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

        // 找到玩家的英雄（TD錢包）－鏡像handle_tower_sell_from_input。
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

        // 黃金支票（閱讀）。
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

        // 扣金。
        {
            let mut golds = world.write_storage::<Gold>();
            if let Some(g) = golds.get_mut(hero_entity) {
                g.0 -= def.cost;
            }
        }

        // 將效果分類為 flag adds + stat-mod buff 條目（這樣我們就可以
        // 收集它們而不在存儲上持有重疊的借用）。
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
            // Sentinel 透過原始 i64::MAX 實現“永久”，與傳統版本相匹配
            // Upgrade_tower 約定（BuffStore::add 採用 Fix64）。
            store.add(
                target_entity,
                &buff_id,
                omoba_sim::Fixed64::from_raw(i64::MAX),
                payload,
            );
        }

        // 遞增upgrade_levels[path] + 重複資料刪除upgrade_flags。
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
            owner_pid,
            tower_entity_id,
            unit_id,
            path,
            next_level,
            def.cost
        );
        Ok(())
    }

    /// 階段 2.3：排空 `PendingTowerUpgradeQueue` 並處理每個升級。
    /// 必須在調度程序的“player_input_tick::Sys”完成後調用
    /// 填充隊列，但在下一個快照提取之前 - 兩個主機
    /// `state::core::tick` 和副本 `omfx sim_runner` 呼叫它。
    pub fn drain_pending_tower_upgrades(world: &mut World) {
        let drained: Vec<crate::comp::PendingTowerUpgrade> = {
            let mut q = world.write_resource::<crate::comp::PendingTowerUpgradeQueue>();
            std::mem::take(&mut q.requests)
        };
        for req in drained {
            if let Err(e) = Self::handle_tower_upgrade_from_input(
                world,
                req.tower_entity_id,
                req.path,
                req.level,
                req.owner_pid,
            ) {
                log::warn!(
                    "TowerUpgrade failed pid={} eid={} path={}: {}",
                    req.owner_pid,
                    req.tower_entity_id,
                    req.path,
                    e
                );
            }
        }
    }

    /// Lockstep `PlayerInputEnum::UpgradeAbility` 處理程序。
    ///
    /// 更新 Player faction hero 的 ability level、消耗一點 skill point，並排入
    /// SkillLearn，讓 passive/on-learn scripts 在同一 tick 的 script dispatch
    /// 階段執行。
    pub fn handle_ability_upgrade_from_input(
        world: &mut World,
        ability_index: u32,
        owner_pid: u32,
    ) -> Result<(), Error> {
        use specs::Join;

        if ability_index >= 4 {
            return Err(failure::err_msg(format!(
                "AbilityUpgrade: invalid ability_index={} (must be 0..=3) pid={}",
                ability_index, owner_pid
            )));
        }
        let slot = ability_index as usize;

        let hero_entity = {
            let entities = world.entities();
            let heroes = world.read_storage::<Hero>();
            let factions = world.read_storage::<Faction>();
            (&entities, &heroes, &factions)
                .join()
                .find(|(_, _, f)| f.faction_id == FactionType::Player)
                .map(|(e, _, _)| e)
        };
        let hero_entity = hero_entity.ok_or_else(|| {
            failure::err_msg(format!(
                "AbilityUpgrade: no Player-faction Hero entity (pid={})",
                owner_pid
            ))
        })?;

        let ability_id = {
            let heroes = world.read_storage::<Hero>();
            let hero = heroes.get(hero_entity).ok_or_else(|| {
                failure::err_msg(format!(
                    "AbilityUpgrade: hero entity vanished before read (pid={})",
                    owner_pid
                ))
            })?;
            hero.abilities
                .get(slot)
                .filter(|id| !id.is_empty())
                .cloned()
                .ok_or_else(|| {
                    failure::err_msg(format!(
                        "AbilityUpgrade: hero has no ability bound at slot={} (pid={})",
                        slot, owner_pid
                    ))
                })?
        };

        let max_level = {
            let registry = world.read_resource::<crate::ability_runtime::AbilityRegistry>();
            registry
                .get(&ability_id)
                .map(|def| i32::from(def.max_level).max(1))
                .unwrap_or(5)
        };

        let new_level = {
            let mut heroes = world.write_storage::<Hero>();
            let hero = heroes.get_mut(hero_entity).ok_or_else(|| {
                failure::err_msg(format!(
                    "AbilityUpgrade: hero entity vanished before write (pid={})",
                    owner_pid
                ))
            })?;
            if hero.skill_points <= 0 {
                return Err(failure::err_msg(format!(
                    "AbilityUpgrade: no skill points slot={} ability='{}' pid={}",
                    slot, ability_id, owner_pid
                )));
            }
            let current = hero.ability_levels.get(&ability_id).copied().unwrap_or(0);
            if current >= max_level {
                return Err(failure::err_msg(format!(
                    "AbilityUpgrade: slot={} ability='{}' already maxed ({}/{}) pid={}",
                    slot, ability_id, current, max_level, owner_pid
                )));
            }
            let next = current + 1;
            hero.ability_levels.insert(ability_id.clone(), next);
            hero.skill_points -= 1;
            next
        };

        world
            .write_resource::<crate::scripting::ScriptEventQueue>()
            .push(crate::scripting::ScriptEvent::SkillLearn {
                caster: hero_entity,
                skill_id: ability_id.clone(),
                new_level: new_level.max(1) as u8,
            });

        log::info!(
            "AbilityUpgrade ok pid={} slot={} ability='{}' level={}",
            owner_pid,
            slot,
            ability_id,
            new_level
        );
        Ok(())
    }

    /// 在 player_input_tick 之後、script dispatch 之前 drain queued ability upgrades。
    pub fn drain_pending_ability_upgrades(world: &mut World) {
        let drained: Vec<crate::comp::PendingAbilityUpgrade> = {
            let mut q = world.write_resource::<crate::comp::PendingAbilityUpgradeQueue>();
            std::mem::take(&mut q.requests)
        };
        for req in drained {
            if let Err(e) =
                Self::handle_ability_upgrade_from_input(world, req.ability_index, req.owner_pid)
            {
                log::warn!(
                    "AbilityUpgrade failed pid={} ability_index={}: {}",
                    req.owner_pid,
                    req.ability_index,
                    e
                );
            }
        }
    }

    /// Lockstep `PlayerInputEnum::CastAbility` 處理程序。
    ///
    /// 解析 Player faction hero，驗證 slot 與 learned/cooldown gate，接著排入
    /// SkillCast，讓同一 tick 的 script dispatch 處理。
    pub fn handle_ability_cast_from_input(
        world: &mut World,
        ability_index: u32,
        target_pos: Option<omoba_sim::Vec2>,
        target_entity: Option<u32>,
        owner_pid: u32,
    ) -> Result<(), Error> {
        use specs::Join;

        if ability_index >= 4 {
            return Err(failure::err_msg(format!(
                "AbilityCast: invalid ability_index={} (must be 0..=3) pid={}",
                ability_index, owner_pid
            )));
        }
        let slot = ability_index as usize;

        let caster = {
            let entities = world.entities();
            let heroes = world.read_storage::<Hero>();
            let factions = world.read_storage::<Faction>();
            (&entities, &heroes, &factions)
                .join()
                .find(|(_, _, f)| f.faction_id == FactionType::Player)
                .map(|(e, _, _)| e)
        };
        let caster = caster.ok_or_else(|| {
            failure::err_msg(format!(
                "AbilityCast: no Player-faction Hero entity (pid={})",
                owner_pid
            ))
        })?;

        let ability_id = {
            let heroes = world.read_storage::<Hero>();
            let hero = heroes.get(caster).ok_or_else(|| {
                failure::err_msg(format!(
                    "AbilityCast: hero entity vanished before read (pid={})",
                    owner_pid
                ))
            })?;
            hero.abilities
                .get(slot)
                .filter(|id| !id.is_empty())
                .cloned()
                .ok_or_else(|| {
                    failure::err_msg(format!(
                        "AbilityCast: hero has no ability bound at slot={} (pid={})",
                        slot, owner_pid
                    ))
                })?
        };

        {
            let heroes = world.read_storage::<Hero>();
            let hero = heroes.get(caster).ok_or_else(|| {
                failure::err_msg(format!(
                    "AbilityCast: hero entity vanished before gate (pid={})",
                    owner_pid
                ))
            })?;
            if !hero.can_use_ability(&ability_id) {
                return Err(failure::err_msg(format!(
                    "AbilityCast: slot={} ability='{}' not learned (pid={})",
                    slot, ability_id, owner_pid
                )));
            }
            if hero.is_on_cooldown(&ability_id) {
                return Err(failure::err_msg(format!(
                    "AbilityCast: slot={} ability='{}' still on cooldown ({:.2}s) pid={}",
                    slot,
                    ability_id,
                    hero.get_cooldown(&ability_id).to_f32_for_render(),
                    owner_pid
                )));
            }
        }

        let target = if let Some(entity_id) = target_entity {
            let entities = world.entities();
            (&entities)
                .join()
                .find(|e| e.id() == entity_id)
                .map(crate::scripting::event::SkillTarget::Entity)
                .unwrap_or(crate::scripting::event::SkillTarget::None)
        } else if let Some(pos) = target_pos {
            crate::scripting::event::SkillTarget::Point { x: pos.x, y: pos.y }
        } else {
            crate::scripting::event::SkillTarget::None
        };

        world
            .write_resource::<crate::scripting::ScriptEventQueue>()
            .push(crate::scripting::ScriptEvent::SkillCast {
                caster,
                skill_id: ability_id.clone(),
                target,
            });

        log::info!(
            "AbilityCast ok pid={} slot={} ability='{}'",
            owner_pid,
            slot,
            ability_id
        );
        Ok(())
    }

    /// 在 player_input_tick 之後、script dispatch 之前 drain queued ability casts。
    pub fn drain_pending_ability_casts(world: &mut World) {
        let drained: Vec<crate::comp::PendingAbilityCast> = {
            let mut q = world.write_resource::<crate::comp::PendingAbilityCastQueue>();
            std::mem::take(&mut q.requests)
        };
        for req in drained {
            if let Err(e) = Self::handle_ability_cast_from_input(
                world,
                req.ability_index,
                req.target_pos,
                req.target_entity,
                req.owner_pid,
            ) {
                log::warn!(
                    "AbilityCast failed pid={} ability_index={}: {}",
                    req.owner_pid,
                    req.ability_index,
                    e
                );
            }
        }
    }

    /// 階段 2.4：鎖定步驟 `PlayerInputEnum::ItemUse` 處理程序。
    ///
    /// 反映了遊戲的邏輯
    /// `state::resource_management::use_item`（舊版 MQTT 條目）：
    /// * 驗證`slot < INVENTORY_SLOTS`。
    /// * 找到玩家派系英雄實體（TD單人錢包）；
    /// 與 TowerSell / TowerUpgrade 相同的尋找模式。
    /// * 讀取槽位，透過`ItemRegistry`查找`ItemConfig`。
    /// * 如果沒有物品/冷卻時間未準備好/沒有「活動」效果，則拒絕。
    /// * 將主動效果應用於英雄的`C屬性`（盾牌→HP up
    /// 到 `mhp`，SprintBuff → `msd += Bonus`，其他的暫時只記錄 —
    /// 與傳統 MVP 匹配）。
    /// * 設定 `item.cooldown_remaining = cfg.cooldown`。
    ///
    /// 接受原型中的“target_pos”/“target_entity”，但不接受
    /// 由當前效果集使用；它們被轉發給未來
    /// 有針對性的活動項目。
    ///
    /// 在主機 (omb) 和副本 (omfx sim_runner) 上確定性地運行
    /// 因為隊列兩邊的填充量相同
    /// `TickBatch.inputs` 並在同一調度邊界處耗盡。
    pub fn handle_item_use_from_input(
        world: &mut World,
        item_slot: u32,
        _target_pos: Option<omoba_sim::Vec2>,
        _target_entity: Option<u32>,
        owner_pid: u32,
    ) -> Result<(), Error> {
        use crate::comp::inventory::INVENTORY_SLOTS;
        use specs::Join;

        let slot_i = item_slot as usize;
        if slot_i >= INVENTORY_SLOTS {
            return Err(failure::err_msg(format!(
                "ItemUse: invalid slot={} (max {}) pid={}",
                slot_i, INVENTORY_SLOTS, owner_pid
            )));
        }

        // 找到玩家派系英雄（TD單人錢包 - 相同
        // 模式為handle_tower_sell_from_input）。多人支持
        // 會比較每個玩家的標記而不是第一次命中。
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

        // 尋找插槽的 ItemConfig + 準備情況。
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

        // 將效果應用於英雄 CProperty。反映 MVP 來自
        // state::resource_management::use_item — 實際上是 Shield + SprintBuff
        // 改變統計數據；其他的僅記錄（遊戲玩法待定，無增益系統）。
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
                        log::info!(
                            "ItemUse RestoreMana +{} MP pid={} (mp not wired in MVP)",
                            amount,
                            owner_pid
                        );
                    }
                    crate::item::ActiveEffect::SprintBuff { ms_bonus, duration } => {
                        let bonus_fx = omoba_sim::Fixed64::from_raw((*ms_bonus * 1024.0) as i64);
                        p.msd += bonus_fx;
                        log::info!(
                            "ItemUse SprintBuff +{} ms {}s pid={} (MVP no expiry)",
                            ms_bonus,
                            duration,
                            owner_pid
                        );
                    }
                    crate::item::ActiveEffect::DamageReduce { percent, duration } => {
                        log::info!(
                            "ItemUse DamageReduce {}% {}s pid={} (buff pipeline TBD)",
                            percent * 100.0,
                            duration,
                            owner_pid
                        );
                    }
                    crate::item::ActiveEffect::HeadshotNext { bonus_damage } => {
                        log::info!(
                            "ItemUse HeadshotNext +{} dmg pid={} (projectile hook TBD)",
                            bonus_damage,
                            owner_pid
                        );
                    }
                }
            }
        }

        // 在插槽上開始冷卻。
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
            owner_pid,
            slot_i,
            cfg.id,
            cfg.cooldown
        );
        Ok(())
    }

    /// 階段 2.4：排出 `PendingItemUseQueue` 並處理每個請求。
    /// 必須在調度程序的“player_input_tick::Sys”完成後調用
    /// 填充隊列，但在下一個快照提取之前 - 兩個主機
    /// `state::core::tick` 和副本 `omfx sim_runner` 呼叫它。
    pub fn drain_pending_item_uses(world: &mut World) {
        let drained: Vec<crate::comp::PendingItemUse> = {
            let mut q = world.write_resource::<crate::comp::PendingItemUseQueue>();
            std::mem::take(&mut q.requests)
        };
        for req in drained {
            if let Err(e) = Self::handle_item_use_from_input(
                world,
                req.item_slot,
                req.target_pos,
                req.target_entity,
                req.owner_pid,
            ) {
                log::warn!(
                    "ItemUse failed pid={} slot={}: {}",
                    req.owner_pid,
                    req.item_slot,
                    e
                );
            }
        }
    }

    /// MoveTo (右鍵移動): drain `PendingMoveQueue` and write `MoveTarget`
    /// 玩家英雄實體上的組件。 TD模式：單人錢包，
    /// 選擇第一個「(Hero, Faction == Player)」實體。確定性：隊列是
    /// 來自相同“TickBatch.inputs”的主機+副本上的填充相同，
    /// 在同一調度邊界耗盡，英雄查找使用相同的連接
    /// 雙方訂購。
    pub fn drain_pending_moves(world: &mut World) {
        use specs::Join;
        let drained: Vec<crate::comp::PendingMoveTo> = {
            let mut q = world.write_resource::<crate::comp::PendingMoveQueue>();
            std::mem::take(&mut q.requests)
        };
        if drained.is_empty() {
            return;
        }
        // 單人 TD：第一個具有玩家派系的英雄實體。
        let hero_entity: Option<Entity> = {
            let entities = world.entities();
            let heroes = world.read_storage::<Hero>();
            let factions = world.read_storage::<Faction>();
            (&entities, &heroes, &factions)
                .join()
                .find(|(_, _, f)| f.faction_id == FactionType::Player)
                .map(|(e, _, _)| e)
        };
        let Some(hero) = hero_entity else {
            log::warn!(
                "MoveTo: no Player-faction hero found ({} requests dropped)",
                drained.len()
            );
            return;
        };
        let mut move_targets = world.write_storage::<MoveTarget>();
        for req in drained {
            log::info!(
                "MoveTo pid={} → hero={:?} pos=({:.1},{:.1})",
                req.owner_pid,
                hero,
                req.pos.x.to_f32_for_render(),
                req.pos.y.to_f32_for_render(),
            );
            let _ = move_targets.insert(hero, MoveTarget(req.pos));
        }
    }

    fn handle_creep_stop(
        ecs: &mut World,
        mqtx: &crossbeam_channel::Sender<OutboundMsg>,
        source: Entity,
        target: Entity,
    ) -> Result<(), Error> {
        let mut creeps = ecs.write_storage::<Creep>();
        let c = creeps
            .get_mut(target)
            .ok_or_else(|| failure::err_msg("Creep not found"))?;
        c.block_tower = Some(source);
        c.status = CreepStatus::Stop;

        let positions = ecs.read_storage::<Pos>();
        let pos = positions
            .get(target)
            .ok_or_else(|| failure::err_msg("Creep position not found"))?;

        // 階段 5.2：遺留 0x02 GameEvent 廣播剪輯。鎖步刻度批次處理
        // (0x10) 攜帶權威狀態；客戶端從 sim 渲染。
        let (_px, _py) = pos.xy_f32();
        let _ = (mqtx, target);
        Ok(())
    }

    fn handle_creep_walk(ecs: &mut World, target: Entity) -> Result<(), Error> {
        let mut creeps = ecs.write_storage::<Creep>();
        let creep = creeps
            .get_mut(target)
            .ok_or_else(|| failure::err_msg("Creep not found"))?;
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
        let dmg_multiplier = if raw_mul < Fixed64::ZERO {
            Fixed64::ZERO
        } else {
            raw_mul
        };

        {
            let mut properties = ecs.write_storage::<CProperty>();
            if let Some(target_props) = properties.get_mut(target) {
                let hp_before = target_props.hp;
                let total_damage = (phys + magi + real) * dmg_multiplier;
                target_props.hp = target_props.hp - total_damage;
                hp_after = target_props.hp;
                max_hp = target_props.mhp;

                let (source_name, target_name) = Self::get_entity_names(ecs, source, target);

                // 注意：log 使用 f32 邊界 — Fix64 沒有顯示。
                let damage_parts = {
                    let mut parts = Vec::new();
                    if phys > Fixed64::ZERO {
                        parts.push(format!("Phys {:.1}", phys.to_f32_for_render()));
                    }
                    if magi > Fixed64::ZERO {
                        parts.push(format!("Magi {:.1}", magi.to_f32_for_render()));
                    }
                    if real > Fixed64::ZERO {
                        parts.push(format!("Pure {:.1}", real.to_f32_for_render()));
                    }
                    if parts.is_empty() {
                        parts.push(format!("Total {:.1}", total_damage.to_f32_for_render()));
                    }
                    parts.join(", ")
                };

                log::debug!(
                    "⚔️ {} 攻擊 {} | {} damage | HP: {:.1} → {:.1}/{:.1}",
                    source_name,
                    target_name,
                    damage_parts,
                    hp_before.to_f32_for_render(),
                    hp_after.to_f32_for_render(),
                    target_props.mhp.to_f32_for_render()
                );

                if target_props.hp <= Fixed64::ZERO {
                    target_props.hp = Fixed64::ZERO;
                    hp_after = Fixed64::ZERO;
                    died = true;
                    // [DEBUG-STRESS] 死亡關鍵診斷：印 max_hp / hp_before / total_damage / source
                    // 篩 mhp > 100 跳過 1HP 塔本身的死亡（目前只關心 creep 怎麼死）
                    if max_hp > Fixed64::from_i32(100) {
                        log::info!(
                            "💀 {} died | max_hp={} hp_before={} dmg={:.1} (×{:.2}) source={}",
                            target_name,
                            max_hp.to_f32_for_render(),
                            hp_before.to_f32_for_render(),
                            total_damage.to_f32_for_render(),
                            dmg_multiplier.to_f32_for_render(),
                            source_name
                        );
                    }
                }
            }
        }

        if died {
            next_outcomes.push(Outcome::Death {
                pos: pos,
                ent: target,
            });
        }

        Ok(())
    }

    fn handle_heal(
        ecs: &mut World,
        target: Entity,
        amount: omoba_sim::Fixed64,
    ) -> Result<(), Error> {
        let mut properties = ecs.write_storage::<CProperty>();
        if let Some(target_props) = properties.get_mut(target) {
            let summed = target_props.hp + amount;
            target_props.hp = if summed > target_props.mhp {
                target_props.mhp
            } else {
                summed
            };
        }
        Ok(())
    }

    fn handle_attack_update(
        ecs: &mut World,
        target: Entity,
        asd_count: Option<omoba_sim::Fixed64>,
        cooldown_reset: bool,
    ) -> Result<(), Error> {
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
                log::info!(
                    "Hero '{}' gained {} experience and leveled up!",
                    hero.name,
                    amount
                );
            } else {
                log::info!("Hero '{}' gained {} experience", hero.name, amount);
            }
        }
        Ok(())
    }

    fn handle_gold_gain(ecs: &mut World, target: Entity, amount: i32) -> Result<(), Error> {
        if amount == 0 {
            return Ok(());
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use omoba_core::ability_meta::{AbilityDef, AbilityType, CastType, TargetType};
    use specs::{Builder, World, WorldExt};

    fn ability_def(id: &str, max_level: u8) -> AbilityDef {
        AbilityDef {
            id: id.to_string(),
            name: id.to_string(),
            description: String::new(),
            ability_type: AbilityType::Active,
            target_type: TargetType::None,
            cast_type: CastType::Instant,
            icon: None,
            max_level,
            levels: HashMap::new(),
            effects_preview: Vec::new(),
            conditions: Vec::new(),
            properties: HashMap::new(),
        }
    }

    fn world_with_hero(
        skill_points: i32,
        ability_level: i32,
        max_level: u8,
    ) -> (World, Entity, String) {
        let mut world = World::new();
        world.register::<Hero>();
        world.register::<Faction>();

        let ability_id = "test_ability".to_string();
        let mut registry = crate::ability_runtime::AbilityRegistry::new();
        registry.register(ability_def(&ability_id, max_level));
        world.insert(registry);
        world.insert(crate::scripting::ScriptEventQueue::default());
        world.insert(crate::comp::PendingAbilityUpgradeQueue::default());
        world.insert(crate::comp::PendingAbilityCastQueue::default());

        let mut hero = Hero::new(
            "hero_test".to_string(),
            "Hero Test".to_string(),
            "Tester".to_string(),
        );
        hero.abilities = vec![ability_id.clone()];
        hero.ability_levels
            .insert(ability_id.clone(), ability_level);
        hero.skill_points = skill_points;

        let entity = world
            .create_entity()
            .with(hero)
            .with(Faction {
                faction_id: FactionType::Player,
                team_id: 1,
            })
            .build();
        (world, entity, ability_id)
    }

    #[test]
    fn ability_upgrade_spends_point_and_queues_skill_learn() {
        let (mut world, hero_entity, ability_id) = world_with_hero(2, 0, 2);

        GameProcessor::handle_ability_upgrade_from_input(&mut world, 0, 1).unwrap();

        {
            let heroes = world.read_storage::<Hero>();
            let hero = heroes.get(hero_entity).unwrap();
            assert_eq!(hero.skill_points, 1);
            assert_eq!(hero.ability_levels.get(&ability_id).copied(), Some(1));
        }

        let events = world
            .write_resource::<crate::scripting::ScriptEventQueue>()
            .drain();
        assert_eq!(events.len(), 1);
        match &events[0] {
            crate::scripting::ScriptEvent::SkillLearn {
                caster,
                skill_id,
                new_level,
            } => {
                assert_eq!(*caster, hero_entity);
                assert_eq!(skill_id, &ability_id);
                assert_eq!(*new_level, 1);
            }
            other => panic!("unexpected event: {:?}", other),
        }
    }

    #[test]
    fn ability_upgrade_rejects_without_skill_points() {
        let (mut world, hero_entity, ability_id) = world_with_hero(0, 0, 2);

        let err = GameProcessor::handle_ability_upgrade_from_input(&mut world, 0, 1)
            .expect_err("upgrade should reject without skill points");
        assert!(format!("{}", err).contains("no skill points"));

        let heroes = world.read_storage::<Hero>();
        let hero = heroes.get(hero_entity).unwrap();
        assert_eq!(hero.skill_points, 0);
        assert_eq!(hero.ability_levels.get(&ability_id).copied(), Some(0));
        assert!(world
            .read_resource::<crate::scripting::ScriptEventQueue>()
            .is_empty());
    }

    #[test]
    fn ability_upgrade_rejects_at_max_level() {
        let (mut world, hero_entity, ability_id) = world_with_hero(1, 2, 2);

        let err = GameProcessor::handle_ability_upgrade_from_input(&mut world, 0, 1)
            .expect_err("upgrade should reject at max level");
        assert!(format!("{}", err).contains("already maxed"));

        let heroes = world.read_storage::<Hero>();
        let hero = heroes.get(hero_entity).unwrap();
        assert_eq!(hero.skill_points, 1);
        assert_eq!(hero.ability_levels.get(&ability_id).copied(), Some(2));
        assert!(world
            .read_resource::<crate::scripting::ScriptEventQueue>()
            .is_empty());
    }

    #[test]
    fn ability_cast_drain_queues_skill_cast_for_learned_ability() {
        let (mut world, hero_entity, ability_id) = world_with_hero(0, 1, 2);
        {
            let mut q = world.write_resource::<crate::comp::PendingAbilityCastQueue>();
            q.requests.push(crate::comp::PendingAbilityCast {
                ability_index: 0,
                target_pos: None,
                target_entity: None,
                owner_pid: 1,
            });
        }

        GameProcessor::drain_pending_ability_casts(&mut world);

        let events = world
            .write_resource::<crate::scripting::ScriptEventQueue>()
            .drain();
        assert_eq!(events.len(), 1);
        match &events[0] {
            crate::scripting::ScriptEvent::SkillCast {
                caster,
                skill_id,
                target,
            } => {
                assert_eq!(*caster, hero_entity);
                assert_eq!(skill_id, &ability_id);
                assert!(matches!(target, crate::scripting::event::SkillTarget::None));
            }
            other => panic!("unexpected event: {:?}", other),
        }
    }

    #[test]
    fn ability_cast_rejects_unlearned_ability() {
        let (mut world, _hero_entity, _ability_id) = world_with_hero(0, 0, 2);

        let err = GameProcessor::handle_ability_cast_from_input(&mut world, 0, None, None, 1)
            .expect_err("cast should reject unlearned ability");
        assert!(format!("{}", err).contains("not learned"));
        assert!(world
            .read_resource::<crate::scripting::ScriptEventQueue>()
            .is_empty());
    }
}
