use std::{thread, ops::Deref, collections::BTreeMap};
use rayon::{ThreadPool, ThreadPoolBuilder};
use specs::{
    prelude::Resource,
    shred::{Fetch, FetchMut},
    storage::{MaskedStorage as EcsMaskedStorage, Storage as EcsStorage},
    Component, DispatcherBuilder, Entity, WorldExt, Builder,
};
use specs::world::Generation;
use std::sync::Arc;
use vek::*;
use crate::{comp::*, msg::MqttMsg};
use super::last::Last;
use std::time::{Instant};
use core::{convert::identity, time::Duration};
use failure::{err_msg, Error};
use serde::{Deserialize, Serialize};

use crate::tick::*;
use crate::Outcome;
use crate::Projectile;
use crate::PlayerData;
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
    mqrx: Receiver<PlayerData>,
    // Avoid lifetime annotation by storing a thread pool instead of the whole dispatcher
    thread_pool: Arc<ThreadPool>,
}

/// How much faster should an in-game day be compared to a real day?
// TODO: Don't hard-code this.
const DAY_CYCLE_FACTOR: f64 = 24.0 * 1.0;
const MAX_DELTA_TIME: f32 = 1.0;

impl State {
    pub fn new(pcw: CreepWaveData, mqtx: Sender<MqttMsg>, mqrx: Receiver<PlayerData>) -> Self {
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
            mqrx: mqrx.clone(),
            thread_pool,
        };
        res.init_creep_wave();
        res.create_test_scene();
        res
    }
    fn init_creep_wave(&mut self) {
        self.ecs.insert(vec![self.mqtx.clone()]);
        self.ecs.insert(vec![self.mqrx.clone()]);
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
                    root: Creep{name: cp.Name.clone(), path: "".to_owned(), pidx: 0, block_tower: None, status: CreepStatus::Walk}, 
                    property: CProperty { hp: cp.HP, mhp: cp.HP, msd: cp.MoveSpeed, def_physic: cp.DefendPhysic, def_magic: cp.DefendMagic } });
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
        // Register all components.
        ecs.register::<Pos>();
        ecs.register::<Vel>();
        ecs.register::<TProperty>();
        ecs.register::<TAttack>();
        ecs.register::<CProperty>();
        ecs.register::<Tower>();
        ecs.register::<Creep>();
        ecs.register::<Projectile>();
        // Register unsynced resources used by the ECS.
        ecs.insert(TimeOfDay(0.0));
        ecs.insert(Time(0.0));
        ecs.insert(DeltaTime(0.0));
        ecs.insert(Tick(0));
        ecs.insert(TickStart(Instant::now()));
        ecs.insert(SysMetrics::default());
        ecs.insert(Vec::<Outcome>::new());
        ecs.insert(Vec::<TakenDamage>::new());
        ecs.insert(Vec::<CreepWave>::new());
        ecs.insert(CurrentCreepWave{wave: 0, path: vec![]});
        ecs.insert(BTreeMap::<String, Player>::new());
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
        /*for x in (0..200).step_by(100) {
            for y in (0..200).step_by(100) {
                count += 1;
                ocs.push(Outcome::Tower { pos: Vec2::new(x as f32+200., y as f32+200.),
                    td: TowerData {
                    tpty: TProperty::new(10, 3, 100.),
                    tatk: TAttack::new(3., 1., 300., 100.),
                } });
            }    
        }*/
        log::warn!("count {}", count);
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
        
        dispatch::<projectile_tick::Sys>(&mut dispatch_builder, &[]);
        dispatch::<nearby_tick::Sys>(&mut dispatch_builder, &[]);
        dispatch::<player_tick::Sys>(&mut dispatch_builder, &[]);
        dispatch::<tower_tick::Sys>(&mut dispatch_builder, &[]);
        dispatch::<creep_tick::Sys>(&mut dispatch_builder, &[]);
        dispatch::<creep_wave::Sys>(&mut dispatch_builder, &[]);

        let mut dispatcher = dispatch_builder.build();
        dispatcher.dispatch(&self.ecs);

        self.creep_wave();
        self.process_outcomes();
        self.process_playerdatas();
        self.ecs.maintain();
        Ok(())
    }
    pub fn handle_tower(&mut self, pd: PlayerData) -> Result<(), Error> {
        match pd.a.as_str() {
            "R" => {
                self.mqtx.try_send(MqttMsg::new_s("td/all/res", "tower", "R", json!({"msg":"ok"})))?;
            }
            "C" => {
                #[derive(Serialize, Deserialize)]
                struct JData {
                    tid: i32,
                    x: f32,
                    y: f32,
                };
                let mut v: JData = serde_json::from_value(pd.d)?;
                let t = {
                    let mut pmap = self.ecs.get_mut::<BTreeMap<String, Player>>().unwrap();
                    if let Some(p) = pmap.get_mut(&pd.name) {
                        if let Some(t) = p.towers.get(v.tid as usize) {
                            Some(t.clone())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };
                let mut ocs = self.ecs.get_mut::<Vec<Outcome>>().unwrap();
                if let Some(t) = t {
                    ocs.push(Outcome::Tower { pos: Vec2::new(v.x,v.y), td: TowerData { tpty: t.tpty, tatk: t.tatk } });
                    self.mqtx.try_send(MqttMsg::new_s("td/all/res", "tower", "C", json!({"msg":"ok"})))?;
                } else {
                    self.mqtx.try_send(MqttMsg::new_s("td/all/res", "tower", "C", json!({"msg":"fail"})))?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub fn handle_player(&mut self, pd: PlayerData) -> Result<(), Error> {
        let mut pmap = self.ecs.get_mut::<BTreeMap<String, Player>>().unwrap();
        match pd.a.as_str() {
            "C" => {
                let mut p = Player { name: pd.name.clone(), cost: 100., towers: vec![] };
                p.towers.push(TowerData { tpty: TProperty::new(10., 1, 100.), tatk: TAttack::new(3., 0.3, 300., 100.) });
                pmap.insert(pd.name.clone(), p);
                self.mqtx.try_send(MqttMsg::new_s("td/all/res", "player", "C", json!({"msg":"ok"})))?;
            }
            _ => {}
        }
        Ok(())
    }
    pub fn process_playerdatas(&mut self) -> Result<(), Error> {
        let n = self.mqrx.len();
        for i in 0..n {
            let data = self.mqrx.try_recv();
            if let Ok(d) = data {
                log::warn!("{:?}", d);
                match d.t.as_str() {
                    "tower" => {
                        self.handle_tower(d)?;
                    }
                    "player" => {
                        self.handle_player(d)?;
                    }
                    _ => {}
                }
            } else {
                log::warn!("json error");
            }
        }
        Ok(())
    }
    pub fn process_outcomes(&mut self) -> Result<(), Error> {
        let mut remove_uids = vec![];
        let mut next_outcomes = vec![];
        {
            let mut ocs = self.ecs.get_mut::<Vec<Outcome>>().unwrap();
            let mut outcomes = vec![];
            outcomes.append(ocs);
            for out in outcomes {
                match out {
                    Outcome::Death { pos: p, ent: e } => {
                        remove_uids.push(e);
                        let mut creeps = self.ecs.write_storage::<Creep>();
                        let mut towers = self.ecs.write_storage::<Tower>();
                        let mut projs = self.ecs.write_storage::<Projectile>();
                        let t = if let Some(c) = creeps.get_mut(e) {
                            if let Some(bt) = c.block_tower {
                                if let Some(t) = towers.get_mut(bt) { 
                                    t.block_creeps.retain(|&x| x != e);
                                }
                            }
                            "creep"
                        } else if let Some(t) = towers.get_mut(e) {
                            for ce in t.block_creeps.iter() {
                                if let Some(c) = creeps.get_mut(*ce) { 
                                    c.block_tower = None;
                                    next_outcomes.push(Outcome::CreepWalk { target: ce.clone() });
                                }
                            }
                            "tower"
                        } else if let Some(p) = projs.get_mut(e) {
                            "projectile"
                        } else { "" };
                        if t != "" {
                            self.mqtx.send(MqttMsg::new_s("td/all/res", t, "D", json!({"id": e.id()})));
                        }
                    }
                    Outcome::ProjectileLine2{ pos, source, target } => { 
                        let mut e1 = source.ok_or(err_msg("err"))?;
                        let mut e2 = target.ok_or(err_msg("err"))?;
                        let (msd, p2) = {
                            let positions = self.ecs.read_storage::<Pos>();
                            let tproperty = self.ecs.read_storage::<TAttack>();
                            
                            let p1 = positions.get(e1).ok_or(err_msg("err"))?;
                            let p2 = positions.get(e2).ok_or(err_msg("err"))?;
                            let tp = tproperty.get(e1).ok_or(err_msg("err"))?;
                            (tp.bullet_speed, p2.0)
                        };
                        let ntarget = if let Some(t) = target {
                            t.id()
                        } else { 0 };
                        let e = self.ecs.create_entity().with(Pos(pos))
                            .with(Projectile { time_left: 3., owner: e1.clone(), tpos: p2, target: target, radius: 0., msd: msd }).build();
                        let mut pjs = json!(ProjectileData {
                            id: e.id(), pos: pos.clone(), msd: msd,
                            time_left: 3., owner: e1.id(), target: ntarget, radius: 0.,
                        });
                        self.mqtx.try_send(MqttMsg::new_s("td/all/res", "projectile", "C", json!(pjs)));
                    }
                    Outcome::Creep { cd } => {
                        let mut cjs = json!(cd);
                        let e = self.ecs.create_entity().with(Pos(cd.pos)).with(cd.creep).with(cd.cdata).build();
                        cjs.as_object_mut().unwrap().insert("id".to_owned(), json!(e.id()));
                        self.mqtx.try_send(MqttMsg::new_s("td/all/res", "creep", "C", json!(cjs)));
                    }
                    Outcome::Tower { pos, td } => {
                        let mut cjs = json!(td);
                        let e = self.ecs.create_entity().with(Pos(pos)).with(Tower::new()).with(td.tpty).with(td.tatk).build();
                        cjs.as_object_mut().unwrap().insert("id".to_owned(), json!(e.id()));
                        cjs.as_object_mut().unwrap().insert("pos".to_owned(), json!(pos));
                        self.mqtx.try_send(MqttMsg::new_s("td/all/res", "tower", "C", json!(cjs)));
                        self.ecs.get_mut::<Searcher>().unwrap().tower.needsort = true;
                    }
                    Outcome::CreepStop { source, target } => {
                        let mut creeps = self.ecs.write_storage::<Creep>();
                        let c = creeps.get_mut(target).ok_or(err_msg("err"))?;
                        c.block_tower = Some(source);
                        c.status = CreepStatus::Stop;
                        let positions = self.ecs.read_storage::<Pos>();
                        let pos = positions.get(target).ok_or(err_msg("err"))?;
                        self.mqtx.try_send(MqttMsg::new_s("td/all/res", "creep", "M", json!({
                            "id": target.id(),
                            "x": pos.0.x,
                            "y": pos.0.y,
                        })));
                    }
                    Outcome::CreepWalk { target } => {
                        let mut creeps = self.ecs.write_storage::<Creep>();
                        let creep = creeps.get_mut(target).ok_or(err_msg("err"))?;
                        creep.status = CreepStatus::PreWalk;
                    }
                    _=>{}
                }
            }
        }
        self.ecs.delete_entities(&remove_uids[..]);
        self.ecs.write_resource::<Vec<Outcome>>().clear();
        self.ecs.write_resource::<Vec<Outcome>>().append(&mut next_outcomes);
        Ok(())
    }
    pub fn creep_wave(&mut self) -> Result<(), Error> {
        Ok(())
    }
}