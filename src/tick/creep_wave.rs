use rayon::iter::IntoParallelRefIterator;
use specs::{
    shred, Entities, Join, LazyUpdate, Read, ReadExpect, ReadStorage, SystemData,
    Write, WriteStorage, ParJoin, World,
};
use std::{thread, ops::Deref, collections::BTreeMap};
use crate::comp::*;
use specs::prelude::ParallelIterator;
use vek::Vec2;
use crossbeam_channel::{Receiver, Sender};
use crate::transport::OutboundMsg;
use serde_json::json;

#[derive(SystemData)]
pub struct CreepWaveRead<'a> {
    entities: Entities<'a>,
    time: Read<'a, Time>,
    dt: Read<'a, DeltaTime>,
    creep_emiters: Read<'a, BTreeMap<String, CreepEmiter>>,
    paths: Read<'a, BTreeMap<String, Path>>,
    check_points : Read<'a, BTreeMap<String, CheckPoint>>,
    creeps: ReadStorage<'a, Creep>,
    game_mode: Read<'a, GameMode>,
}

#[derive(SystemData)]
pub struct CreepWaveWrite<'a> {
    outcomes: Write<'a, Vec<Outcome>>,
    cur_creep_wave: Write<'a, CurrentCreepWave>,
    creep_waves: Write<'a, Vec<CreepWave>>,
    mqtx: Write<'a, Vec<Sender<OutboundMsg>>>,
}

#[derive(Default)]
pub struct Sys;

impl<'a> System<'a> for Sys {
    type SystemData = (
        CreepWaveRead<'a>,
        CreepWaveWrite<'a>,
    );

    const NAME: &'static str = "creep_wave";

    fn run(_job: &mut Job<Self>, (tr, mut tw): Self::SystemData) {
        let totaltime = tr.time.0;
        let is_td = tr.game_mode.is_td();
        let tx = tw.mqtx.get(0).unwrap().clone();
        let mut cw = tw.cur_creep_wave;
        if cw.wave >= tw.creep_waves.len() {
            return;
        }
        let Some(w) = tw.creep_waves.get(cw.wave) else { return };

        // TD 模式：只有按 StartRound 後 is_running=true 才出怪；
        // 波的參考開始時間改用 `cw.wave_start_time`（按下時記錄的 totaltime）。
        // 非 TD：沿用原時間觸發（`w.time` 絕對開始時間）。
        let ref_time = if is_td { cw.wave_start_time } else { w.time };
        let can_run = if is_td { cw.is_running } else { w.time < totaltime as f32 };
        if !can_run {
            return;
        }

        if cw.path.is_empty() {
            cw.path.resize(w.path_creeps.len(), 0);
        }

        let mut is_end = true;
        for (i, pc) in w.path_creeps.iter().enumerate() {
            let cur_path_idx = cw.path[i];
            if cur_path_idx < pc.creeps.len() {
                is_end = false;
                if pc.creeps[cur_path_idx].time + ref_time < totaltime as f32 {
                    let cp = tr.creep_emiters.get(&pc.creeps[cur_path_idx].name);
                    let path = tr.paths.get(&pc.path_name);
                    if let (Some(cp), Some(path)) = (cp, path) {
                        if let Some(ct) = path.check_points.first() {
                            let mut cpp = cp.root.clone();
                            cpp.path = pc.path_name.clone();
                            let cp0 = CreepData {
                                pos: ct.pos.clone(),
                                creep: cpp.clone(),
                                cdata: cp.property.clone(),
                                faction_name: cp.faction_name.clone(),
                                turn_speed_deg: cp.turn_speed_deg,
                                collision_radius: cp.collision_radius,
                            };
                            tw.outcomes.push(Outcome::Creep { cd: cp0 });
                        }
                    }
                    cw.path[i] += 1;
                }
            }
        }

        if is_end {
            // 所有本波小兵都已派出；TD 模式還要等地圖上沒有活著的 creep 才算結束。
            let any_alive = (&tr.creeps).join().next().is_some();
            if is_td && any_alive {
                return;
            }
            if is_td {
                // TD 模式：推進到下一波、進入 idle，等玩家按 StartRound
                cw.wave += 1;
                cw.path.clear();
                cw.is_running = false;
                let finished = cw.wave; // 已完成的波數（從 1 開始給前端看）
                let total = tw.creep_waves.len();
                let payload = json!({
                    "round": finished,
                    "total": total,
                    "is_running": false,
                });
                // P5: game/round + game/end are game-wide — reach every player.
                #[cfg(any(feature = "grpc", feature = "kcp"))]
                let round_msg = OutboundMsg::new_s_all("td/all/res", "game", "round", payload);
                #[cfg(not(any(feature = "grpc", feature = "kcp")))]
                let round_msg = OutboundMsg::new_s("td/all/res", "game", "round", payload);
                let _ = tx.try_send(round_msg);
                log::info!("✅ TD 第 {} 波結束，等待 StartRound（已完成 {}/{}）", finished, finished, total);
                // 所有波都打完 → 勝利
                if finished >= total {
                    let end_payload = json!({ "result": "victory", "reason": "all_rounds_cleared" });
                    #[cfg(any(feature = "grpc", feature = "kcp"))]
                    let end_msg = OutboundMsg::new_s_all("td/all/res", "game", "end", end_payload);
                    #[cfg(not(any(feature = "grpc", feature = "kcp")))]
                    let end_msg = OutboundMsg::new_s("td/all/res", "game", "end", end_payload);
                    let _ = tx.try_send(end_msg);
                    log::info!("🏆 TD 勝利：全部 {} 波已清空", total);
                }
            } else {
                cw.wave += 1;
                cw.path.clear();
            }
        }
    }
}
