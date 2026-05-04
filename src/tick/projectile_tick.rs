use omb_script_abi::buff_ids::BuffId;
use omb_script_abi::stat_keys::StatKey;
use specs::{
    shred, Entities, Join, LazyUpdate, Read, ReadExpect, ReadStorage, SystemData,
    Write, WriteStorage, ParJoin, World,
};
use crossbeam_channel::Sender;
use crate::comp::*;
use crate::transport::OutboundMsg;
use specs::prelude::ParallelIterator;
use specs::Entity;
use omoba_sim::{Fixed64, Vec2 as SimVec2};

#[derive(SystemData)]
pub struct ProjectileRead<'a> {
    entities: Entities<'a>,
    time: Read<'a, Time>,
    dt: Read<'a, DeltaTime>,
    searcher : Read<'a, Searcher>,
    hero_attacks: ReadStorage<'a, TAttack>,
}

#[derive(SystemData)]
pub struct ProjectileWrite<'a> {
    pos : WriteStorage<'a, Pos>,
    projs : WriteStorage<'a, Projectile>,
    outcomes: Write<'a, Vec<Outcome>>,
    taken_damages: Write<'a, Vec<TakenDamage>>,
    damage_instances: Write<'a, Vec<DamageInstance>>,
    mqtx: Write<'a, Vec<Sender<OutboundMsg>>>,
}

#[derive(Default)]
pub struct Sys;

impl<'a> System<'a> for Sys {
    type SystemData = (
        ProjectileRead<'a>,
        ProjectileWrite<'a>,
    );

    const NAME: &'static str = "projectile";

