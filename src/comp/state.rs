use std::{thread, ops::Deref};
use rayon::{ThreadPool, ThreadPoolBuilder};
use specs::{
    prelude::Resource,
    shred::{Fetch, FetchMut},
    storage::{MaskedStorage as EcsMaskedStorage, Storage as EcsStorage},
    Component, DispatcherBuilder, Entity as EcsEntity, WorldExt, Builder,
};
use std::sync::Arc;
use vek::*;
use crate::comp::*;
use super::last::Last;
use std::time::{Instant};
use core::{convert::identity, time::Duration};
use failure::{err_msg, Error};

use crate::tick::*;
use crate::sync::*;
use crate::sync::interpolation::*;
use crate::Outcome;
use crate::ProjectileConstructor;

use specs::saveload::MarkerAllocator;
use rand::{thread_rng, Rng};
use rand::distributions::{Alphanumeric, Uniform, Standard};


pub struct State {
    ecs: specs::World,
    // Avoid lifetime annotation by storing a thread pool instead of the whole dispatcher
    thread_pool: Arc<ThreadPool>,
}

/// How much faster should an in-game day be compared to a real day?
// TODO: Don't hard-code this.
const DAY_CYCLE_FACTOR: f64 = 24.0 * 1.0;
const MAX_DELTA_TIME: f32 = 1.0;

impl State {
    pub fn new() -> Self {

        let thread_pool = Arc::new(
            ThreadPoolBuilder::new()
                .num_threads(num_cpus::get())
                .thread_name(move |i| format!("rayon-{}", i))
                .build()
                .unwrap(),
        );
        let mut res = Self {
            ecs: Self::setup_ecs_world(&thread_pool),
            thread_pool,
        };
        Self::create_test_scene(&mut res.ecs);
        res
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
        ecs.insert(EventBus::<ServerEvent>::default());

        ecs.insert(Tick(0));
        ecs.insert(TickStart(Instant::now()));
        ecs.insert(SysMetrics::default());
        ecs.insert(Vec::<Outcome>::new());
        ecs.insert(Vec::<TakenDamage>::new());
        ecs.insert(Vec::<EcsEntity>::new());
        ecs.insert(vec![instant_distance::Builder::default().build(vec![Pos(Vec2::new(0.,0.))], vec![uid::Uid(0)])]);

        // Set starting time for the server.
        ecs.write_resource::<TimeOfDay>().0 = 0.0;
        ecs
    }
    
    fn create_test_scene(ecs: &mut specs::World) {
        ecs.create_entity_synced()
            .with(Pos(Vec2::new(0.,0.)))
            .with(Tower{lv:1, projectile_kind: ProjectileConstructor::Arrow{}, nearby_creeps: vec![]})
            .with(TProperty::new(10, 1., 2., 0.1, 30.))
            .build();
        ecs.create_entity_synced()
            .with(Pos(Vec2::new(0.,10.)))
            .with(Creep{lv:1})
            .with(CProperty{hp:100., msd:0.5, def_physic: 1., def_magic: 2.})
            .build();
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
        
        self.process_outcomes();

        let mut dispatch_builder = DispatcherBuilder::new().with_pool(Arc::clone(&self.thread_pool));
        
        //dispatch::<interpolation::Sys>(&mut dispatch_builder, &[]);
        dispatch::<projectile_tick::Sys>(&mut dispatch_builder, &[]);
        dispatch::<nearby_tick::Sys>(&mut dispatch_builder, &[]);
        dispatch::<tower_tick::Sys>(&mut dispatch_builder, &[&nearby_tick::Sys::sys_name()]);
        dispatch::<creep_tick::Sys>(&mut dispatch_builder, &[&tower_tick::Sys::sys_name()]);

        let mut dispatcher = dispatch_builder.build();
        dispatcher.dispatch(&self.ecs);
        self.ecs.maintain();
        Ok(())
    }
    pub fn process_outcomes(&mut self) -> Result<(), Error> {
        let mut sevents = vec![];
        let mut remove_uids = vec![];
        {
            let outcomes = self.ecs.read_resource::<Vec<Outcome>>();
            for out in outcomes.iter() {
                match out {
                    Outcome::Death { pos: p, uid: u } => {
                        remove_uids.push(u.0);
                    }
                    Outcome::ProjectileLine2{ pos, source, target } => { 
                        let mut e1 = self.ecs.entity_from_uid(source.ok_or(err_msg("err"))?.0).ok_or(err_msg("err"))?;
                        let mut e2 = self.ecs.entity_from_uid(target.ok_or(err_msg("err"))?.0).ok_or(err_msg("err"))?;
                        
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
                        
                        for i in 0..100000 {
                            let v = v * roll_scale.next().unwrap();
                            sevents.push(ServerEvent::ProjectileLine{pos: pos.clone(), vel: v, 
                                p: t.projectile_kind.create_projectile(source.clone(), tp.atk_physic, tp.range)});
                        }
                        
                    }
                    _=>{}
                }
            }
        }
        let ents = self.ecs.read_resource::<Vec<EcsEntity>>().deref().clone();
        for e in ents.iter() {
            //self.ecs.delete_entity(*e);
        }
        log::info!("map size {}", self.mut_resource::<UidAllocator>().mapping.len());
        if self.mut_resource::<UidAllocator>().mapping.len() > 248000 {
            self.mut_resource::<UidAllocator>().mapping.clear();
            self.ecs.write_storage::<Projectile>().clear();
        }
        
        for u in remove_uids {
            if let Some(e) = self.mut_resource::<UidAllocator>().retrieve_entity_internal(u) {
                //self.ecs.delete_entity(e);
            }
            self.ecs.delete_entity_and_clear_from_uid_allocator(u);
        }
        for s in sevents {
            match s {
                ServerEvent::ProjectileLine { pos, vel, p } => {
                    self.ecs.create_entity_synced().with(Pos(pos)).with(Vel(vel)).with(p).build();
                    //self.ecs.create_entity().with(Pos(pos)).with(Vel(vel)).with(p).build();
                }
            }
        }
        self.ecs.write_resource::<Vec<EcsEntity>>().clear();
        self.ecs.write_resource::<Vec<Outcome>>().clear();
        Ok(())
    }
}