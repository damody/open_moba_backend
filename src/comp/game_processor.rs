use failure::Error;
use omb_script_abi::stat_keys::StatKey;
use serde_json::json;
use specs::{
    storage::{ReadStorage, WriteStorage},
    Builder, Entity, World, WorldExt,
};
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
        Outcome::AttackPhaseCue { .. } => "AttackPhaseCue",
        Outcome::EntityRemoved { .. } => "EntityRemoved",
    }
}

pub struct GameProcessor;

impl GameProcessor {
    fn interrupt_attack_for_accepted_command(
        world: &mut World,
        entity: Entity,
    ) -> Option<AttackCancelPhase> {
        omoba_core::runtime::game_processor::interrupt_attack_for_accepted_command(world, entity)
    }

    pub fn process_outcomes(
        ecs: &mut World,
        mqtx: &crossbeam_channel::Sender<OutboundMsg>,
    ) -> Result<(), Error> {
        let mut sink = omoba_core::runtime::RuntimeEventVecSink::default();
        omoba_core::runtime::game_processor::process_outcomes(ecs, &mut sink)?;
        for msg in crate::runtime_events::runtime_events_to_outbound(sink.events) {
            let _ = mqtx.try_send(msg);
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
        // Phase 1.6: 不再廣播 entity.death；snapshot consumer 透過
        // SimWorldSnapshot.removed_entity_ids 偵測死亡並釋放渲染資源。
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
            let buff_store = ecs.read_resource::<omoba_core::runtime::ability_runtime::BuffStore>();
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
            let stats =
                omoba_core::runtime::ability_runtime::UnitStats::from_refs(&*buff_store, is_b);
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
        ecs.write_resource::<omoba_core::runtime::ability_runtime::BuffStore>()
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
            let mut store = ecs.write_resource::<omoba_core::runtime::ability_runtime::BuffStore>();
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
    /// 由 authoritative runtime 與 local lockstep replica 確定性地執行：
    /// queue 由相同 `TickBatch.inputs` 填入，並在同一 dispatch boundary drain。
    pub fn handle_tower_spawn_from_input(
        world: &mut World,
        kind_id: u32,
        pos: omoba_sim::Vec2,
        owner_pid: u32,
    ) -> Result<specs::Entity, Error> {
        omoba_core::runtime::game_processor::handle_tower_spawn_from_input(
            world, kind_id, pos, owner_pid,
        )
    }

    /// 階段 2.1：排空 `PendingTowerSpawnQueue` 並產生每個請求的塔。
    /// 必須在 `player_input_tick::Sys` 填完 queue 後、下一個 snapshot extraction
    /// 前呼叫。authoritative runtime 與 local lockstep replica 都在相同 tick
    /// boundary drain 這個 queue。
    pub fn drain_pending_tower_spawns(world: &mut World) {
        omoba_core::runtime::game_processor::drain_pending_tower_spawns(world);
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
    /// 由 authoritative runtime 與 local lockstep replica 確定性地執行：
    /// queue 由相同 `TickBatch.inputs` 填入，並在同一 dispatch boundary drain。
    pub fn handle_tower_sell_from_input(
        world: &mut World,
        tower_entity_id: u32,
        owner_pid: u32,
    ) -> Result<(), Error> {
        omoba_core::runtime::game_processor::handle_tower_sell_from_input(
            world,
            tower_entity_id,
            owner_pid,
        )
    }

    /// 階段 2.2：排空 `PendingTowerSellQueue` 並處理每個銷售請求。
    /// 必須在 `player_input_tick::Sys` 填完 queue 後、下一個 snapshot extraction
    /// 前呼叫。authoritative runtime 與 local lockstep replica 都在相同 tick
    /// boundary drain 這個 queue。
    pub fn drain_pending_tower_sells(world: &mut World) {
        omoba_core::runtime::game_processor::drain_pending_tower_sells(world);
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
    /// 由 authoritative runtime 與 local lockstep replica 確定性地執行：
    /// queue 由相同 `TickBatch.inputs` 填入，並在同一 dispatch boundary drain。
    pub fn handle_tower_upgrade_from_input(
        world: &mut World,
        tower_entity_id: u32,
        path: u8,
        _level_hint: u8,
        owner_pid: u32,
    ) -> Result<(), Error> {
        omoba_core::runtime::game_processor::handle_tower_upgrade_from_input(
            world,
            tower_entity_id,
            path,
            _level_hint,
            owner_pid,
        )
    }

    /// 階段 2.3：排空 `PendingTowerUpgradeQueue` 並處理每個升級。
    /// 必須在 `player_input_tick::Sys` 填完 queue 後、下一個 snapshot extraction
    /// 前呼叫。authoritative runtime 與 local lockstep replica 都在相同 tick
    /// boundary drain 這個 queue。
    pub fn drain_pending_tower_upgrades(world: &mut World) {
        omoba_core::runtime::game_processor::drain_pending_tower_upgrades(world);
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
        omoba_core::runtime::game_processor::handle_ability_upgrade_from_input(
            world,
            ability_index,
            owner_pid,
        )
    }

    /// 在 player_input_tick 之後、script dispatch 之前 drain queued ability upgrades。
    pub fn drain_pending_ability_upgrades(world: &mut World) {
        omoba_core::runtime::game_processor::drain_pending_ability_upgrades(world);
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
        omoba_core::runtime::game_processor::handle_ability_cast_from_input(
            world,
            ability_index,
            target_pos,
            target_entity,
            owner_pid,
        )
    }

    /// 在 player_input_tick 之後、script dispatch 之前 drain queued ability casts。
    pub fn drain_pending_ability_casts(world: &mut World) {
        omoba_core::runtime::game_processor::drain_pending_ability_casts(world);
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
    /// 由 authoritative runtime 與 local lockstep replica 確定性地執行：
    /// queue 由相同 `TickBatch.inputs` 填入，並在同一 dispatch boundary drain。
    pub fn handle_item_use_from_input(
        world: &mut World,
        item_slot: u32,
        _target_pos: Option<omoba_sim::Vec2>,
        _target_entity: Option<u32>,
        owner_pid: u32,
    ) -> Result<(), Error> {
        omoba_core::runtime::game_processor::handle_item_use_from_input(
            world,
            item_slot,
            _target_pos,
            _target_entity,
            owner_pid,
        )
    }

    /// 階段 2.4：排出 `PendingItemUseQueue` 並處理每個請求。
    /// 必須在 `player_input_tick::Sys` 填完 queue 後、下一個 snapshot extraction
    /// 前呼叫。authoritative runtime 與 local lockstep replica 都在相同 tick
    /// boundary drain 這個 queue。
    pub fn drain_pending_item_uses(world: &mut World) {
        omoba_core::runtime::game_processor::drain_pending_item_uses(world);
    }

    /// MoveTo (右鍵移動): drain `PendingMoveQueue` and write `MoveTarget`
    /// 玩家英雄實體上的組件。 TD模式：單人錢包，
    /// 選擇第一個「(Hero, Faction == Player)」實體。確定性：隊列是
    /// 來自相同 `TickBatch.inputs` 的 authoritative/local replica 填充相同，
    /// 在同一調度邊界耗盡，英雄查找使用相同的連接
    /// 雙方訂購。
    pub fn drain_pending_moves(world: &mut World) {
        omoba_core::runtime::game_processor::drain_pending_moves(world);
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
            let bs = ecs.read_resource::<omoba_core::runtime::ability_runtime::BuffStore>();
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
    use std::collections::{BTreeMap, HashMap};

    use omoba_core::ability_meta::{AbilityDef, AbilityType, CastType, TargetType};
    use specs::{Builder, Join, World, WorldExt};

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
        world.register::<MoveTarget>();
        world.register::<TAttack>();

        let ability_id = "test_ability".to_string();
        let mut registry = omoba_core::runtime::ability_runtime::AbilityRegistry::new();
        registry.register(ability_def(&ability_id, max_level));
        world.insert(registry);
        world.insert(crate::scripting::ScriptEventQueue::default());
        world.insert(crate::comp::PendingAbilityUpgradeQueue::default());
        world.insert(crate::comp::PendingAbilityCastQueue::default());
        world.insert(crate::comp::PendingMoveQueue::default());
        world.insert(crate::comp::AttackCancelFxQueue::default());
        world.insert(Tick(0));

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
            .with(TAttack::new(
                omoba_sim::Fixed64::from_i32(10),
                omoba_sim::Fixed64::from_i32(1),
                omoba_sim::Fixed64::from_i32(100),
                omoba_sim::Fixed64::from_i32(1000),
            ))
            .build();
        (world, entity, ability_id)
    }

    fn test_tower_template(
        unit_id: &str,
        placement_radius: f32,
        cost: i32,
    ) -> crate::comp::tower_registry::TowerTemplate {
        use crate::comp::tower_registry::{
            AttackTimingMetadata, TowerRecoil, TowerRenderAnimation, TowerRenderMetadata,
            TowerRenderPoint, TowerTemplate,
        };

        TowerTemplate {
            unit_id: unit_id.to_string(),
            label: unit_id.to_string(),
            atk: 1.0,
            asd_interval: 1.0,
            range: 100.0,
            bullet_speed: 100.0,
            splash_radius: 0.0,
            hit_radius: 0.0,
            slow_factor: 0.0,
            slow_duration: 0.0,
            cost,
            footprint: 10.0,
            placement_radius,
            hp: 1.0,
            turn_speed_deg: 360.0,
            render: TowerRenderMetadata {
                render_mode: "base_barrel".to_string(),
                base: String::new(),
                barrel: String::new(),
                visual_size: 180.0,
                barrel_frames: Vec::new(),
                body_frames: Vec::new(),
                barrel_animation: TowerRenderAnimation {
                    fps: 1.0,
                    loop_animation: true,
                    fire_fps: 1.0,
                    fire_once: true,
                },
                body_animation: TowerRenderAnimation {
                    fps: 1.0,
                    loop_animation: true,
                    fire_fps: 1.0,
                    fire_once: true,
                },
                rotation_mode: "targeted".to_string(),
                barrel_layout: "single".to_string(),
                barrel_variants: Vec::new(),
                barrel_offset: TowerRenderPoint { x: 0.0, y: 0.0 },
                barrel_pivot: TowerRenderPoint { x: 0.5, y: 0.65 },
                muzzle_offset: TowerRenderPoint { x: 0.0, y: 0.0 },
                default_angle_deg: 0.0,
                recoil: TowerRecoil {
                    mode: "directional".to_string(),
                    distance: 0.0,
                    scale: 1.0,
                    duration_ms: 1,
                    return_ms: 1,
                },
            },
            attack_timing: AttackTimingMetadata {
                windup: 500,
                backswing: 500,
            },
        }
    }

    fn world_with_tower_place_context(placement_radius: f32) -> (World, Entity) {
        let mut world = World::new();
        world.register::<Hero>();
        world.register::<Faction>();
        world.register::<Gold>();
        world.register::<Tower>();
        world.register::<Pos>();
        world.register::<crate::scripting::ScriptUnitTag>();

        let hero = world
            .create_entity()
            .with(Hero::new(
                "hero_test".to_string(),
                "Hero Test".to_string(),
                "Tester".to_string(),
            ))
            .with(Faction {
                faction_id: FactionType::Player,
                team_id: 1,
            })
            .with(Gold(1_000))
            .build();

        let mut registry = crate::comp::tower_registry::TowerTemplateRegistry::default();
        registry.insert(test_tower_template("tower_ice", placement_radius, 400));
        world.insert(registry);
        world.insert(BlockedRegions::default());
        world.insert(BTreeMap::<String, Path>::from([(
            "main".to_string(),
            Path {
                check_points: vec![
                    CheckPoint {
                        name: "a".to_string(),
                        class: String::new(),
                        pos: vek::Vec2::new(500.0, -100.0),
                    },
                    CheckPoint {
                        name: "b".to_string(),
                        class: String::new(),
                        pos: vek::Vec2::new(500.0, 100.0),
                    },
                ],
            },
        )]));

        (world, hero)
    }

    #[test]
    fn tower_place_lockstep_rejects_path_overlap_from_placement_radius() {
        let (mut world, hero_entity) = world_with_tower_place_context(900.0);
        let pos = omoba_sim::Vec2::new(omoba_sim::Fixed64::ZERO, omoba_sim::Fixed64::ZERO);

        let err = GameProcessor::handle_tower_spawn_from_input(
            &mut world,
            omoba_template_ids::TOWER_ICE.0 as u32,
            pos,
            1,
        )
        .expect_err("large placement radius should block path-overlapping tower place");

        assert!(format!("{}", err).contains("blocked by path"));
        let golds = world.read_storage::<Gold>();
        assert_eq!(golds.get(hero_entity).map(|g| g.0), Some(1_000));
        let entities = world.entities();
        let towers = world.read_storage::<Tower>();
        assert_eq!((&entities, &towers).join().count(), 0);
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

    fn set_attack_sequence(
        world: &mut World,
        entity: Entity,
        phase: AttackSequencePhase,
        asd_count: omoba_sim::Fixed64,
        seq: u32,
    ) {
        let mut attacks = world.write_storage::<TAttack>();
        let attack = attacks.get_mut(entity).expect("hero attack component");
        attack.attack_phase = phase;
        attack.asd_count = asd_count;
        attack.attack_seq = seq;
    }

    fn assert_attack_cancel(
        world: &mut World,
        phase: AttackCancelPhase,
        seq: u32,
        impact_committed: bool,
    ) {
        let mut queue = world.write_resource::<AttackCancelFxQueue>();
        assert_eq!(queue.pending.len(), 1);
        let fx = queue.pending.pop().unwrap();
        assert_eq!(fx.phase, phase);
        assert_eq!(fx.attack_seq, seq);
        assert_eq!(fx.impact_committed, impact_committed);
    }

    fn assert_attack_idle_and_ready(world: &mut World, entity: Entity) {
        let attacks = world.read_storage::<TAttack>();
        let attack = attacks.get(entity).expect("hero attack component");
        assert_eq!(attack.attack_phase, AttackSequencePhase::Idle);
        assert_eq!(attack.asd_count, attack.asd.v);
    }

    #[test]
    fn move_command_cancels_windup_before_damage() {
        let (mut world, hero_entity, _ability_id) = world_with_hero(0, 1, 2);
        set_attack_sequence(
            &mut world,
            hero_entity,
            AttackSequencePhase::Windup,
            omoba_sim::Fixed64::ZERO - omoba_sim::Fixed64::from_raw(100),
            7,
        );
        world
            .write_resource::<PendingMoveQueue>()
            .requests
            .push(PendingMoveTo {
                pos: omoba_sim::Vec2::new(
                    omoba_sim::Fixed64::from_i32(10),
                    omoba_sim::Fixed64::from_i32(20),
                ),
                owner_pid: 1,
            });

        GameProcessor::drain_pending_moves(&mut world);

        assert_attack_idle_and_ready(&mut world, hero_entity);
        assert!(world
            .read_storage::<MoveTarget>()
            .get(hero_entity)
            .is_some());
        assert_attack_cancel(&mut world, AttackCancelPhase::Windup, 7, false);
    }

    #[test]
    fn skill_command_cancels_windup_before_damage() {
        let (mut world, hero_entity, _ability_id) = world_with_hero(0, 1, 2);
        set_attack_sequence(
            &mut world,
            hero_entity,
            AttackSequencePhase::Windup,
            omoba_sim::Fixed64::ZERO - omoba_sim::Fixed64::from_raw(100),
            8,
        );

        GameProcessor::handle_ability_cast_from_input(&mut world, 0, None, None, 1).unwrap();

        assert_attack_idle_and_ready(&mut world, hero_entity);
        assert_attack_cancel(&mut world, AttackCancelPhase::Windup, 8, false);
        assert_eq!(
            world
                .read_resource::<crate::scripting::ScriptEventQueue>()
                .len(),
            1
        );
    }

    #[test]
    fn move_command_during_backswing_preserves_committed_attack() {
        let (mut world, hero_entity, _ability_id) = world_with_hero(0, 1, 2);
        set_attack_sequence(
            &mut world,
            hero_entity,
            AttackSequencePhase::Backswing,
            omoba_sim::Fixed64::from_raw(100),
            9,
        );
        world
            .write_resource::<PendingMoveQueue>()
            .requests
            .push(PendingMoveTo {
                pos: omoba_sim::Vec2::new(
                    omoba_sim::Fixed64::from_i32(30),
                    omoba_sim::Fixed64::from_i32(40),
                ),
                owner_pid: 1,
            });

        GameProcessor::drain_pending_moves(&mut world);

        assert_attack_idle_and_ready(&mut world, hero_entity);
        assert_attack_cancel(&mut world, AttackCancelPhase::Backswing, 9, true);
    }

    #[test]
    fn skill_command_during_backswing_preserves_committed_attack() {
        let (mut world, hero_entity, _ability_id) = world_with_hero(0, 1, 2);
        set_attack_sequence(
            &mut world,
            hero_entity,
            AttackSequencePhase::Backswing,
            omoba_sim::Fixed64::from_raw(100),
            10,
        );

        GameProcessor::handle_ability_cast_from_input(&mut world, 0, None, None, 1).unwrap();

        assert_attack_idle_and_ready(&mut world, hero_entity);
        assert_attack_cancel(&mut world, AttackCancelPhase::Backswing, 10, true);
        assert_eq!(
            world
                .read_resource::<crate::scripting::ScriptEventQueue>()
                .len(),
            1
        );
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
