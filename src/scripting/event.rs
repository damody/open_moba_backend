//! 由主機的滴答系統排隊的事件；由「run_script_dispatch」耗盡。
//!
//! 直接在主機端使用“specs::Entity”（跨越 FFI 邊界
//! 轉換為 `omb_script_abi::EntityHandle`)。
//!
//! 變種分類：
//! * **生命週期**：Spawn / Death / Respawn
//! * **傷害 / 攻擊**：Damage / AttackHit / AttackStart / AttackLanded / AttackFail / Attacked
//! * **資源 / 狀態**：HealthGained / ManaGained / SpentMana / HealReceived
//! / StateChanged / 修改器已新增 / 修改器已刪除
//! * **技能 / 命令**：SkillCast / Order

use omb_script_abi::types::DamageKind;
use specs::Entity;
use omoba_sim::Fixed64;

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
    /// 在 HP 減少之前由傷害管道提高。
    /// 腳本可能會在調度期間改變金額。
    Damage {
        attacker: Option<Entity>,
        victim: Entity,
        amount: Fixed64,
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
        damage: Fixed64,
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
        amount: Fixed64,
    },
    ManaGained {
        e: Entity,
        amount: Fixed64,
    },
    SpentMana {
        caster: Entity,
        cost: Fixed64,
        ability_id: String,
    },
    HealReceived {
        target: Entity,
        amount: Fixed64,
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
    /// 英雄習得技能（或升等）時 push。dispatch 會呼對應 AbilityScript::on_learn；
    /// Passive 技用此時機套永久 buff。
    SkillLearn {
        caster: Entity,
        skill_id: String,
        new_level: u8,
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
    Point { x: Fixed64, y: Fixed64 },
    None,
}

/// 規範「資源」保存待處理腳本事件的佇列。
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
