/// ECS 狀態查詢模塊
/// 提供 read-only 的 ECS World 查詢，供 MCP server 使用

use std::collections::BTreeMap;
use specs::{World, WorldExt, Join, Entity};
use serde_json::json;

use crate::comp::*;
use crate::transport::QueryResponse;

/// 列出所有玩家及其英雄的基本資訊
pub fn query_list_players(world: &World) -> QueryResponse {
    let entities = world.entities();
    let heroes = world.read_storage::<Hero>();
    let positions = world.read_storage::<Pos>();
    let properties = world.read_storage::<CProperty>();
    let pmap = world.read_resource::<BTreeMap<String, Player>>();

    let mut player_list = Vec::new();

    let factions = world.read_storage::<Faction>();

    for (name, player) in pmap.iter() {
        // 找到此玩家對應的英雄（先按名稱匹配，再按 Player 陣營匹配）
        let hero_info: Option<serde_json::Value> = (&entities, &heroes, &positions)
            .join()
            .find(|(_, h, _)| h.name == *name || h.id == *name)
            .or_else(|| {
                (&entities, &heroes, &positions)
                    .join()
                    .find(|(ent, _, _)| {
                        factions.get(*ent)
                            .map(|f| f.faction_id == FactionType::Player)
                            .unwrap_or(false)
                    })
            })
            .map(|(ent, hero, pos)| {
                let prop = properties.get(ent);
                json!({
                    "entity_id": ent.id(),
                    "hero_id": hero.id,
                    "hero_name": hero.name,
                    "title": hero.title,
                    "level": hero.level,
                    "hp": prop.map(|p| p.hp).unwrap_or(0.0),
                    "max_hp": prop.map(|p| p.mhp).unwrap_or(0.0),
                    "pos_x": pos.0.x,
                    "pos_y": pos.0.y,
                })
            });

        player_list.push(json!({
            "name": player.name,
            "cost": player.cost,
            "tower_count": player.towers.len(),
            "hero": hero_info,
        }));
    }

    let data = json!({ "players": player_list });

    QueryResponse {
        success: true,
        error: String::new(),
        data_json: serde_json::to_vec(&data).unwrap_or_default(),
    }
}

/// 查詢指定玩家視角中所有單位的狀態
pub fn query_inspect_player_view(world: &World, player_name: &str) -> QueryResponse {
    let entities = world.entities();
    let heroes = world.read_storage::<Hero>();
    let units = world.read_storage::<Unit>();
    let creeps = world.read_storage::<Creep>();
    let towers = world.read_storage::<Tower>();
    let positions = world.read_storage::<Pos>();
    let properties = world.read_storage::<CProperty>();
    let move_targets = world.read_storage::<MoveTarget>();
    let tattacks = world.read_storage::<TAttack>();
    let tproperties = world.read_storage::<TProperty>();

    // 驗證玩家存在（從 BTreeMap 檢查）
    let pmap = world.read_resource::<BTreeMap<String, Player>>();
    if !pmap.contains_key(player_name) {
        return QueryResponse {
            success: false,
            error: format!("Player '{}' not found", player_name),
            data_json: Vec::new(),
        };
    }
    drop(pmap);

    let game_time = world.read_resource::<TimeOfDay>().0;
    let tick = world.read_resource::<Tick>().0;

    // 收集英雄資料
    let mut hero_list = Vec::new();
    for (ent, hero, pos) in (&entities, &heroes, &positions).join() {
        let prop = properties.get(ent);
        let mt = move_targets.get(ent);

        let abilities_json: Vec<serde_json::Value> = hero.abilities.iter().map(|ability_id| {
            let level = hero.ability_levels.get(ability_id).copied().unwrap_or(0);
            json!({
                "ability_id": ability_id,
                "level": level,
            })
        }).collect();

        hero_list.push(json!({
            "entity_id": ent.id(),
            "name": hero.name,
            "title": hero.title,
            "level": hero.level,
            "hp": prop.map(|p| p.hp).unwrap_or(0.0),
            "max_hp": prop.map(|p| p.mhp).unwrap_or(0.0),
            "x": pos.0.x,
            "y": pos.0.y,
            "move_target": mt.map(|m| json!({"x": m.0.x, "y": m.0.y})),
            "abilities": abilities_json,
        }));
    }

    // 收集單位資料
    let mut unit_list = Vec::new();
    for (ent, unit, pos) in (&entities, &units, &positions).join() {
        let mt = move_targets.get(ent);

        unit_list.push(json!({
            "entity_id": ent.id(),
            "name": unit.name,
            "type": format!("{:?}", unit.unit_type),
            "hp": unit.current_hp,
            "max_hp": unit.max_hp,
            "x": pos.0.x,
            "y": pos.0.y,
            "atk_target": unit.current_target.map(|t| t.id()),
            "move_target": mt.map(|m| json!({"x": m.0.x, "y": m.0.y})),
        }));
    }

    // 收集小兵資料
    let mut creep_list = Vec::new();
    for (ent, creep, pos) in (&entities, &creeps, &positions).join() {
        let prop = properties.get(ent);

        creep_list.push(json!({
            "entity_id": ent.id(),
            "name": creep.name,
            "path": creep.path,
            "status": format!("{:?}", creep.status),
            "hp": prop.map(|p| p.hp).unwrap_or(0.0),
            "max_hp": prop.map(|p| p.mhp).unwrap_or(0.0),
            "x": pos.0.x,
            "y": pos.0.y,
            "block_tower": creep.block_tower.map(|t| t.id()),
        }));
    }

    // 收集塔資料
    let mut tower_list = Vec::new();
    for (ent, _tower, pos) in (&entities, &towers, &positions).join() {
        let tatk = tattacks.get(ent);
        let tprop = tproperties.get(ent);

        tower_list.push(json!({
            "entity_id": ent.id(),
            "x": pos.0.x,
            "y": pos.0.y,
            "hp": tprop.map(|p| f32::from(p.hp)).unwrap_or(0.0),
            "block": tprop.map(|p| p.block).unwrap_or(0),
            "max_block": tprop.map(|p| p.mblock).unwrap_or(0),
            "atk_physic": tatk.map(|a| f32::from(a.atk_physic)).unwrap_or(0.0),
            "atk_speed": tatk.map(|a| f32::from(a.asd)).unwrap_or(0.0),
            "range": tatk.map(|a| f32::from(a.range)).unwrap_or(0.0),
        }));
    }

    let data = json!({
        "player": player_name,
        "tick": tick,
        "game_time": game_time,
        "heroes": hero_list,
        "units": unit_list,
        "creeps": creep_list,
        "towers": tower_list,
    });

    QueryResponse {
        success: true,
        error: String::new(),
        data_json: serde_json::to_vec(&data).unwrap_or_default(),
    }
}
