//! Events enqueued by the host's tick systems; drained by `run_script_dispatch`.
//!
//! Uses `specs::Entity` directly on the host side (crossing the FFI boundary
//! converts to `omb_script_abi::EntityHandle`).
//!
//! 變種分類：
//! * **生命週期**：Spawn / Death / Respawn
//! * **傷害 / 攻擊**：Damage / AttackHit / AttackStart / AttackLanded / AttackFail / Attacked
//! * **資源 / 狀態**：HealthGained / ManaGained / SpentMana / HealReceived
//!   / StateChanged / ModifierAdded / ModifierRemoved
//! * **技能 / 命令**：SkillCast / Order

use omb_script_abi::types::DamageKind;
use specs::Entity;

#[derive(Clone, Debug)]
pub enum ScriptEvent {
    // ---- 生命週期 ----
    Spawn {
        e: Entity,
    },
    Death {
        victim: Entity,
        killer: Option<Entity>,
    },
    Respawn {
        e: Entity,
    },

    // ---- 傷害 / 攻擊 ----
    /// Raised by the damage pipeline BEFORE HP is decremented.
    /// Scripts may mutate the amount during dispatch.
    Damage {
        attacker: Option<Entity>,
        victim: Entity,
        amount: f32,
        kind: DamageKind,
    },
    AttackHit {
        attacker: Entity,
        victim: Entity,
    },
    /// 攻擊動作準備發射（target 可能為 None，例如 orb 技能無目標）。
    AttackStart {
        attacker: Entity,
        target: Option<Entity>,
    },
    /// 攻擊確認命中（含最終 damage）。
    AttackLanded {
        attacker: Entity,
        victim: Entity,
        damage: f32,
    },
    /// 攻擊 miss / 被閃避。
    AttackFail {
        attacker: Entity,
        victim: Entity,
    },
    /// 被攻擊的通用事件（victim side；命中或未命中皆派發）。
    Attacked {
        attacker: Entity,
        victim: Entity,
    },

    // ---- 資源 / 狀態 ----
    HealthGained {
        e: Entity,
        amount: f32,
    },
    ManaGained {
        e: Entity,
        amount: f32,
    },
    SpentMana {
        caster: Entity,
        cost: f32,
        ability_id: String,
    },
    HealReceived {
        target: Entity,
        amount: f32,
        source: Option<Entity>,
    },
    StateChanged {
        e: Entity,
        state_id: String,
        active: bool,
    },
    ModifierAdded {
        e: Entity,
        modifier_id: String,
    },
    ModifierRemoved {
        e: Entity,
        modifier_id: String,
    },

    // ---- 技能 / 命令 ----
    SkillCast {
        caster: Entity,
        skill_id: String,
        target: SkillTarget,
    },
    Order {
        e: Entity,
        order_kind: String,
        target: SkillTarget,
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
