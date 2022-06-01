use std::{thread, ops::Deref, collections::BTreeMap};
use rayon::{ThreadPool, ThreadPoolBuilder};
use specs::{
    prelude::Resource,
    shred::{Fetch, FetchMut},
    storage::{MaskedStorage as EcsMaskedStorage, Storage as EcsStorage},
    Component, DispatcherBuilder, Entity as EcsEntity, WorldExt, Builder,
};
use std::sync::Arc;
use vek::*;
use crate::{comp::*, msg::MqttMsg};
use super::last::Last;
use std::time::{Instant};
use core::{convert::identity, time::Duration};
use failure::{err_msg, Error};

use crate::tick::*;
use crate::sync::*;
use crate::sync::interpolation::*;
use crate::Outcome;
use crate::Projectile;

use crate::ue4::import_map::CreepWaveData;
use serde_json::json;

use specs::saveload::MarkerAllocator;
use rand::{thread_rng, Rng};
use rand::distributions::{Alphanumeric, Uniform, Standard};
use crossbeam_channel::{bounded, select, tick, Receiver, Sender};

pub struct State {
    ecs: specs::World,
    cw: CreepWaveData,
    mqtx: Sender<MqttMsg>,
    // Avoid lifetime annotation by storing a thread pool instead of the whole dispatcher
    thread_pool: Arc<ThreadPool>,
}

/// How much faster should an in-game day be compared to a real day?
// TODO: Don't hard-code this.
const DAY_CYCLE_FACTOR: f64 = 24.0 * 1.0;
const MAX_DELTA_TIME: f32 = 1.0;

impl State {
    pub fn new(pcw: CreepWaveData, mqtx: Sender<MqttMsg>) -> Self {
        let thread_pool = Arc::new(
            ThreadPoolBuilder::new()
                .num_threads(num_cpus::get())
                .thread_name(move |i| format!("rayon-{}", i))
                .build()
                .unwrap(),
        );
        let mut res = Self {
            ecs: Self::setup_ecs_world(&thread_pool),
            cw: pcw,
            mqtx: mqtx.clone(),
            thread_pool,
        };
        res.init_creep_wave();
        res.create_test_scene();
        res
    }
    fn init_creep_wave(&mut self) {
        self.ecs.insert(vec![self.mqtx.clone()]);
        let cps = {
            let mut cps = self.ecs.get_mut::<BTreeMap::<String, CheckPoint>>().unwrap();
            for p in self.cw.CheckPoint.iter() {
                cps.insert(p.Name.clone(), 
                    CheckPoint{name:p.Name.clone(), class: p.Class.clone(), pos: Vec2::new(p.X, p.Y)});
            }
            cps.clone()
        };
        {
            let mut paths = self.ecs.get_mut::<BTreeMap::<String, Path>>().unwrap();
            for p in self.cw.Path.iter() {
                let mut cp_in_path = vec![];
                for ps in p.Points.iter() {
                    if let Some(v) = cps.get(ps) {
                        cp_in_path.push(v.clone());
                    }
                }
                paths.insert(p.Name.clone(), 
                    Path {check_points: cp_in_path});
            }
        }
        {
            let mut ces = self.ecs.get_mut::<BTreeMap::<String, CreepEmiter>>().unwrap();
            for cp in self.cw.Creep.iter() {
                ces.insert(cp.Name.clone(), CreepEmiter { 
                    root: Creep{class: cp.Class.clone(), path: "".to_owned(), pidx: 0}, 
                    property: CProperty { hp: cp.HP, msd: cp.MoveSpeed, def_physic: cp.DefendPhysic, def_magic: cp.DefendMagic } });
            }
        }
        {
            let mut cws = self.ecs.get_mut::<Vec::<CreepWave>>().unwrap();
            for cw in self.cw.CreepWave.iter() {
                let mut tcw = CreepWave { time: cw.StartTime, path_creeps: vec![] };
                let mut pcs: &mut Vec<PathCreeps> = &mut tcw.path_creeps;
                for d in cw.Detail.iter() {
                    let mut es = vec![];
                    for cjd in d.Creeps.iter() {
                        es.push(CreepEmit{time: cjd.Time, name: cjd.Creep.clone()});
                    }
                    pcs.push(PathCreeps { creeps: es, path_name: d.Path.clone() });
                }
                cws.push(tcw);
            }
        }
    }
    fn setup_ecs_world(thread_pool: &Arc<ThreadPool>) -> specs::World {
        let mut ecs = specs::World::new();
        // Uids for sync
        ecs.register_sync_marker();
        // Register all components.
        ecs.register::<Pos>();
        ecs.register::<Vel>();
        ecs.register::<TProperty>();
        ecs.register::<CProperty>();
        ecs.register::<Last<Pos>>();
        ecs.register::<Last<Vel>>();
        ecs.register::<InterpBuffer<Pos>>();
        ecs.register::<InterpBuffer<Vel>>();
        ecs.register::<Tower>();
        ecs.register::<Creep>();
        ecs.register::<Projectile>();
        // Register unsynced resources used by the ECS.
        ecs.insert(TimeOfDay(0.0));
        ecs.insert(Time(0.0));
        ecs.insert(DeltaTime(0.0));
        ecs.insert(PlayerEntity(None));
        ecs.insert(Tick(0));
        ecs.insert(TickStart(Instant::now()));
        ecs.insert(SysMetrics::default());
        ecs.insert(Vec::<Outcome>::new());
        ecs.insert(Vec::<TakenDamage>::new());
        ecs.insert(Vec::<CreepWave>::new());
        ecs.insert(CurrentCreepWave{wave: 0, path: vec![]});
        ecs.insert(BTreeMap::<String, CreepEmiter>::new());
        ecs.insert(BTreeMap::<String, Path>::new());
        ecs.insert(BTreeMap::<String, CheckPoint>::new());
        ecs.insert(Searcher::default());
        let e = ecs.entities_mut().create();

        // Set starting time for the server.
        ecs.write_resource::<TimeOfDay>().0 = 0.0;
        ecs
    }
    
