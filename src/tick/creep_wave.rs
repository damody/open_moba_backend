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
use crate::msg::MqttMsg;
use serde_json::json;

#[derive(SystemData)]
pub struct CreepWaveRead<'a> {
    entities: Entities<'a>,
    time: Read<'a, Time>,
    dt: Read<'a, DeltaTime>,
    creep_emiters: Read<'a, BTreeMap<String, CreepEmiter>>,
    paths: Read<'a, BTreeMap<String, Path>>,
    check_points : Read<'a, BTreeMap<String, CheckPoint>>,
}

#[derive(SystemData)]
pub struct CreepWaveWrite<'a> {
    outcomes: Write<'a, Vec<Outcome>>,
    cur_creep_wave: Write<'a, CurrentCreepWave>,
    creep_waves: Write<'a, Vec<CreepWave>>,
    mqtx: Write<'a, Vec<Sender<MqttMsg>>>,
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
        let dt = tr.dt.0;
        let tx = tw.mqtx.get(0).unwrap().clone();
        let mut cw = tw.cur_creep_wave;
        if  cw.wave < tw.creep_waves.len() {
            if let Some(w) = tw.creep_waves.get(cw.wave) {
                if w.time < totaltime as f32 {
                    if cw.path.len() == 0 { // 第一次進來這波怪要初始化
                        cw.path.resize(w.path_creeps.len(), 0);
                    }
                    let mut is_end = true;
                    for (i, pc) in w.path_creeps.iter().enumerate() {
                        let cur_path_idx = cw.path[i];
                        if cur_path_idx < pc.creeps.len() {
                            is_end = false;
                            if pc.creeps[cur_path_idx].time + w.time < totaltime as f32 {
                                // 增加creep
                                let cp = tr.creep_emiters.get(&pc.creeps[cur_path_idx].name);
                                let path = tr.paths.get(&pc.path_name);
                                if let (Some(cp), Some(path)) = (cp, path) {
                                    let cpoint = path.check_points.get(0);
                                    if let Some(ct) = cpoint {
                                        let mut cpp = cp.root.clone();
                                        cpp.path = pc.path_name.clone();
                                        let cp0 = CreepData {
                                            pos: ct.pos.clone(),
                                            creep: cpp.clone(),
                                            cdata: cp.property.clone(),
                                        };
                                        tw.outcomes.push(Outcome::Creep { cd: cp0 });
                                    }
                                }
                                cw.path[i] += 1;
                            }
                        }
                    }
                    if is_end {
                        cw.wave += 1;
                        cw.path.clear(); // 清空路徑狀態，讓下一波重新初始化
                    }
                }
            }
        }
    }
}
