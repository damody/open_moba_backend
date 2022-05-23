use crate::{comp, uid::Uid, Creep, CProperty};
use hashbrown::HashSet;
use serde::{Deserialize, Serialize};
use vek::*;
use specs::Entity as EcsEntity;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::ops::DerefMut;

use super::Projectile;

#[derive(Clone, Debug)]
pub enum Outcome {
    Damage {
        pos: Vec2<f32>,
        phys: f32,
        magi: f32,
        real: f32,
        source: Uid,
        target: Uid,
    },
    // not yet used
    ProjectileLine2 {
        pos: Vec2<f32>,
        source: Option<EcsEntity>,
        target: Option<EcsEntity>,
    },
    Death {
        pos: Vec2<f32>,
        ent: EcsEntity,
    },
    Creep {
        pos: Vec2<f32>,
        creep: Creep,
        cdata: CProperty,
    }
}
// 位置是更新用的
// 需要讓玩家更新的事件才需要位置
/*
impl Outcome {
    pub fn get_pos(&self) -> Option<Vec2<f32>> {
        match self {
            Outcome::ProjectileLine { pos, .. }
            | Outcome::ProjectileLine2 { pos, .. }
            | Outcome::ProjectileHit { pos, .. }
            | Outcome::Damage { pos, .. }
            | Outcome::Death { pos, .. } => Some(pos.clone()),
            Outcome::Death { .. }  => None,
        }
    }
}*/

#[derive(Clone, Debug)]
pub enum ServerEvent {
    ProjectileLine {
        pos: Vec2<f32>,
        vel: Vec2<f32>,
        p: Projectile,
    },
}

pub struct EventBus<E> {
    queue: Mutex<VecDeque<E>>,
}

impl<E> Default for EventBus<E> {
    fn default() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
        }
    }
}

impl<E> EventBus<E> {
    pub fn emitter(&self) -> Emitter<E> {
        Emitter {
            bus: self,
            events: VecDeque::new(),
        }
    }

    pub fn emit_now(&self, event: E) { self.queue.lock().unwrap().push_back(event); }

    pub fn recv_all(&self) -> impl ExactSizeIterator<Item = E> {
        std::mem::take(self.queue.lock().unwrap().deref_mut()).into_iter()
    }
}

pub struct Emitter<'a, E> {
    bus: &'a EventBus<E>,
    events: VecDeque<E>,
}

impl<'a, E> Emitter<'a, E> {
    pub fn emit(&mut self, event: E) { self.events.push_back(event); }

    pub fn append(&mut self, other: &mut VecDeque<E>) { self.events.append(other) }

    // TODO: allow just emitting the whole vec of events at once? without copying
    pub fn append_vec(&mut self, vec: Vec<E>) { self.events.extend(vec) }
}

impl<'a, E> Drop for Emitter<'a, E> {
    fn drop(&mut self) { self.bus.queue.lock().unwrap().append(&mut self.events); }
}

