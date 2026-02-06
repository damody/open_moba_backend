/// 召喚物AI系統
/// 
/// 處理召喚物的行為邏輯，包括：
/// - 目標搜尋和選擇
/// - 攻擊行為
/// - 移動和追擊
/// - 生命週期管理

use specs::{
    shred, Entities, Join, LazyUpdate, Read, ReadExpect, ReadStorage, SystemData,
    Write, WriteStorage, ParJoin, Entity, World,
};
use crate::comp::*;
use log::{info, warn, debug};
use vek::Vec2;

#[derive(SystemData)]
pub struct SummonRead<'a> {
    entities: Entities<'a>,
    time: Read<'a, Time>,
    dt: Read<'a, DeltaTime>,
    units: ReadStorage<'a, Unit>,
    summoned_units: ReadStorage<'a, SummonedUnit>,
    positions: ReadStorage<'a, Pos>,
    factions: ReadStorage<'a, Faction>,
    properties: ReadStorage<'a, CProperty>,
    attacks: ReadStorage<'a, TAttack>,
    searcher: Read<'a, Searcher>,
}

#[derive(SystemData)]
pub struct SummonWrite<'a> {
    outcomes: Write<'a, Vec<Outcome>>,
    summoned_units: WriteStorage<'a, SummonedUnit>,
    units: WriteStorage<'a, Unit>,
    positions: WriteStorage<'a, Pos>,
    damage_instances: Write<'a, Vec<DamageInstance>>,
}

pub struct Sys;

impl Default for Sys {
    fn default() -> Self {
        Self
    }
}

impl<'a> crate::comp::ecs::System<'a> for Sys {
    type SystemData = (
        SummonRead<'a>,
        SummonWrite<'a>,
    );

    const NAME: &'static str = "summon";

    fn run(job: &mut crate::comp::ecs::Job<Self>, (tr, mut tw): Self::SystemData) {
        let time = tr.time.0;
        let dt = tr.dt.0;
        
        // 處理召喚物生命週期
        let mut expired_summons = Vec::new();
        
        for (entity, mut summoned_unit) in (&tr.entities, &mut tw.summoned_units).join() {
            if summoned_unit.update(dt) {
                // 召喚物過期
                expired_summons.push(entity);
            }
        }
        
        // 移除過期的召喚物
        for entity in expired_summons {
            if let Some(unit) = tr.units.get(entity) {
                if let Some(pos) = tr.positions.get(entity) {
                    info!("召喚物 {} 時間到期消失", unit.name);
                    tw.outcomes.push(Outcome::Death {
                        pos: pos.0,
                        ent: entity,
                    });
                }
            }
        }
        
        // 處理召喚物AI行為
        process_summon_ai(&tr, &mut tw, time, dt);
    }
}

/// 處理召喚物AI行為
fn process_summon_ai(
    tr: &SummonRead,
    tw: &mut SummonWrite,
    time: f64,
    dt: f32,
) {
    for (entity, unit, pos, faction) in (&tr.entities, &tw.units, &tr.positions, &tr.factions).join() {
        // 只處理召喚物
        if unit.unit_type != UnitType::Summon {
            continue;
        }
        
        // 根據AI類型執行不同行為
        match unit.ai_type {
            AiType::Aggressive => {
                process_aggressive_ai(entity, unit, pos, faction, tr, tw, time);
            },
            AiType::Defensive => {
                process_defensive_ai(entity, unit, pos, faction, tr, tw, time);
            },
            AiType::Patrol => {
                process_patrol_ai(entity, unit, pos, faction, tr, tw, time);
            },
            AiType::Guard => {
                process_guard_ai(entity, unit, pos, faction, tr, tw, time);
            },
            _ => {} // Passive 或 None 不做任何事
        }
    }
}