    fn create_test_scene(&mut self) {
        let mut count = 0;
        let mut ocs = self.ecs.get_mut::<Vec<Outcome>>().unwrap();
        for x in (0..100).step_by(10) {
            for y in (0..100).step_by(10) {
                count += 1;
                ocs.push(Outcome::Tower { td: TowerData {
                    pos: Vec2::new(x as f32, y as f32),
                    tdata: TProperty::new(10, 3., 1., 1., 20.),
                } });
            }    
        }
        log::warn!("count {}", count);
        
        /*
        ecs.create_entity()
            .with(Pos(Vec2::new(0.,10.)))
            .with(Creep{class: "cp1".to_owned(), path: "path1".to_owned(), pidx: 0})
            .with(CProperty{hp:100., msd:0.5, def_physic: 1., def_magic: 2.})
            .build();*/
    }
    
    /// Get a reference to the internal ECS world.
    pub fn ecs(&self) -> &specs::World { &self.ecs }

    /// Get a mutable reference to the internal ECS world.
    pub fn ecs_mut(&mut self) -> &mut specs::World { &mut self.ecs }

    pub fn thread_pool(&self) -> &Arc<ThreadPool> { &self.thread_pool }

    /// Get the current in-game time of day.
    ///
    /// Note that this should not be used for physics, animations or other such
    /// localised timings.
    pub fn get_time_of_day(&self) -> f64 { self.ecs.read_resource::<TimeOfDay>().0 }

    /// Get the current in-game day period (period of the day/night cycle)
    /// Get the current in-game day period (period of the day/night cycle)
    pub fn get_day_period(&self) -> DayPeriod { self.get_time_of_day().into() }

    /// Get the current in-game time.
    ///
    /// Note that this does not correspond to the time of day.
    pub fn get_time(&self) -> f64 { self.ecs.read_resource::<Time>().0 }

    /// Get the current delta time.
    pub fn get_delta_time(&self) -> f32 { self.ecs.read_resource::<DeltaTime>().0 }

    /// Given mutable access to the resource R, assuming the resource
    /// component exists (this is already the behavior of functions like `fetch`
    /// and `write_component_ignore_entity_dead`).  Since all of our resources
    /// are generated up front, any failure here is definitely a code bug.
    pub fn mut_resource<R: Resource>(&mut self) -> &mut R {
        self.ecs.get_mut::<R>().expect(
            "Tried to fetch an invalid resource even though all our resources should be known at \
             compile time.",
        )
    }


    pub fn send_chat(&mut self, msg: String) {

    }

