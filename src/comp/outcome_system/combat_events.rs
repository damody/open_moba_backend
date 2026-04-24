/// 戰鬥相關事件處理

use specs::{Entity, World, WriteStorage, ReadStorage, WorldExt};
use crate::ability_runtime::UnitStats;
use crate::comp::*;
use crate::transport::OutboundMsg;
use crossbeam_channel::Sender;
use omb_script_abi::stat_keys::StatKey;
use omb_script_abi::types::DamageKind;
use serde_json::json;

/// 戰鬥事件處理器
pub struct CombatEventHandler;

impl CombatEventHandler {
    /// 處理傷害事件 — 完整 Dota 2 pipeline：
    /// 1. 對 attacker 扣 Attacked 事件先 push（本 unit 被攻擊）
    /// 2. Evasion / miss roll → miss 就 push AttackFail 直接 return
    /// 3. UnitStats::apply_incoming_damage 套 armor / resist / block / prevention / incoming%
    /// 4. 扣 HP，套 MIN_HEALTH 下限
    /// 5. 死亡檢查：若有 REINCARNATION buff → Respawn 事件、HP 補滿、不發 Death
    /// 6. push AttackHit / AttackLanded 事件
    pub fn handle_damage(
        world: &mut World,
        mqtx: &Sender<OutboundMsg>,
        pos: vek::Vec2<f32>,
        phys: f32,
        magi: f32,
        real: f32,
        source: Entity,
        target: Entity,
    ) -> Vec<Outcome> {
        let mut next_outcomes = Vec::new();

        // ---- 先發 Attacked 給 victim 側（命中與否都發）----
        world.write_resource::<crate::scripting::ScriptEventQueue>()
            .push(crate::scripting::ScriptEvent::Attacked {
                attacker: source,
                victim: target,
            });

        // ---- Evasion / miss roll（基於 target 的閃避 + attacker 的 miss）----
        let (miss_chance, evasion) = {
            let buffs = world.read_resource::<crate::ability_runtime::BuffStore>();
            let is_bldgs = world.read_storage::<IsBuilding>();
            let tgt_stats = UnitStats::from_refs(&*buffs, is_bldgs.get(target).is_some());
            let src_stats = UnitStats::from_refs(&*buffs, is_bldgs.get(source).is_some());
            (src_stats.miss_chance(source), tgt_stats.evasion_chance(target))
        };
        let miss_roll = 1.0 - (1.0 - miss_chance) * (1.0 - evasion);
        if miss_roll > 0.0 && fastrand::f32() < miss_roll {
            world.write_resource::<crate::scripting::ScriptEventQueue>()
                .push(crate::scripting::ScriptEvent::AttackFail {
                    attacker: source,
                    victim: target,
                });
            let target_name = Self::get_entity_name(world, target);
            log::info!("🌀 {} 閃避攻擊（miss={:.2}, evasion={:.2}）", target_name, miss_chance, evasion);
            return next_outcomes;
        }

        // ---- 套 UnitStats::apply_incoming_damage 逐類型減免 ----
        // CProperty 的 def_physic 當 armor；def_magic 當 magic_resist (0..1)。
        let (final_phys, final_magi, final_real) = {
            let buffs = world.read_resource::<crate::ability_runtime::BuffStore>();
            let is_bldgs = world.read_storage::<IsBuilding>();
            let cps = world.read_storage::<CProperty>();
            let tgt_stats = UnitStats::from_refs(&*buffs, is_bldgs.get(target).is_some());
            let (base_armor, base_resist) = cps.get(target)
                .map(|cp| (cp.def_physic, cp.def_magic))
                .unwrap_or((0.0, 0.0));
            (
                tgt_stats.apply_incoming_damage(phys, DamageKind::Physical, target, base_armor, base_resist),
                tgt_stats.apply_incoming_damage(magi, DamageKind::Magical, target, base_armor, base_resist),
                tgt_stats.apply_incoming_damage(real, DamageKind::Pure, target, base_armor, base_resist),
            )
        };
        let final_total = final_phys + final_magi + final_real;

        // ---- 先發 AttackHit / AttackLanded（含 final damage 數值）----
        {
            let mut queue = world.write_resource::<crate::scripting::ScriptEventQueue>();
            queue.push(crate::scripting::ScriptEvent::AttackHit {
                attacker: source,
                victim: target,
            });
            queue.push(crate::scripting::ScriptEvent::AttackLanded {
                attacker: source,
                victim: target,
                damage: final_total,
            });
        }

        // ---- 扣 HP，套 MIN_HEALTH 下限 ----
        let min_health = {
            let buffs = world.read_resource::<crate::ability_runtime::BuffStore>();
            buffs.sum_add(target, StatKey::MinHealth)
        };
        let has_reincarnation = {
            let buffs = world.read_resource::<crate::ability_runtime::BuffStore>();
            buffs.has(target, StatKey::Reincarnation.as_str())
        };

        let mut died = false;
        {
            let mut properties = world.write_storage::<CProperty>();
            if let Some(target_props) = properties.get_mut(target) {
                let hp_before = target_props.hp;
                target_props.hp -= final_total;
                // MIN_HEALTH clamp：> 0 時 HP 不低於此值
                if min_health > 0.0 && target_props.hp < min_health {
                    target_props.hp = min_health;
                }
                if target_props.hp <= 0.0 {
                    target_props.hp = 0.0;
                    died = true;
                }
                let hp_after = target_props.hp;

                let (source_name, target_name) = Self::get_entity_names(world, source, target);
                let damage_info = Self::format_damage_info(final_phys, final_magi, final_real, final_total);
                log::info!("⚔️ {} 攻擊 {} | {} | HP: {:.1} → {:.1}/{:.1}",
                    source_name, target_name, damage_info, hp_before, hp_after, target_props.mhp
                );
            }
        }

        // ---- 死亡處理：reincarnation 優先 ----
        if died {
            if has_reincarnation {
                // 補滿 HP，push Respawn；不發 Death
                {
                    let mut properties = world.write_storage::<CProperty>();
                    if let Some(p) = properties.get_mut(target) {
                        p.hp = p.mhp;
                    }
                }
                // 移除 reincarnation（一次性）
                {
                    let mut buffs = world.write_resource::<crate::ability_runtime::BuffStore>();
                    buffs.remove(target, StatKey::Reincarnation.as_str());
                }
                let target_name = Self::get_entity_name(world, target);
                log::info!("✨ {} 重生！", target_name);
                world.write_resource::<crate::scripting::ScriptEventQueue>()
                    .push(crate::scripting::ScriptEvent::Respawn { e: target });
            } else {
                let target_name = Self::get_entity_name(world, target);
                log::info!("💀 {} 死亡！", target_name);
                next_outcomes.push(Outcome::Death { pos, ent: target });
                world.write_resource::<crate::scripting::ScriptEventQueue>()
                    .push(crate::scripting::ScriptEvent::Death {
                        victim: target,
                        killer: Some(source),
                    });
            }
        }

        next_outcomes
    }