/// 主動攻擊AI
fn process_aggressive_ai(
    entity: Entity,
    unit: &Unit,
    pos: &Pos,
    faction: &Faction,
    tr: &SummonRead,
    tw: &mut SummonWrite,
    time: f64,
) {
    // 搜尋範圍內的敵人
    let enemies = search_enemies(pos.0, unit.aggro_range, faction, tr);
    
    if let Some(target) = find_best_target(&enemies, pos.0, unit) {
        // 檢查是否在攻擊範圍內
        if let Some(target_pos) = tr.positions.get(target) {
            let distance = pos.0.distance(target_pos.0);
            
            if distance <= unit.attack_range {
                // 在攻擊範圍內，執行攻擊
                if unit.can_attack(time as f32) {
                    execute_attack(entity, target, unit, tr, tw);
                    
                    // 更新攻擊時間
                    if let Some(mut summon_unit) = tw.units.get_mut(entity) {
                        summon_unit.last_attack_time = time as f32;
                    }
                }
            } else {
                // 移動到攻擊範圍內
                move_towards_target(entity, target_pos.0, pos.0, unit, tw);
            }
        }
    } else {
        // 沒有敵人，返回出生點
        return_to_spawn(entity, unit, pos, tw);
    }
}

/// 防守型AI
fn process_defensive_ai(
    entity: Entity,
    unit: &Unit,
    pos: &Pos,
    faction: &Faction,
    tr: &SummonRead,
    tw: &mut SummonWrite,
    time: f64,
) {
    // 只在較小範圍內搜尋敵人
    let defensive_range = unit.aggro_range * 0.7;
    let enemies = search_enemies(pos.0, defensive_range, faction, tr);
    
    if let Some(target) = find_best_target(&enemies, pos.0, unit) {
        if let Some(target_pos) = tr.positions.get(target) {
            let distance = pos.0.distance(target_pos.0);
            
            if distance <= unit.attack_range {
                if unit.can_attack(time as f32) {
                    execute_attack(entity, target, unit, tr, tw);
                    
                    if let Some(mut summon_unit) = tw.units.get_mut(entity) {
                        summon_unit.last_attack_time = time as f32;
                    }
                }
            } else if distance <= defensive_range {
                // 只在防守範圍內追擊
                move_towards_target(entity, target_pos.0, pos.0, unit, tw);
            }
        }
    }
    
    // 防守型單位不會主動離開太遠
}

/// 巡邏AI
fn process_patrol_ai(
    entity: Entity,
    unit: &Unit,
    pos: &Pos,
    faction: &Faction,
    tr: &SummonRead,
    tw: &mut SummonWrite,
    time: f64,
) {
    // 先檢查是否有敵人
    let enemies = search_enemies(pos.0, unit.aggro_range, faction, tr);
    
    if let Some(target) = find_best_target(&enemies, pos.0, unit) {
        // 有敵人時的行為類似主動攻擊
        process_aggressive_ai(entity, unit, pos, faction, tr, tw, time);
    } else {
        // 沒有敵人時進行巡邏
        patrol_movement(entity, unit, pos, tw, time);
    }
}

/// 守衛AI
fn process_guard_ai(
    entity: Entity,
    unit: &Unit,
    pos: &Pos,
    faction: &Faction,
    tr: &SummonRead,
    tw: &mut SummonWrite,
    time: f64,
) {
    // 守衛只在出生點附近活動
    let guard_range = 200.0;
    let spawn_pos = Vec2::new(unit.spawn_position.0, unit.spawn_position.1);
    let distance_from_spawn = pos.0.distance(spawn_pos);
    
    if distance_from_spawn > guard_range {
        // 離出生點太遠，返回
        move_towards_position(entity, spawn_pos, pos.0, unit, tw);
        return;
    }
    
    // 在守衛範圍內搜尋敵人
    let enemies = search_enemies(pos.0, unit.aggro_range.min(guard_range), faction, tr);
    
    if let Some(target) = find_best_target(&enemies, pos.0, unit) {
        if let Some(target_pos) = tr.positions.get(target) {
            let distance = pos.0.distance(target_pos.0);
            
            if distance <= unit.attack_range {
                if unit.can_attack(time as f32) {
                    execute_attack(entity, target, unit, tr, tw);
                    
                    if let Some(mut summon_unit) = tw.units.get_mut(entity) {
                        summon_unit.last_attack_time = time as f32;
                    }
                }
            } else if target_pos.0.distance(spawn_pos) <= guard_range {
                // 目標在守衛範圍內才追擊
                move_towards_target(entity, target_pos.0, pos.0, unit, tw);
            }
        }
    }
}