    pub fn tick(&mut self, dt: Duration) -> Result<(), Error> {
        self.ecs.write_resource::<Tick>().0 += 1;
        self.ecs.write_resource::<TickStart>().0 = Instant::now();
        self.ecs.write_resource::<TimeOfDay>().0 += dt.as_secs_f64() * DAY_CYCLE_FACTOR;
        self.ecs.write_resource::<Time>().0 += dt.as_secs_f64();
        self.ecs.write_resource::<DeltaTime>().0 = dt.as_secs_f32().min(MAX_DELTA_TIME);
        
        let mut dispatch_builder = DispatcherBuilder::new().with_pool(Arc::clone(&self.thread_pool));
        
        //dispatch::<interpolation::Sys>(&mut dispatch_builder, &[]);
        dispatch::<projectile_tick::Sys>(&mut dispatch_builder, &[]);
        dispatch::<nearby_tick::Sys>(&mut dispatch_builder, &[]);
        dispatch::<tower_tick::Sys>(&mut dispatch_builder, &[&nearby_tick::Sys::sys_name()]);
        dispatch::<creep_tick::Sys>(&mut dispatch_builder, &[&tower_tick::Sys::sys_name()]);
        dispatch::<creep_wave::Sys>(&mut dispatch_builder, &[&creep_tick::Sys::sys_name()]);

        let mut dispatcher = dispatch_builder.build();
        dispatcher.dispatch(&self.ecs);

        self.creep_wave();
        self.process_outcomes();
        self.ecs.maintain();
        Ok(())
    }
    pub fn process_outcomes(&mut self) -> Result<(), Error> {
        let mut remove_uids = vec![];
        {
            let mut ocs = self.ecs.get_mut::<Vec<Outcome>>().unwrap();
            let mut outcomes = vec![];
            outcomes.append(ocs);
            for out in outcomes {
                match out {
                    Outcome::Death { pos: p, ent: e } => {
                        remove_uids.push(e);
                        let creeps = self.ecs.read_storage::<Creep>();
                        let towers = self.ecs.read_storage::<Tower>();
                        let projs = self.ecs.read_storage::<Projectile>();
                        let t = if let Some(_) = creeps.get(e) {
                            "creep"
                        } else if let Some(_) = towers.get(e) {
                            "tower"
                        } else if let Some(_) = projs.get(e) {
                            "projectle"
                        } else { "" };
                        if t != "" {
                            //self.mqtx.send(MqttMsg::new_s("td/all/res", t, "D", json!({"id": e.id()})));
                        }
                    }
                    Outcome::ProjectileLine2{ pos, source, target } => { 
                        let mut e1 = source.ok_or(err_msg("err"))?;
                        let mut e2 = target.ok_or(err_msg("err"))?;
                        let v = {
                            let positions = self.ecs.read_storage::<Pos>();
                            let tower = self.ecs.read_storage::<Tower>();
                            let tproperty = self.ecs.read_storage::<TProperty>();
                            
                            let p1 = positions.get(e1).ok_or(err_msg("err"))?;
                            let p2 = positions.get(e2).ok_or(err_msg("err"))?;
                            let t = tower.get(e1).ok_or(err_msg("err"))?;
                            let tp = tproperty.get(e1).ok_or(err_msg("err"))?;
                            
                            let mut v = p2.0 - p1.0;
                            let mut rng = thread_rng();
                            let scale: Uniform<f32> = Uniform::new_inclusive(1., 2.);
                            let mut roll_scale = (&mut rng).sample_iter(scale);
                            
                            let v = v * roll_scale.next().unwrap();
                            v
                        };
                        let ntarget = if let Some(t) = target {
                            t.id()
                        } else { 0 };
                        let e = self.ecs.create_entity().with(Pos(pos)).with(Vel(v))
                            .with(Projectile { time_left: 3., owner: e1.clone(), target: target, radius: 0. }).build();
                        let mut pjs = json!(ProjectileData {
                            id: e.id(), pos: pos.clone(), vel: v.clone(),
                            time_left: 3., owner: e1.id(), target: ntarget, radius: 0.,
                        });
                        //self.mqtx.try_send(MqttMsg::new_s("td/all/res", "projectile", "C", json!(pjs)));
                    }
                    Outcome::Creep { cd } => {
                        let mut cjs = json!(cd);
                        let e = self.ecs.create_entity().with(Pos(cd.pos)).with(cd.creep).with(cd.cdata).build();
                        cjs.as_object_mut().unwrap().insert("id".to_owned(), json!(e.id()));
                        //self.mqtx.try_send(MqttMsg::new_s("td/all/res", "creep", "C", json!(cjs)));
                    }
                    Outcome::Tower { td } => {
                        let mut cjs = json!(td);
                        let e = self.ecs.create_entity().with(Pos(td.pos)).with(Tower::new()).with(td.tdata).build();
                        cjs.as_object_mut().unwrap().insert("id".to_owned(), json!(e.id()));
                        self.mqtx.try_send(MqttMsg::new_s("td/all/res", "tower", "C", json!(cjs)));
                        self.ecs.get_mut::<Searcher>().unwrap().tower.needsort = true;
                    }
                    _=>{}
                }
            }
        }
        self.ecs.delete_entities(&remove_uids[..]);
        self.ecs.write_resource::<Vec<Outcome>>().clear();
        Ok(())
    }
    pub fn creep_wave(&mut self) -> Result<(), Error> {
        Ok(())
    }
}