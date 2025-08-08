use std::collections::BTreeMap;
use failure::Error;
use serde_json::json;
use specs::{World, WorldExt, Entity, Builder, storage::{WriteStorage, ReadStorage}};

use crate::comp::*;
use crate::msg::MqttMsg;
use crate::Outcome;
use crate::Projectile;

pub struct GameProcessor;

impl GameProcessor {
    pub fn process_outcomes(ecs: &mut World, mqtx: &crossbeam_channel::Sender<MqttMsg>) -> Result<(), Error> {
        let mut remove_uids = vec![];
        let mut next_outcomes = vec![];
        
        {
            let mut ocs = ecs.get_mut::<Vec<Outcome>>().unwrap();
            let mut outcomes = vec![];
            outcomes.append(ocs);
            
            for out in outcomes {
                match out {
                    Outcome::Death { pos: p, ent: e } => {
                        remove_uids.push(e);
                        Self::handle_death(ecs, &mut next_outcomes, mqtx, e)?;
                    }
                    Outcome::ProjectileLine2 { pos, source, target } => {
                        Self::handle_projectile(ecs, mqtx, pos, source, target)?;
                    }
                    Outcome::Creep { cd } => {
                        Self::handle_creep_spawn(ecs, mqtx, cd)?;
                    }
                    Outcome::Tower { pos, td } => {
                        Self::handle_tower_spawn(ecs, mqtx, pos, td)?;
                    }
                    Outcome::CreepStop { source, target } => {
                        Self::handle_creep_stop(ecs, mqtx, source, target)?;
                    }
                    Outcome::CreepWalk { target } => {
                        Self::handle_creep_walk(ecs, target)?;
                    }
                    Outcome::Damage { pos, phys, magi, real, source, target } => {
                        Self::handle_damage(ecs, &mut next_outcomes, pos, phys, magi, real, source, target)?;
                    }
                    Outcome::Heal { pos, target, amount } => {
                        Self::handle_heal(ecs, target, amount)?;
                    }
                    Outcome::UpdateAttack { target, asd_count, cooldown_reset } => {
                        Self::handle_attack_update(ecs, target, asd_count, cooldown_reset)?;
                    }
                    Outcome::GainExperience { target, amount } => {
                        Self::handle_experience_gain(ecs, target, amount as u32)?;
                    }
                    _ => {}
                }
            }
        }
        
        ecs.delete_entities(&remove_uids[..]);
        ecs.write_resource::<Vec<Outcome>>().clear();
        ecs.write_resource::<Vec<Outcome>>().append(&mut next_outcomes);
        Ok(())
    }
    
    fn handle_death(
        ecs: &mut World, 
        next_outcomes: &mut Vec<Outcome>, 
        mqtx: &crossbeam_channel::Sender<MqttMsg>, 
        entity: Entity
    ) -> Result<(), Error> {
        let mut creeps = ecs.write_storage::<Creep>();
        let mut towers = ecs.write_storage::<Tower>();
        let mut projs = ecs.write_storage::<Projectile>();
        
        let entity_type = if let Some(c) = creeps.get_mut(entity) {
            if let Some(bt) = c.block_tower {
                if let Some(t) = towers.get_mut(bt) { 
                    t.block_creeps.retain(|&x| x != entity);
                }
            }
            "creep"
        } else if let Some(t) = towers.get_mut(entity) {
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
            ""
        };
        
        if !entity_type.is_empty() {
            mqtx.send(MqttMsg::new_s("td/all/res", entity_type, "D", json!({"id": entity.id()})));
        }
        Ok(())
    }
    
