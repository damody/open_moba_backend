//! Events enqueued by the host's tick systems; drained by `run_script_dispatch`.
//!
//! Uses `specs::Entity` directly on the host side (crossing the FFI boundary
//! converts to `omb_script_abi::EntityHandle`).

use omb_script_abi::types::DamageKind;
use specs::Entity;

#[derive(Clone, Debug)]
pub enum ScriptEvent {
    Spawn {
        e: Entity,
    },
    Death {
        victim: Entity,
        killer: Option<Entity>,
    },
    /// Raised by the damage pipeline BEFORE HP is decremented.
    /// Scripts may mutate the amount during dispatch.
    Damage {
        attacker: Option<Entity>,
        victim: Entity,
        amount: f32,
        kind: DamageKind,
    },
    SkillCast {
        caster: Entity,
        skill_id: String,
        target: SkillTarget,
    },
    AttackHit {
        attacker: Entity,
        victim: Entity,
    },
}

#[derive(Clone, Debug)]
pub enum SkillTarget {
    Entity(Entity),
    Point(f32, f32),
    None,
}

/// specs `Resource` holding the queue of pending script events.
#[derive(Default)]
pub struct ScriptEventQueue {
    events: Vec<ScriptEvent>,
}

impl ScriptEventQueue {
    pub fn push(&mut self, ev: ScriptEvent) {
        self.events.push(ev);
    }
    pub fn drain(&mut self) -> Vec<ScriptEvent> {
        std::mem::take(&mut self.events)
    }
    pub fn len(&self) -> usize {
        self.events.len()
    }
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}
