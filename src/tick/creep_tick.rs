use crate::comp::phys::MAX_COLLISION_RADIUS;
use crate::comp::*;
use crate::transport::OutboundMsg;
use crossbeam_channel::Sender;
use omoba_sim::trig::{angle_rotate_toward, atan2 as sim_atan2, fixed_rad_to_ticks, TAU_TICKS};
use omoba_sim::{Angle, Fixed64, Vec2 as SimVec2};
use rayon::iter::IntoParallelRefIterator;
use serde_json::json;
use specs::prelude::ParallelIterator;
use specs::{
    shred, Entities, Join, LazyUpdate, ParJoin, Read, ReadExpect, ReadStorage, SystemData, World,
    Write, WriteStorage,
};
use std::ops::Sub;
use std::{collections::BTreeMap, ops::Deref, thread};

/// MOBA 鏡頭下肉眼無感的 facing 變化量（~15°）。舊值 0.05 (~3°) 造成過多 F event。
const FACING_BROADCAST_THRESHOLD_RAD: f32 = 0.26;

#[derive(SystemData)]
pub struct CreepRead<'a> {
    entities: Entities<'a>,
    time: Read<'a, Time>,
    dt: Read<'a, DeltaTime>,
    /// P4：伺服器滴答計數器；在客戶端的 cree.M 中用作 `start_tick`
    /// 外推錨。
    tick: Read<'a, Tick>,
    paths: Read<'a, BTreeMap<String, Path>>,
    check_points: Read<'a, BTreeMap<String, CheckPoint>>,
    cpropertys: ReadStorage<'a, CProperty>,
    turn_speeds: ReadStorage<'a, TurnSpeed>,
    radii: ReadStorage<'a, CollisionRadius>,
    searcher: Read<'a, Searcher>,
    buff_store: Read<'a, crate::ability_runtime::BuffStore>,
    is_buildings: ReadStorage<'a, IsBuilding>,
}

#[derive(SystemData)]
pub struct CreepWrite<'a> {
    creeps: WriteStorage<'a, Creep>,
    pos: WriteStorage<'a, Pos>,
    facings: WriteStorage<'a, Facing>,
    facing_bcs: WriteStorage<'a, FacingBroadcast>,
    /// P4：用於 M 發射選通的每個 Creep 最後廣播快照。
    /// 在第一次發射時延遲插入（對於 Creep 來說組件可能不存在
    /// 在 P4 升級路徑之前就存在）。
    mv_broadcasts: WriteStorage<'a, CreepMoveBroadcast>,
    outcomes: Write<'a, Vec<Outcome>>,
    taken_damages: Write<'a, Vec<TakenDamage>>,
    mqtx: Write<'a, Vec<Sender<OutboundMsg>>>,
}

#[derive(Default)]
pub struct Sys;

impl<'a> System<'a> for Sys {
    type SystemData = (CreepRead<'a>, CreepWrite<'a>);

    const NAME: &'static str = "creep";