    fn handle_projectile(
        ecs: &mut World, 
        mqtx: &crossbeam_channel::Sender<MqttMsg>, 
        pos: vek::Vec2<f32>, 
        source: Option<Entity>, 
        target: Option<Entity>
    ) -> Result<(), Error> {
        let source_entity = source.ok_or_else(|| failure::err_msg("Missing source entity"))?;
        let target_entity = target.ok_or_else(|| failure::err_msg("Missing target entity"))?;
        
        let (msd, p2) = {
            let positions = ecs.read_storage::<Pos>();
            let tproperty = ecs.read_storage::<TAttack>();
            
            let _p1 = positions.get(source_entity).ok_or_else(|| failure::err_msg("Source position not found"))?;
            let p2 = positions.get(target_entity).ok_or_else(|| failure::err_msg("Target position not found"))?;
            let tp = tproperty.get(source_entity).ok_or_else(|| failure::err_msg("Source attack properties not found"))?;
            (tp.bullet_speed, p2.0)
        };
        
        let ntarget = target_entity.id();
        let e = ecs.create_entity()
            .with(Pos(pos))
            .with(Projectile { 
                time_left: 3., 
                owner: source_entity.clone(), 
                tpos: p2, 
                target: target, 
                radius: 0., 
                msd: msd,
                damage_phys: 25.0, // 預設物理傷害
                damage_magi: 0.0,  // 預設魔法傷害 
                damage_real: 0.0   // 預設真實傷害
            })
            .build();
            
        let pjs = json!(ProjectileData {
            id: e.id(), 
            pos: pos.clone(), 
            msd: msd,
            time_left: 3., 
            owner: source_entity.id(), 
            target: ntarget, 
            radius: 0.,
        });
        
        mqtx.try_send(MqttMsg::new_s("td/all/res", "projectile", "C", pjs));
        Ok(())
    }
    
    fn handle_creep_spawn(ecs: &mut World, mqtx: &crossbeam_channel::Sender<MqttMsg>, cd: CreepData) -> Result<(), Error> {
        let mut cjs = json!(cd);
        let e = ecs.create_entity().with(Pos(cd.pos)).with(cd.creep).with(cd.cdata).build();
        cjs.as_object_mut().unwrap().insert("id".to_owned(), json!(e.id()));
        mqtx.try_send(MqttMsg::new_s("td/all/res", "creep", "C", cjs));
        Ok(())
    }
    
    fn handle_tower_spawn(ecs: &mut World, mqtx: &crossbeam_channel::Sender<MqttMsg>, pos: vek::Vec2<f32>, td: TowerData) -> Result<(), Error> {
        let mut cjs = json!(td);
        let e = ecs.create_entity().with(Pos(pos)).with(Tower::new()).with(td.tpty).with(td.tatk).build();
        cjs.as_object_mut().unwrap().insert("id".to_owned(), json!(e.id()));
        cjs.as_object_mut().unwrap().insert("pos".to_owned(), json!(pos));
        mqtx.try_send(MqttMsg::new_s("td/all/res", "tower", "C", cjs));
        ecs.get_mut::<Searcher>().unwrap().tower.needsort = true;
        Ok(())
    }
    
    fn handle_creep_stop(ecs: &mut World, mqtx: &crossbeam_channel::Sender<MqttMsg>, source: Entity, target: Entity) -> Result<(), Error> {
        let mut creeps = ecs.write_storage::<Creep>();
        let c = creeps.get_mut(target).ok_or_else(|| failure::err_msg("Creep not found"))?;
        c.block_tower = Some(source);
        c.status = CreepStatus::Stop;
        
        let positions = ecs.read_storage::<Pos>();
        let pos = positions.get(target).ok_or_else(|| failure::err_msg("Creep position not found"))?;
        
        mqtx.try_send(MqttMsg::new_s("td/all/res", "creep", "M", json!({
            "id": target.id(),
            "x": pos.0.x,
            "y": pos.0.y,
        })));
        Ok(())
    }
    
    fn handle_creep_walk(ecs: &mut World, target: Entity) -> Result<(), Error> {
        let mut creeps = ecs.write_storage::<Creep>();
        let creep = creeps.get_mut(target).ok_or_else(|| failure::err_msg("Creep not found"))?;
        creep.status = CreepStatus::PreWalk;
        Ok(())
    }
    
