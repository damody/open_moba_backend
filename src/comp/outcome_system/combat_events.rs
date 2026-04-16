/// 戰鬥相關事件處理

use specs::{Entity, World, WriteStorage, ReadStorage, WorldExt};
use crate::comp::*;
use crate::transport::OutboundMsg;
use crossbeam_channel::Sender;
use serde_json::json;

/// 戰鬥事件處理器
pub struct CombatEventHandler;

impl CombatEventHandler {
    /// 處理傷害事件
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
        let mut properties = world.write_storage::<CProperty>();
        
        if let Some(target_props) = properties.get_mut(target) {
            let hp_before = target_props.hp;
            let total_damage = phys + magi + real;
            target_props.hp -= total_damage;
            let hp_after = target_props.hp;
            
            // 獲取攻擊者和目標名稱
            let (source_name, target_name) = Self::get_entity_names(world, source, target);
            
            // 格式化傷害信息
            let damage_info = Self::format_damage_info(phys, magi, real, total_damage);
            
            log::info!("⚔️ {} 攻擊 {} | {} | HP: {:.1} → {:.1}/{:.1}", 
                source_name, target_name, damage_info, hp_before, hp_after, target_props.mhp
            );
            
            // 檢查是否死亡
            if target_props.hp <= 0.0 {
                target_props.hp = 0.0;
                log::info!("💀 {} 死亡！", target_name);
                next_outcomes.push(Outcome::Death { pos, ent: target });
            }
        }
        
        next_outcomes
    }

    /// 處理治療事件
    pub fn handle_heal(
        world: &mut World,
        _mqtx: &Sender<OutboundMsg>,
        _pos: vek::Vec2<f32>,
        target: Entity,
        amount: f32,
    ) -> Vec<Outcome> {
        let mut properties = world.write_storage::<CProperty>();
        
        if let Some(target_props) = properties.get_mut(target) {
            let hp_before = target_props.hp;
            target_props.hp = (target_props.hp + amount).min(target_props.mhp);
            let hp_after = target_props.hp;
            
            let target_name = Self::get_entity_name(world, target);
            log::info!("💚 {} 回復 {:.1} HP | HP: {:.1} → {:.1}/{:.1}", 
                target_name, amount, hp_before, hp_after, target_props.mhp
            );
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
            let _ = mqtx.send(OutboundMsg::new_s("td/all/res", entity_type, "D", json!({"id": entity.id()})));
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