    fn run(_job: &mut Job<Self>, (tr, mut tw): Self::SystemData) {
        let time = tr.time.0;
        let dt = tr.dt.0;
        // CProperty.msd 的 dt 的舊版 f32 視圖（仍然是 f32；第 1c 階段）。
        let dt_f = dt.to_f32_for_render();
        let server_tick = tr.tick.0;
        // omfx sim_runner 不連接傳輸；回退到接收器發送器
        // 因此靜默廣播站點無操作（try_send 返回斷開連接，被忽略）。
        let tx = tw.mqtx.get(0).cloned().unwrap_or_else(|| {
            let (tx, _rx) = crossbeam_channel::unbounded::<OutboundMsg>();
            tx
        });

        // P4 發出從 par_join 通道收集的候選者，由實體鍵入。
        // 承載電流（目標、速度、起始位置、朝向）- 閘控 +
        // 記錄更新在下面連續發生，因此我們可以觸摸 mv_broadcasts
        // 無需在並行閉包內對抗借用規則。
        // 採用固定 64 有效負載 — 在第 2 階段 KCP 標籤返工中重新設計。
        struct MoveCandidate {
            entity: specs::Entity,
            target: vek::Vec2<f32>,
            velocity: f32,
            start_pos: vek::Vec2<f32>,
            facing: f32,
        }

        let (mut outcomes, move_candidates) = (
            &tr.entities,
            &mut tw.creeps,
            &mut tw.pos,
            &tr.cpropertys,
            &mut tw.facings,
            &mut tw.facing_bcs,
        )
            .par_join()
            .filter(|(_e, _creep, _p, _cp, _f, _fb)| true)
            .map_init(
                || {
                    prof_span!(guard, "creep update rayon job");
                    guard
                },
                |_guard, (e, creep, pos, cp, facing, facing_bc)| {
                    let mut outcomes: Vec<Outcome> = Vec::new();
                    let mut cands: Vec<MoveCandidate> = Vec::new();
                    // 內聯邊界助手 - 必須使用明確的 `&*pos` / 來調用
                    // `&*faceing` 以避免捕獲儲存引用作為借用
                    // 會阻止 pos.0/faceing.0 的後續突變。
                    #[inline(always)]
                    fn p_to_f(p: SimVec2) -> vek::Vec2<f32> {
                        vek::Vec2::new(p.x.to_f32_for_render(), p.y.to_f32_for_render())
                    }
                    #[inline(always)]
                    fn a_to_rad(a: Angle) -> f32 {
                        (a.ticks() as f32 / TAU_TICKS as f32) * std::f32::consts::TAU
                    }

                    if cp.hp <= Fixed64::ZERO {
                        // [DEBUG-STRESS] creep_tick 看到的 hp 值（應該與 handle_damage 寫入後的 hp 一致）
                        log::info!(
                            "☠️ creep_tick sees hp<=0: name={} hp={:.1} mhp={:.1} ent={}",
                            creep.name,
                            cp.hp.to_f32_for_render(),
                            cp.mhp.to_f32_for_render(),
                            e.id()
                        );
                        outcomes.push(Outcome::Death {
                            pos: pos.0,
                            ent: e.clone(),
                        });
                    } else {
                        if let Some(path) = tr.paths.get(&creep.path) {
                            if let Some(_b) = creep.block_tower {
                                // 被檔住了
                            } else if creep.pidx >= path.check_points.len() {
                                // TD 模式：走到 path 終點 → 漏怪事件（GameProcessor 扣 PlayerLives）
                                // 設定 status=Leaked 避免下個 tick 重複觸發
                                if !matches!(creep.status, CreepStatus::Leaked) {
                                    outcomes.push(Outcome::CreepLeaked { ent: e.clone() });
                                    creep.status = CreepStatus::Leaked;
                                }
                            } else {
                                if let Some(p) = path.check_points.get(creep.pidx) {
                                    // CheckPoint.pos 仍然是 vek::Vec2<f32> （第 1c 階段將遷移
                                    // 路徑資料到Fixed64）。每次迭代橋接一次。
                                    let target_point_f: vek::Vec2<f32> = p.pos;
                                    let target_point: SimVec2 = SimVec2::new(
                                        Fixed64::from_raw(
                                            (target_point_f.x * omoba_sim::fixed::SCALE as f32)
                                                as i64,
                                        ),
                                        Fixed64::from_raw(
                                            (target_point_f.y * omoba_sim::fixed::SCALE as f32)
                                                as i64,
                                        ),
                                    );
                                    let mut next_status = creep.status.clone();
                                    // P4：每個週期計算一次有效移動速度 - 分享
                                    // 在移動步驟和 M 發射候選之間。
                                    let stats = crate::ability_runtime::UnitStats::from_refs(
                                        &*tr.buff_store,
                                        tr.is_buildings.get(e).is_some(),
                                    );
                                    let effective_msd = stats.final_move_speed(cp.msd, e);
                                    match creep.status {
                                        CreepStatus::PreWalk => {
                                            // 首先在spawn / PreWalk → 無條件候選時發出。
                                            cands.push(MoveCandidate {
                                                entity: e,
                                                target: target_point_f,
                                                velocity: effective_msd.to_f32_for_render(),
                                                start_pos: p_to_f(pos.0),
                                                facing: a_to_rad(facing.0),
                                            });
                                            next_status = CreepStatus::Walk;
                                        }
                                        CreepStatus::Walk => {
                                            // Root / stun：本 tick 完全不前進（閉包提早返回 → 此 creep 本 tick 無 outcomes）
                                            if tr.buff_store.is_rooted(e) {
                                                return (outcomes, cands);
                                            }
                                            // 階段 1c.3： effective_msd 為固定 64（UnitStats 已遷移）。
                                            // 步驟 = effective_msd × dt (Fixed64 × Fix64)。
                                            let step = effective_msd * dt;
                                            let diff = target_point - pos.0;
                                            let dist_sq = diff.length_squared();
                                            // 0.01 在固定 64 原始 = 輪(0.01 * 1024) = 10
                                            let arrived_eps_sq = Fixed64::from_raw(10);
                                            if dist_sq < arrived_eps_sq {
                                                // 已抵達 waypoint — pidx advances, new waypoint
                                                // 觸發 M 候選（目標變更）。
                                                creep.pidx += 1;
                                                if let Some(t) = path.check_points.get(creep.pidx) {
                                                    cands.push(MoveCandidate {
                                                        entity: e,
                                                        target: t.pos,
                                                        velocity: effective_msd.to_f32_for_render(),
                                                        start_pos: p_to_f(pos.0),
                                                        facing: a_to_rad(facing.0),
                                                    });
                                                }
                                            } else {
                                                // 先轉向目標
                                                let desired_angle: Angle =
                                                    sim_atan2(diff.y, diff.x);
                                                let turn_rate = tr
                                                    .turn_speeds
                                                    .get(e)
                                                    .map(|t| t.0)
                                                    .unwrap_or(Fixed64::from_raw(1608)); // π/2 rad/s default
                                                let max_step_ticks =
                                                    fixed_rad_to_ticks(turn_rate * dt);
                                                facing.0 = angle_rotate_toward(
                                                    facing.0,
                                                    desired_angle,
                                                    max_step_ticks,
                                                );
                                                let new_facing_rad = a_to_rad(facing.0);
                                                // 廣播 facing 變化：和「上次廣播」差 > 15° 才送。
                                                let needs_emit = match facing_bc.0 {
                                                    None => true,
                                                    Some(last) => {
                                                        (new_facing_rad - last).abs()
                                                            > FACING_BROADCAST_THRESHOLD_RAD
                                                    }
                                                };
                                                if needs_emit {
                                                    facing_bc.0 = Some(new_facing_rad);
                                                }

                                                // 角度對齊（<30°）才移動 — Angle ticks comparison.
                                                let diff_ticks = (desired_angle.ticks()
                                                    - facing.0.ticks())
                                                .rem_euclid(TAU_TICKS);
                                                let signed_diff_ticks =
                                                    if diff_ticks > TAU_TICKS / 2 {
                                                        diff_ticks - TAU_TICKS
                                                    } else {
                                                        diff_ticks
                                                    };
                                                if signed_diff_ticks.abs()
                                                    < MOVE_ANGLE_THRESHOLD_TICKS
                                                {
                                                    let radius = tr
                                                        .radii
                                                        .get(e)
                                                        .map(|r| r.0)
                                                        .unwrap_or(Fixed64::from_i32(20));
                                                    let self_entity = e;
                                                    // 注意：搜尋器在內部使用 f32 來實作 instant_distance lib 相容性。
                                                    // 呼叫者的最終距離檢查是固定64。
                                                    let radius_f = radius.to_f32_for_render();
                                                    let hits = |p_sim: SimVec2| -> bool {
                                                        let q_r = radius_f + MAX_COLLISION_RADIUS;
                                                        let p_vek = vek::Vec2::new(
                                                            p_sim.x.to_f32_for_render(),
                                                            p_sim.y.to_f32_for_render(),
                                                        );
                                                        for di in tr
                                                            .searcher
                                                            .search_collidable(p_vek, q_r, 16)
                                                        {
                                                            if di.e == self_entity {
                                                                continue;
                                                            }
                                                            let Some(other_r) =
                                                                tr.radii.get(di.e).map(|cr| cr.0)
                                                            else {
                                                                continue;
                                                            };
                                                            let touch = radius + other_r;
                                                            let touch_f = touch.to_f32_for_render();
                                                            if di.dis < touch_f * touch_f {
                                                                return true;
                                                            }
                                                        }
                                                        false
                                                    };
                                                    // 記錄本 tick 是否因為碰撞而停住
                                                    let mut blocked = false;
                                                    if dist_sq > step * step {
                                                        let v = diff.normalized() * step;
                                                        let full = pos.0 + v;
                                                        if !hits(full) {
                                                            pos.0 = full;
                                                        } else {
                                                            let only_x = SimVec2::new(
                                                                pos.0.x + v.x,
                                                                pos.0.y,
                                                            );
                                                            let only_y = SimVec2::new(
                                                                pos.0.x,
                                                                pos.0.y + v.y,
                                                            );
                                                            if !hits(only_x) {
                                                                pos.0 = only_x;
                                                            } else if !hits(only_y) {
                                                                pos.0 = only_y;
                                                            } else {
                                                                blocked = true;
                                                            }
                                                        }
                                                    } else {
                                                        if !hits(target_point) {
                                                            pos.0 = target_point;
                                                            creep.pidx += 1;
                                                            // 到達中途航路點：前進並
                                                            // 為下一個航路點發出 M（目標變更）。
                                                            if let Some(t) =
                                                                path.check_points.get(creep.pidx)
                                                            {
                                                                cands.push(MoveCandidate {
                                                                    entity: e,
                                                                    target: t.pos,
                                                                    velocity: effective_msd
                                                                        .to_f32_for_render(),
                                                                    start_pos: p_to_f(pos.0),
                                                                    facing: a_to_rad(facing.0),
                                                                });
                                                            }
                                                        } else {
                                                            blocked = true;
                                                        }
                                                    }
                                                    if blocked {
                                                        // 凍結前端 lerp（action="stall"），避免視覺上穿過其他單位。
                                                    } else {
                                                        // 不是一個航點前進，也沒有被阻擋──但是
                                                        // 如果速度改變仍然考慮發射
                                                        // （緩慢應用/刪除）。下面的門通行證
                                                        // 與上次廣播相比，如果相同則丟棄。
                                                        cands.push(MoveCandidate {
                                                            entity: e,
                                                            target: target_point_f,
                                                            velocity: effective_msd
                                                                .to_f32_for_render(),
                                                            start_pos: p_to_f(pos.0),
                                                            facing: a_to_rad(facing.0),
                                                        });
                                                    }
                                                }
                                                // 角度太大：只轉向、本 tick 不位移
                                            }
                                        }
                                        CreepStatus::Stop => {
                                            next_status = CreepStatus::PreWalk;
                                        }
                                        CreepStatus::Leaked => {
                                            // 不該發生（Leaked 狀態在外層已被 pidx>=len 分支攔下）
                                        }
                                    }
                                    creep.status = next_status;
                                } else {
                                    // creep 到終點了
                                    outcomes.push(Outcome::Death { pos: pos.0, ent: e });
                                }
                            }
                        }
                    }
                    (outcomes, cands)
                },
            )
            .fold(
                || (Vec::new(), Vec::<MoveCandidate>::new()),
                |(mut all_outcomes, mut all_cands), (mut outcomes, mut cands)| {
                    all_outcomes.append(&mut outcomes);
                    all_cands.append(&mut cands);
                    (all_outcomes, all_cands)
                },
            )
            .reduce(
                || (Vec::new(), Vec::<MoveCandidate>::new()),
                |(mut outcomes_a, mut cands_a), (mut outcomes_b, mut cands_b)| {
                    outcomes_a.append(&mut outcomes_b);
                    cands_a.append(&mut cands_b);
                    (outcomes_a, cands_a)
                },
            );

        // P4 串行發射閘通：將每個候選者與
        // 實體的最後廣播快照（CreepMoveBroadcast 元件）。
        // 僅當目標偏離或速度變化 > 5% / 時才發出蠕變。
        // > 1.0 絕對值或實體沒有先前的快照。更新
        // 發出後的組件，因此下一個刻度的比較使用新的基線。
        for cand in move_candidates.into_iter() {
            let need_emit = match tw.mv_broadcasts.get(cand.entity) {
                Some(bcast) => bcast.should_emit(cand.target, cand.velocity),
                None => true, // first-ever candidate for this entity
            };
            if !need_emit {
                continue;
            }

            // 階段 5.2：遺留 0x02 GameEvent 製作人刪減。鎖步刻度批次處理
            // (0x10)攜帶權威pos；客戶端從 sim 渲染。

            // 更新（或插入）廣播快照以便後續刻度
            // 與新基線進行比較。規格::寫入儲存::插入
            // 僅在無效實體上傳回 Err — 可以安全地忽略。
            let mut snap = tw
                .mv_broadcasts
                .get(cand.entity)
                .cloned()
                .unwrap_or_default();
            snap.record(cand.target, cand.velocity, server_tick);
            let _ = tw.mv_broadcasts.insert(cand.entity, snap);
        }

        tw.outcomes.append(&mut outcomes);
        // 傷害計算 - 改為生成 Damage 事件
        for td in tw.taken_damages.iter() {
            if let Some(cp) = tr.cpropertys.get(td.ent) {
                // 記錄攻擊前狀態
                let hp_before = cp.hp;
                let max_hp = cp.mhp;

                // Phase 1c.4: cp.* / td.* / Outcome::Damage.{phys,magi,real} 全 Fixed64。
                let phys_raw = td.phys - cp.def_physic;
                let phys_damage: Fixed64 = if phys_raw < Fixed64::ZERO {
                    Fixed64::ZERO
                } else {
                    phys_raw
                };
                let magi_raw = td.magi - cp.def_magic;
                let magi_damage: Fixed64 = if magi_raw < Fixed64::ZERO {
                    Fixed64::ZERO
                } else {
                    magi_raw
                };
                let total_damage: Fixed64 = phys_damage + magi_damage;

                // 獲取目標名稱用於日誌
                let target_name = if let Some(creep) = tw.creeps.get(td.ent) {
                    creep.name.clone()
                } else {
                    // 暫時使用實體 ID，因為沒有在 Read 結構中包含 Hero
                    format!("Entity({:?})", td.ent.id())
                };

                if total_damage > Fixed64::ZERO {
                    // 階段 1c.4：Outcome::Damage.pos 是 SimVec2（階段 1c.2）。
                    let target_pos = tw.pos.get(td.ent).map(|p| p.0).unwrap_or(SimVec2::ZERO);

                    // 生成傷害事件（日誌將在 state.rs 中統一處理）
                    tw.outcomes.push(Outcome::Damage {
                        pos: target_pos,
                        phys: phys_damage,
                        magi: magi_damage,
                        real: Fixed64::ZERO,
                        source: td.source, // 使用正確的攻擊者
                        target: td.ent,
                        predeclared: false, // melee / on-touch damage — never pre-declared
                    });
                } else if td.phys > Fixed64::ZERO || td.magi > Fixed64::ZERO {
                    // 只有在有原始傷害但被完全防禦時才顯示
                    log::info!(
                        "🛡️ {} | Damage BLOCKED: Phys {:.1} vs Def {:.1}, Magi {:.1} vs Def {:.1}",
                        target_name,
                        td.phys.to_f32_for_render(),
                        cp.def_physic.to_f32_for_render(),
                        td.magi.to_f32_for_render(),
                        cp.def_magic.to_f32_for_render()
                    );
                }
            }
        }
        tw.taken_damages.clear();
    }
}
