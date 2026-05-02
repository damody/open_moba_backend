/// 戰鬥相關事件處理

use specs::{Entity, World, WriteStorage, ReadStorage, WorldExt};
use crate::ability_runtime::UnitStats;
use crate::comp::*;
use crate::transport::OutboundMsg;
use crossbeam_channel::Sender;
use omb_script_abi::stat_keys::StatKey;
use omb_script_abi::types::DamageKind;
use serde_json::json;

/// Per-entity SimRng op_kind for combat_events. Phase 1de.2: replaces fastrand
/// for the miss/evasion roll. Reordering or reusing this constant across systems
/// would invalidate replay determinism.
const OP_COMBAT_MISS_ROLL: u32 = 30;

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
        pos: omoba_sim::Vec2,
        phys: omoba_sim::Fixed64,
        magi: omoba_sim::Fixed64,
        real: omoba_sim::Fixed64,
        source: Entity,
        target: Entity,
    ) -> Vec<Outcome> {
        // convert at boundary; redesign in Phase 2 KCP tag rework.
        let phys_f = phys.to_f32_for_render();
        let magi_f = magi.to_f32_for_render();
        let real_f = real.to_f32_for_render();
        let mut next_outcomes = Vec::new();

        // ---- 先發 Attacked 給 victim 側（命中與否都發）----
        world.write_resource::<crate::scripting::ScriptEventQueue>()
            .push(crate::scripting::ScriptEvent::Attacked {
                attacker: source,
                victim: target,
            });

        // ---- Evasion / miss roll（基於 target 的閃避 + attacker 的 miss）----
        // Phase 1de.2: deterministic per-(target, OP_COMBAT_MISS_ROLL) stream.
        // The roll keys on `target.id()` so the victim-side state controls the
        // outcome; sub-tick attack ordering won't shuffle which roll a given
        // attack consumes.
        let (miss_chance, evasion): (f32, f32) = {
            let buffs = world.read_resource::<crate::ability_runtime::BuffStore>();
            let is_bldgs = world.read_storage::<IsBuilding>();
            let tgt_stats = UnitStats::from_refs(&*buffs, is_bldgs.get(target).is_some());
            let src_stats = UnitStats::from_refs(&*buffs, is_bldgs.get(source).is_some());
            (
                src_stats.miss_chance(source).to_f32_for_render(),
                tgt_stats.evasion_chance(target).to_f32_for_render(),
            )
        };
        let miss_roll = 1.0 - (1.0 - miss_chance) * (1.0 - evasion);
        let miss_triggered = if miss_roll > 0.0 {
            let master_seed: u64 = world.read_resource::<MasterSeed>().0;
            let tick: u32 = world.read_resource::<Tick>().0 as u32;
            let mut rng = omoba_sim::SimRng::from_master_entity(
                master_seed, tick, target.id(), OP_COMBAT_MISS_ROLL,
            );
            let roll = rng.gen_fixed64_unit().to_f32_for_render();
            roll < miss_roll
        } else {
            false
        };
        if miss_triggered {
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
                .map(|cp| (cp.def_physic.to_f32_for_render(), cp.def_magic.to_f32_for_render()))
                .unwrap_or((0.0, 0.0));
            (
                tgt_stats.apply_incoming_damage(phys_f, DamageKind::Physical, target, base_armor, base_resist),
                tgt_stats.apply_incoming_damage(magi_f, DamageKind::Magical, target, base_armor, base_resist),
                tgt_stats.apply_incoming_damage(real_f, DamageKind::Pure, target, base_armor, base_resist),
            )
        };
        let final_total = final_phys + final_magi + final_real;

        // ---- 先發 AttackHit / AttackLanded（含 final damage 數值）----
        // Phase 1c.3: AttackLanded.damage is Fixed64 — convert at boundary.
        let final_total_fx = omoba_sim::Fixed64::from_raw((final_total * 1024.0) as i64);
        {
            let mut queue = world.write_resource::<crate::scripting::ScriptEventQueue>();
            queue.push(crate::scripting::ScriptEvent::AttackHit {
                attacker: source,
                victim: target,
            });
            queue.push(crate::scripting::ScriptEvent::AttackLanded {
                attacker: source,
                victim: target,
                damage: final_total_fx,
            });
        }

        // ---- 扣 HP，套 MIN_HEALTH 下限 ----
        let min_health: f32 = {
            let buffs = world.read_resource::<crate::ability_runtime::BuffStore>();
            buffs.sum_add(target, StatKey::MinHealth).to_f32_for_render()
        };
        let has_reincarnation = {
            let buffs = world.read_resource::<crate::ability_runtime::BuffStore>();
            buffs.has(target, StatKey::Reincarnation.as_str())
        };

        let mut died = false;
        {
            let mut properties = world.write_storage::<CProperty>();
            if let Some(target_props) = properties.get_mut(target) {
                // Phase 1c.3: target_props.hp / mhp are Fixed64 (Phase 1c.2);
                // final_total / min_health stay f32 here — boundary at the read.
                let hp_before_f = target_props.hp.to_f32_for_render();
                let mut hp_after_f = hp_before_f - final_total;
                // MIN_HEALTH clamp：> 0 時 HP 不低於此值
                if min_health > 0.0 && hp_after_f < min_health {
                    hp_after_f = min_health;
                }
                if hp_after_f <= 0.0 {
                    hp_after_f = 0.0;
                    died = true;
                }
                target_props.hp = omoba_sim::Fixed64::from_raw((hp_after_f * 1024.0) as i64);

                let (source_name, target_name) = Self::get_entity_names(world, source, target);
                let damage_info = Self::format_damage_info(final_phys, final_magi, final_real, final_total);
                log::info!("⚔️ {} 攻擊 {} | {} | HP: {:.1} → {:.1}/{:.1}",
                    source_name, target_name, damage_info, hp_before_f, hp_after_f,
                    target_props.mhp.to_f32_for_render()
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
                next_outcomes.push(Outcome::Death { pos, ent: target });  // pos is SimVec2 (post Phase 1c.2)
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
        _pos: omoba_sim::Vec2,
        target: Entity,
        amount: omoba_sim::Fixed64,
    ) -> Vec<Outcome> {
        use omoba_sim::Fixed64;
        // 先查 buff 套 modifier
        let half = Fixed64::from_raw(512); // 0.5
        let effective_amount: Fixed64 = {
            let buffs = world.read_resource::<crate::ability_runtime::BuffStore>();
            let disabled = buffs.has(target, StatKey::DisableHealing.as_str())
                || buffs.sum_add(target, StatKey::DisableHealing) > half;
            if disabled {
                Fixed64::ZERO
            } else {
                let mult = Fixed64::ONE + buffs.sum_add(target, StatKey::HealReceivedMultiplier);
                amount * mult
            }
        };

        if effective_amount <= Fixed64::ZERO {
            let target_name = Self::get_entity_name(world, target);
            log::info!("🚫 {} 治療被阻擋（disable_healing 或倍率歸零）", target_name);
            return Vec::new();
        }

        let mut actual_heal: Fixed64 = Fixed64::ZERO;
        {
            let mut properties = world.write_storage::<CProperty>();
            if let Some(target_props) = properties.get_mut(target) {
                let hp_before = target_props.hp;
                let summed = target_props.hp + effective_amount;
                target_props.hp = if summed > target_props.mhp { target_props.mhp } else { summed };
                let hp_after = target_props.hp;
                actual_heal = hp_after - hp_before;

                let target_name = Self::get_entity_name(world, target);
                // NOTE: log uses f32 boundary — Fixed64 has no Display.
                log::info!("💚 {} 回復 {:.1} HP（原 {:.1} × 倍率）| HP: {:.1} → {:.1}/{:.1}",
                    target_name,
                    actual_heal.to_f32_for_render(),
                    amount.to_f32_for_render(),
                    hp_before.to_f32_for_render(),
                    hp_after.to_f32_for_render(),
                    target_props.mhp.to_f32_for_render()
                );
            }
        }

        if actual_heal > Fixed64::ZERO {
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
        _pos: omoba_sim::Vec2,
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
                // P9: stamp EntityKind for shim ("hero"/"creep"/"unit"/"tower"/"projectile", "D")
                let entity_kind = match entity_type {
                    "hero" => proto_build::EntityKind::Hero,
                    "unit" => proto_build::EntityKind::Unit,
                    "tower" => proto_build::EntityKind::Tower,
                    "creep" => proto_build::EntityKind::Creep,
                    "projectile" => proto_build::EntityKind::Projectile,
                    _ => proto_build::EntityKind::Entity,
                };
                OutboundMsg::new_typed(
                    "td/all/res", entity_type, "D",
                    TypedOutbound::EntityDeath(proto_build::entity_death_with_kind(entity.id(), entity_kind)),
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
        asd_count: Option<omoba_sim::Fixed64>,
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