    /// 處理治療事件 — 套 `HEAL_RECEIVED_MULTIPLIER` 與 `DISABLE_HEALING` 判定。
    pub fn handle_heal(
        world: &mut World,
        _mqtx: &Sender<OutboundMsg>,
        _pos: vek::Vec2<f32>,
        target: Entity,
        amount: f32,
    ) -> Vec<Outcome> {
        // 先查 buff 套 modifier
        let effective_amount = {
            let buffs = world.read_resource::<crate::ability_runtime::BuffStore>();
            let disabled = buffs.has(target, StatKey::DisableHealing.as_str())
                || buffs.sum_add(target, StatKey::DisableHealing) > 0.5;
            if disabled {
                0.0
            } else {
                let mult = 1.0 + buffs.sum_add(target, StatKey::HealReceivedMultiplier);
                amount * mult
            }
        };

        if effective_amount <= 0.0 {
            let target_name = Self::get_entity_name(world, target);
            log::info!("🚫 {} 治療被阻擋（disable_healing 或倍率歸零）", target_name);
            return Vec::new();
        }

        let mut actual_heal: f32 = 0.0;
        {
            let mut properties = world.write_storage::<CProperty>();
            if let Some(target_props) = properties.get_mut(target) {
                let hp_before = target_props.hp;
                target_props.hp = (target_props.hp + effective_amount).min(target_props.mhp);
                let hp_after = target_props.hp;
                actual_heal = hp_after - hp_before;

                let target_name = Self::get_entity_name(world, target);
                log::info!("💚 {} 回復 {:.1} HP（原 {:.1} × 倍率）| HP: {:.1} → {:.1}/{:.1}",
                    target_name, actual_heal, amount, hp_before, hp_after, target_props.mhp
                );
            }
        }

        if actual_heal > 0.0 {
            let mut queue = world.write_resource::<crate::scripting::ScriptEventQueue>();
            queue.push(crate::scripting::ScriptEvent::HealReceived {
                target,
                amount: actual_heal,
                source: None,
            });
            queue.push(crate::scripting::ScriptEvent::HealthGained {
                e: target,
                amount: actual_heal,
            });
        }

        Vec::new()
    }