    fn handle_damage(
        ecs: &mut World, 
        next_outcomes: &mut Vec<Outcome>, 
        pos: vek::Vec2<f32>,
        phys: f32, 
        magi: f32, 
        real: f32, 
        source: Entity, 
        target: Entity
    ) -> Result<(), Error> {
        let mut properties = ecs.write_storage::<CProperty>();
        if let Some(target_props) = properties.get_mut(target) {
            let hp_before = target_props.hp;
            let total_damage = phys + magi + real;
            target_props.hp -= total_damage;
            let hp_after = target_props.hp;
            
            let (source_name, target_name) = Self::get_entity_names(ecs, source, target);
            
            let damage_parts = {
                let mut parts = Vec::new();
                if phys > 0.0 { parts.push(format!("Phys {:.1}", phys)); }
                if magi > 0.0 { parts.push(format!("Magi {:.1}", magi)); }
                if real > 0.0 { parts.push(format!("Pure {:.1}", real)); }
                if parts.is_empty() { 
                    parts.push(format!("Total {:.1}", total_damage)); 
                }
                parts.join(", ")
            };
            
            log::info!("⚔️ {} 攻擊 {} | {} damage | HP: {:.1} → {:.1}/{:.1}", 
                source_name, target_name, damage_parts, hp_before, hp_after, target_props.mhp
            );
            
            if target_props.hp <= 0.0 {
                target_props.hp = 0.0;
                log::info!("💀 {} died from damage!", target_name);
                next_outcomes.push(Outcome::Death { 
                    pos: pos,
                    ent: target 
                });
            }
        }
        Ok(())
    }
    
    fn handle_heal(ecs: &mut World, target: Entity, amount: f32) -> Result<(), Error> {
        let mut properties = ecs.write_storage::<CProperty>();
        if let Some(target_props) = properties.get_mut(target) {
            target_props.hp = (target_props.hp + amount).min(target_props.mhp);
        }
        Ok(())
    }
    
    fn handle_attack_update(ecs: &mut World, target: Entity, asd_count: Option<f32>, cooldown_reset: bool) -> Result<(), Error> {
        let mut attacks = ecs.write_storage::<TAttack>();
        if let Some(attack) = attacks.get_mut(target) {
            if let Some(new_count) = asd_count {
                attack.asd_count = new_count;
            }
            if cooldown_reset {
                attack.asd_count = attack.asd.v;
            }
        }
        Ok(())
    }
    
    fn handle_experience_gain(ecs: &mut World, target: Entity, amount: u32) -> Result<(), Error> {
        let mut heroes = ecs.write_storage::<Hero>();
        if let Some(hero) = heroes.get_mut(target) {
            let leveled_up = hero.add_experience(amount as i32);
            if leveled_up {
                log::info!("Hero '{}' gained {} experience and leveled up!", hero.name, amount);
            } else {
                log::info!("Hero '{}' gained {} experience", hero.name, amount);
            }
        }
        Ok(())
    }
    
    fn get_entity_names(ecs: &World, source: Entity, target: Entity) -> (String, String) {
        let creeps = ecs.read_storage::<Creep>();
        let heroes = ecs.read_storage::<Hero>();
        let units = ecs.read_storage::<Unit>();
        
        let source_name = if let Some(creep) = creeps.get(source) {
            creep.name.clone()
        } else if let Some(hero) = heroes.get(source) {
            hero.name.clone()
        } else if let Some(unit) = units.get(source) {
            unit.name.clone()
        } else {
            "Unknown".to_string()
        };
        
        let target_name = if let Some(creep) = creeps.get(target) {
            creep.name.clone()
        } else if let Some(hero) = heroes.get(target) {
            hero.name.clone()
        } else if let Some(unit) = units.get(target) {
            unit.name.clone()
        } else {
            "Unknown".to_string()
        };
        
        (source_name, target_name)
    }
}