    fn run(_job: &mut Job<Self>, (tr, mut tw): Self::SystemData) {
        let time = tr.time.0;
        // dt is Fixed64; arithmetic against Projectile fields stays in Fixed64.
        let dt: Fixed64 = tr.dt.0;

        // Snapshot every entity's current Pos so projectiles can home toward the
        // target's LIVE position each tick (homing). Previously `tpos` was frozen
        // at firing time, so the bullet flew to where the target used to be — it
        // visually missed a moving target even though damage was still applied
        // via the stored `target` Entity.
        let target_positions: std::collections::HashMap<specs::Entity, SimVec2> = {
            use specs::Join;
            (&tr.entities, &tw.pos).join()
                .map(|(e, pos)| (e, pos.0))
                .collect()
        };

        //log::info!("projs count {}", tw.projs.count());
        let mut outcomes = (
            &tr.entities,
            &mut tw.projs,
            &mut tw.pos,
        )
            .par_join()
            .filter(|(e, proj, p)| proj.time_left > Fixed64::ZERO)
            .map_init(
                || {
                    prof_span!(guard, "projectile update rayon job");
                    guard
                },
                |_guard, (e, proj, pos)| {
                    let mut outcomes: Vec<Outcome> = Vec::new();
                    // Home onto target's current position if still alive；
                    // target 消失時用 stale tpos，靠 time_left 安全閥讓彈道自然消失。
                    if let Some(target) = proj.target {
                        if let Some(&current_tpos) = target_positions.get(&target) {
                            proj.tpos = current_tpos;
                        }
                    }
                    let delta = proj.tpos - pos.0;
                    let dist = delta.length();
                    let step = proj.msd * dt;

                    // 無 target 的方向性子彈（Tack 放射針）：用掃掠 segment 檢查命中
                    // （不只檢查當前 point，還要檢查本 tick 即將走過的路徑）避免高速子彈
                    // 跨過氣球之間的間隔而沒打中。
                    let needle_r = if proj.hit_radius > Fixed64::ZERO {
                        proj.hit_radius
                    } else {
                        Fixed64::from_i32(50)
                    };
                    if proj.target.is_none() && proj.radius < Fixed64::ONE {
                        // 計算本 tick 的 swept segment：從 pos.0 出發，沿 delta 方向走 step 距離
                        let a: SimVec2 = pos.0;
                        let b: SimVec2 = if dist > Fixed64::ZERO {
                            a + delta.normalized() * step
                        } else { a };
                        // NOTE: mid + half_len computed in f32 only at the search-call boundary
                        // (Searcher uses f32 internally for instant_distance lib compat; final distance check in caller is Fixed64).
                        let a_xf = a.x.to_f32_for_render();
                        let a_yf = a.y.to_f32_for_render();
                        let b_xf = b.x.to_f32_for_render();
                        let b_yf = b.y.to_f32_for_render();
                        let seg_mid_f = vek::Vec2::new((a_xf + b_xf) * 0.5, (a_yf + b_yf) * 0.5);
                        let half_len_f = (vek::Vec2::new(b_xf - a_xf, b_yf - a_yf)).magnitude() * 0.5;
                        let needle_r_f = needle_r.to_f32_for_render();
                        let search_r = half_len_f + needle_r_f + 5.0;
                        let candidates = tr.searcher.creep.search_nn(seg_mid_f, search_r, 16);
                        let needle_r2 = needle_r_f * needle_r_f;
                        let mut hit: Option<specs::Entity> = None;
                        let a_vek = vek::Vec2::new(a_xf, a_yf);
                        let b_vek = vek::Vec2::new(b_xf, b_yf);
                        for ci in candidates.iter() {
                            let cpos_sim = target_positions.get(&ci.e).copied().unwrap_or(SimVec2::ZERO);
                            let cpos_vek = vek::Vec2::new(
                                cpos_sim.x.to_f32_for_render(),
                                cpos_sim.y.to_f32_for_render(),
                            );
                            if crate::util::geometry::point_segment_dist_sq(cpos_vek, a_vek, b_vek) <= needle_r2 {
                                hit = Some(ci.e);
                                break;
                            }
                        }
                        if let Some(hit_ent) = hit {
                            // 命中點：取子彈與氣球最接近那一點（pos.0 or b 取近的）
                            let c_sim = target_positions.get(&hit_ent).copied().unwrap_or(a);
                            let da = (c_sim - a).length_squared();
                            let db = (c_sim - b).length_squared();
                            let hit_pos: SimVec2 = if da <= db { a } else { b };
                            create_projectile_damage(&proj, hit_ent, &mut outcomes, hit_pos);
                            outcomes.push(Outcome::Death { pos: hit_pos, ent: e.clone() });
                            return outcomes;
                        }
                    }

                    // 命中判定：本 tick 的移動量已足夠抵達目標 → 直接 hit
                    let reached = dist <= step || dist < Fixed64::ONE;
                    if reached {
                        // 命中點：優先用 target 的最新位置（snapshot = 本 tick 初的 Pos storage），
                        // 這樣 AoE 圓心和爆炸特效一定落在氣球身上，不會停在子彈剛發射時那一刻。
                        let hit_pos: SimVec2 = if let Some(target) = proj.target {
                            target_positions.get(&target).copied().unwrap_or(proj.tpos)
                        } else {
                            proj.tpos
                        };
                        pos.0 = hit_pos;
                        if proj.radius > Fixed64::ONE {
                            // 範圍攻擊：以 hit_pos 為中心掃半徑內敵人。
                            // NOTE: Searcher uses f32 internally for instant_distance lib compat; final distance check in caller is Fixed64.
                            let hit_pos_vek = vek::Vec2::new(
                                hit_pos.x.to_f32_for_render(),
                                hit_pos.y.to_f32_for_render(),
                            );
                            let radius_f = proj.radius.to_f32_for_render();
                            let targets = tr.searcher.creep.search_nn(hit_pos_vek, radius_f, 5);
                            for target_info in targets.iter() {
                                create_projectile_damage(&proj, target_info.e, &mut outcomes, hit_pos);
                            }
                            // Phase 4.2: 把爆炸 VFX 走 Outcome::Explosion → ExplosionFxQueue
                            // → snapshot → omfx ring render lifecycle。原註解寫「前端
                            // 自己在子彈飛完時 spawn」，但 Phase 1.4 砍了 projectile_create
                            // wire emit 後前端不再收到那個訊息，VFX 整個漏了。
                            // 0.5s duration 跟 legacy make_game_explosion 保持一致。
                            outcomes.push(Outcome::Explosion {
                                pos: hit_pos,
                                radius: proj.radius,
                                duration: Fixed64::from_raw(512), // 0.5 s (raw 512 / SCALE 1024)
                            });
                        } else if let Some(target) = proj.target {
                            // 單體攻擊
                            create_projectile_damage(&proj, target, &mut outcomes, hit_pos);
                        }
                        // 方向性子彈：抵達 end_pos 但沒打到任何敵人 → 直接消失
                        outcomes.push(Outcome::Death { pos: hit_pos, ent: e.clone() });
                    } else {
                        // 還沒抵達：往目標方向前進一個 step
                        let vel = (delta.normalized()) * step;
                        let new_pos = pos.0 + vel;
                        pos.0 = new_pos;
                        // 安全閥：time_left 到期仍未命中（例如 target 死掉 tpos 凍結），讓 projectile 自然消失
                        proj.time_left = proj.time_left - dt;
                        if proj.time_left <= Fixed64::ZERO {
                            outcomes.push(Outcome::Death { pos: new_pos, ent: e.clone() });
                        }
                    }
                    outcomes
                },
            )
            .fold(
                || Vec::new(),
                |mut all_outcomes, mut outcomes| {
                    all_outcomes.append(&mut outcomes);
                    all_outcomes
                },
            )
            .reduce(
                || Vec::new(),
                |mut outcomes_a, mut outcomes_b| {
                    outcomes_a.append(&mut outcomes_b);
                    outcomes_a
                },
            );
        tw.outcomes.append(&mut outcomes);

        // 前端已自管子彈動畫（收 C 時拿 target_id + flight_time_ms 後本地 pursuit lerp），
        // 不再廣播 projectile 每 tick 位置。
    }
}