/// 搜尋敵人
fn search_enemies(
    position: Vec2<f32>,
    range: f32,
    own_faction: &Faction,
    tr: &SummonRead,
) -> Vec<Entity> {
    let mut enemies = Vec::new();
    
    // 使用空間搜尋系統找到範圍內的單位
    let creep_results = tr.searcher.creep.SearchNN_XY(position, range, 10);
    
    for result in creep_results {
        let entity = result.e;
        
        // 檢查陣營是否敵對
        if let Some(other_faction) = tr.factions.get(entity) {
            if own_faction.is_hostile_to(other_faction) {
                // 檢查是否還活著
                if let Some(props) = tr.properties.get(entity) {
                    if props.hp > 0.0 {
                        enemies.push(entity);
                    }
                }
            }
        }
    }
    
    enemies
}

/// 找到最佳目標
fn find_best_target(
    enemies: &[Entity],
    position: Vec2<f32>,
    unit: &Unit,
) -> Option<Entity> {
    if enemies.is_empty() {
        return None;
    }
    
    // 簡單策略：選擇最近的敵人
    enemies.first().copied()
}

/// 執行攻擊
fn execute_attack(
    attacker: Entity,
    target: Entity,
    unit: &Unit,
    tr: &SummonRead,
    tw: &mut SummonWrite,
) {
    // 創建傷害實例
    let damage_types = DamageTypes {
        physical: unit.base_damage as f32,
        magical: 0.0,
        pure: 0.0,
    };
    
    let damage_flags = DamageFlags {
        can_crit: false,
        can_dodge: true,
        ignore_armor: false,
        ignore_magic_resist: false,
        lifesteal: 0.0,
        spell_vamp: 0.0,
    };
    
    tw.damage_instances.push(DamageInstance {
        target,
        damage_types,
        is_critical: false,
        is_dodged: false,
        damage_flags,
        source: DamageSource {
            source_entity: attacker,
            source_type: DamageSourceType::Attack,
            ability_id: None,
        },
    });
    
    debug!("召喚物 {:?} 攻擊 {:?}，造成 {} 點物理傷害", 
           attacker, target, unit.base_damage);
}

/// 向目標移動
fn move_towards_target(
    entity: Entity,
    target_pos: Vec2<f32>,
    current_pos: Vec2<f32>,
    unit: &Unit,
    tw: &mut SummonWrite,
) {
    move_towards_position(entity, target_pos, current_pos, unit, tw);
}

/// 向指定位置移動
fn move_towards_position(
    entity: Entity,
    target_pos: Vec2<f32>,
    current_pos: Vec2<f32>,
    unit: &Unit,
    tw: &mut SummonWrite,
) {
    let direction = (target_pos - current_pos).normalized();
    let move_distance = unit.move_speed * 0.016; // 假設60fps
    let new_pos = current_pos + direction * move_distance;
    
    if let Some(mut pos) = tw.positions.get_mut(entity) {
        pos.0 = new_pos;
    }
}

/// 返回出生點
fn return_to_spawn(
    entity: Entity,
    unit: &Unit,
    current_pos: &Pos,
    tw: &mut SummonWrite,
) {
    let spawn_pos = Vec2::new(unit.spawn_position.0, unit.spawn_position.1);
    let distance = current_pos.0.distance(spawn_pos);
    
    if distance > 50.0 {
        move_towards_position(entity, spawn_pos, current_pos.0, unit, tw);
    }
}

/// 巡邏移動
fn patrol_movement(
    entity: Entity,
    unit: &Unit,
    current_pos: &Pos,
    tw: &mut SummonWrite,
    time: f64,
) {
    // 簡單的巡邏邏輯：圍繞出生點移動
    let spawn_pos = Vec2::new(unit.spawn_position.0, unit.spawn_position.1);
    let patrol_radius = 150.0;
    
    // 使用時間創建圓形巡邏路徑
    let angle = (time * 0.5) as f32; // 慢速旋轉
    let patrol_target = spawn_pos + Vec2::new(
        angle.cos() * patrol_radius,
        angle.sin() * patrol_radius,
    );
    
    move_towards_position(entity, patrol_target, current_pos.0, unit, tw);
}