    /// 處理死亡事件
    pub fn handle_death(
        world: &mut World,
        mqtx: &Sender<OutboundMsg>,
        _pos: vek::Vec2<f32>,
        entity: Entity,
    ) -> Vec<Outcome> {
        let mut next_outcomes = Vec::new();
        let mut creeps = world.write_storage::<Creep>();
        let mut towers = world.write_storage::<Tower>();
        let mut projs = world.write_storage::<Projectile>();
        
        let entity_type = if let Some(c) = creeps.get_mut(entity) {
            // 處理小兵死亡
            if let Some(bt) = c.block_tower {
                if let Some(t) = towers.get_mut(bt) { 
                    t.block_creeps.retain(|&x| x != entity);
                }
            }
            "creep"
        } else if let Some(t) = towers.get_mut(entity) {
            // 處理塔死亡
            for ce in t.block_creeps.iter() {
                if let Some(c) = creeps.get_mut(*ce) { 
                    c.block_tower = None;
                    next_outcomes.push(Outcome::CreepWalk { target: ce.clone() });
                }
            }
            "tower"
        } else if let Some(_p) = projs.get_mut(entity) {
            "projectile"
        } else { 
            "unknown"
        };
        
        if !entity_type.is_empty() && entity_type != "unknown" {
            #[cfg(feature = "kcp")]
            let msg = {
                use crate::state::resource_management::proto_build;
                use crate::transport::TypedOutbound;
                OutboundMsg::new_typed(
                    "td/all/res", entity_type, "D",
                    TypedOutbound::EntityDeath(proto_build::entity_death(entity.id())),
                    json!({ "id": entity.id() }),
                )
            };
            #[cfg(not(feature = "kcp"))]
            let msg = OutboundMsg::new_s("td/all/res", entity_type, "D", json!({"id": entity.id()}));
            let _ = mqtx.send(msg);
        }
        
        next_outcomes
    }

    /// 處理經驗獲得事件
    pub fn handle_experience_gain(
        world: &mut World,
        _mqtx: &Sender<OutboundMsg>,
        target: Entity,
        amount: u32,
    ) -> Vec<Outcome> {
        let mut heroes = world.write_storage::<Hero>();
        
        if let Some(hero) = heroes.get_mut(target) {
            let leveled_up = hero.add_experience(amount as i32);
            if leveled_up {
                log::info!("🌟 英雄 '{}' 獲得 {} 經驗並升級！", hero.name, amount);
            } else {
                log::info!("✨ 英雄 '{}' 獲得 {} 經驗", hero.name, amount);
            }
        }
        
        Vec::new()
    }

    /// 處理攻擊更新事件
    pub fn handle_attack_update(
        world: &mut World,
        _mqtx: &Sender<OutboundMsg>,
        target: Entity,
        asd_count: Option<f32>,
        cooldown_reset: bool,
    ) -> Vec<Outcome> {
        let mut attacks = world.write_storage::<TAttack>();
        
        if let Some(attack) = attacks.get_mut(target) {
            if let Some(new_count) = asd_count {
                attack.asd_count = new_count;
            }
            if cooldown_reset {
                attack.asd_count = attack.asd.v;
            }
        }
        
        Vec::new()
    }

    // 輔助方法
    fn get_entity_names(world: &World, source: Entity, target: Entity) -> (String, String) {
        let source_name = Self::get_entity_name(world, source);
        let target_name = Self::get_entity_name(world, target);
        (source_name, target_name)
    }

    fn get_entity_name(world: &World, entity: Entity) -> String {
        let creeps = world.read_storage::<Creep>();
        let heroes = world.read_storage::<Hero>();
        let units = world.read_storage::<Unit>();
        
        if let Some(creep) = creeps.get(entity) {
            creep.name.clone()
        } else if let Some(hero) = heroes.get(entity) {
            hero.name.clone()
        } else if let Some(unit) = units.get(entity) {
            unit.name.clone()
        } else {
            "Unknown".to_string()
        }
    }

    fn format_damage_info(phys: f32, magi: f32, real: f32, total: f32) -> String {
        let mut parts = Vec::new();
        if phys > 0.0 { parts.push(format!("物理 {:.1}", phys)); }
        if magi > 0.0 { parts.push(format!("魔法 {:.1}", magi)); }
        if real > 0.0 { parts.push(format!("真實 {:.1}", real)); }
        if parts.is_empty() { 
            parts.push(format!("總共 {:.1}", total)); 
        }
        parts.join(", ")
    }
}