/// 創建投射物傷害事件 - 使用新的傷害事件系統。
/// 若 projectile 帶有 slow_factor/slow_duration（Ice 塔）則同時 push `AddBuff`：
/// Slow buff 採單一 instance 設計：buff_id = "slow"。同 creep 上多次命中：
///   - duration 取 max（refresh 不疊加）
///   - payload 只在新 slow_factor 較小（更強）時覆寫，否則保留舊 payload
/// 由 BuffStore::add 的 should_replace 邏輯處理（讀 payload 內的 `slow_factor` 欄位）。
/// payload 寫 `move_speed_bonus = -(1 - factor)`（負值 = 減速）+ `slow_factor`。
///
/// P7 latency hiding: 非 AOE（`proj.radius < 1.0`）的單體追蹤彈在發射時
/// 已把 final damage 寫到 ProjectileCreate.damage 欄位，client 會在 impact
/// tick 自行 local 扣血。此 helper 把 `Outcome::Damage.predeclared` 設 true,
/// `handle_damage` 端在聚合後若仍為 true 則跳過 creep.H 廣播省 bytes。
/// AOE（radius > 1.0）仍照常發 creep.H，因為 client 無法預測哪些 creep 會被
/// 濺射到。
fn create_projectile_damage(
    proj: &Projectile,
    target: specs::Entity,
    outcomes: &mut Vec<Outcome>,
    pos: SimVec2,
) {
    log::debug!("彈道命中目標 {}，物理傷害: {:.1}，魔法傷害: {:.1}，真實傷害: {:.1}",
        target.id(),
        proj.damage_phys.to_f32_for_render(),
        proj.damage_magi.to_f32_for_render(),
        proj.damage_real.to_f32_for_render());

    // P7 layered (re-enabled with heartbeat in_flight_projectiles set):
    // single-target (radius < 1.0) with damage > 0 → predeclared = true. Server
    // skips creep/H. Client maintains pending_pred_dmg, applies on visual hit
    // (t≥1.0), and reconciles via heartbeat in_flight set when server settles.
    let predeclared = proj.radius < Fixed64::ONE && proj.damage_phys > Fixed64::ZERO;
    outcomes.push(Outcome::Damage {
        pos,
        phys: proj.damage_phys,
        magi: proj.damage_magi,
        real: proj.damage_real,
        source: proj.owner,
        target: target,
        predeclared,
    });

    // Ice 塔：附加減速 debuff 到目標
    if proj.slow_factor > Fixed64::ZERO && proj.slow_factor < Fixed64::ONE && proj.slow_duration > Fixed64::ZERO {
        // factor=0.5 → bonus=-0.5 ; bonus = -(1 - factor) = factor - 1
        let bonus = proj.slow_factor - Fixed64::ONE;
        let mut payload = serde_json::Map::new();
        payload.insert(
            StatKey::MoveSpeedBonus.as_str().to_string(),
            serde_json::json!(bonus.to_f32_for_render()),
        );
        payload.insert(
            "slow_factor".into(),
            serde_json::json!(proj.slow_factor.to_f32_for_render()),
        );
        outcomes.push(Outcome::AddBuff {
            target,
            buff_id: "slow".to_string(),
            duration: proj.slow_duration,
            payload: serde_json::Value::Object(payload),
        });
    }

    // matchlock_gun 等 on-hit stun：handle_projectile 擲骰後把時長寫在 proj 上
    if proj.stun_duration > Fixed64::ZERO {
        outcomes.push(Outcome::AddBuff {
            target,
            buff_id: BuffId::Stun.as_str().to_string(),
            duration: proj.stun_duration,
            payload: serde_json::Value::Null,
        });
    }
}
