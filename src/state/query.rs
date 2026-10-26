use serde_json::json;
use specs::{Entity, Join, World, WorldExt};
/// ECS 狀態查詢模塊
/// 提供 read-only 的 ECS World 查詢，供 MCP server 使用
use std::collections::BTreeMap;

use crate::comp::*;
use crate::transport::QueryResponse;
use omoba_core::runtime::ability_runtime::AbilityRegistry;

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
                (&entities, &heroes, &positions).join().find(|(ent, _, _)| {
                    factions
                        .get(*ent)
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
                    // 注意： omb-mcp 查詢回應 — f32 是有意的公共診斷
                    // API。 omb-mcp 消費者不運行 sim；不需要步調一致的決定論
                    // 這裡。不要遷移到固定64 線格式。
                    "hp": prop.map(|p| p.hp.to_f32_for_render()).unwrap_or(0.0),
                    "max_hp": prop.map(|p| p.mhp.to_f32_for_render()).unwrap_or(0.0),
                    "pos_x": pos.0.x.to_f32_for_render(),
                    "pos_y": pos.0.y.to_f32_for_render(),
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

/// 列出所有已註冊的技能 metadata（client 用於 tooltip / 技能樹 UI）。
/// 資料來自 `AbilityRegistry` resource；Phase 1 時 registry 為空 → 回空 list。
pub fn query_list_abilities(world: &World) -> QueryResponse {
    let registry = world.read_resource::<AbilityRegistry>();
    let abilities: Vec<&omoba_core::ability_meta::AbilityDef> = registry.all().collect();
    let data = json!({ "abilities": abilities });
    QueryResponse {
        success: true,
        error: String::new(),
        data_json: serde_json::to_vec(&data).unwrap_or_default(),
    }
}

/// 查詢單一技能的完整 metadata（`ability_id` 借 `player_name` 欄位傳遞，
/// 避免擴充既有 QueryRequest schema）。
pub fn query_get_ability_detail(world: &World, ability_id: &str) -> QueryResponse {
    let registry = world.read_resource::<AbilityRegistry>();
    match registry.get(ability_id) {
        Some(def) => QueryResponse {
            success: true,
            error: String::new(),
            data_json: serde_json::to_vec(def).unwrap_or_default(),
        },
        None => QueryResponse {
            success: false,
            error: format!("Ability '{}' not found", ability_id),
            data_json: Vec::new(),
        },
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

        let abilities_json: Vec<serde_json::Value> = hero
            .abilities
            .iter()
            .map(|ability_id| {
                let level = hero.ability_levels.get(ability_id).copied().unwrap_or(0);
                json!({
                    "ability_id": ability_id,
                    "level": level,
                })
            })
            .collect();

        hero_list.push(json!({
            "entity_id": ent.id(),
            "name": hero.name,
            "title": hero.title,
            "level": hero.level,
            // 注意： omb-mcp 查詢回應 — f32 是有意的公共診斷
            // API。 omb-mcp 消費者不運行 sim；不需要步調一致的決定論
            // 這裡。不要遷移到固定64 線格式。
            "hp": prop.map(|p| p.hp.to_f32_for_render()).unwrap_or(0.0),
            "max_hp": prop.map(|p| p.mhp.to_f32_for_render()).unwrap_or(0.0),
            "x": pos.0.x.to_f32_for_render(),
            "y": pos.0.y.to_f32_for_render(),
            "move_target": mt.map(|m| json!({"x": m.0.x.to_f32_for_render(), "y": m.0.y.to_f32_for_render()})),
            "abilities": abilities_json,
        }));
    }

    // 收集單位資料
    let mut unit_list = Vec::new();
    for (ent, unit, pos) in (&entities, &units, &positions).join() {
        let mt = move_targets.get(ent);

        // 注意： omb-mcp 查詢回應 — 單位 hp i32 / pos 固定 64 邊界。 f32 是
        // 有意的公共診斷 API。 omb-mcp 消費者不運作 sim；步調一致
        // 這裡不需要決定論。不要遷移到固定64 線格式。
        unit_list.push(json!({
            "entity_id": ent.id(),
            "name": unit.name,
            "type": format!("{:?}", unit.unit_type),
            "hp": unit.current_hp,
            "max_hp": unit.max_hp,
            "x": pos.0.x.to_f32_for_render(),
            "y": pos.0.y.to_f32_for_render(),
            "atk_target": unit.current_target.map(|t| t.id()),
            "move_target": mt.map(|m| json!({"x": m.0.x.to_f32_for_render(), "y": m.0.y.to_f32_for_render()})),
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
            // 注意： omb-mcp 查詢回應 — f32 是有意的公共診斷
            // API。 omb-mcp 消費者不運行 sim；不需要步調一致的決定論
            // 這裡。不要遷移到固定64 線格式。
            "hp": prop.map(|p| p.hp.to_f32_for_render()).unwrap_or(0.0),
            "max_hp": prop.map(|p| p.mhp.to_f32_for_render()).unwrap_or(0.0),
            "x": pos.0.x.to_f32_for_render(),
            "y": pos.0.y.to_f32_for_render(),
            "block_tower": creep.block_tower.map(|t| t.id()),
        }));
    }

    // 收集塔資料
    let mut tower_list = Vec::new();
    for (ent, _tower, pos) in (&entities, &towers, &positions).join() {
        let tatk = tattacks.get(ent);
        let tprop = tproperties.get(ent);

        // 注意： omb-mcp 查詢回應 — f32 是有意的公共診斷
        // API。 omb-mcp 消費者不運行 sim；不需要步調一致的決定論
        // 這裡。不要遷移到固定64 線格式。
        tower_list.push(json!({
            "entity_id": ent.id(),
            "x": pos.0.x.to_f32_for_render(),
            "y": pos.0.y.to_f32_for_render(),
            "hp": tprop.map(|p| p.hp.v.to_f32_for_render()).unwrap_or(0.0),
            "block": tprop.map(|p| p.block).unwrap_or(0),
            "max_block": tprop.map(|p| p.mblock).unwrap_or(0),
            "atk_physic": tatk.map(|a| a.atk_physic.v.to_f32_for_render()).unwrap_or(0.0),
            "atk_speed": tatk.map(|a| a.asd.v.to_f32_for_render()).unwrap_or(0.0),
            "range": tatk.map(|a| a.range.v.to_f32_for_render()).unwrap_or(0.0),
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

/// Whole-match authority status for local debugging tools (omb-mcp `match_status`).
/// This is the unfiltered authority view: it must never reach a player, so every
/// state query is refused unless the server runs with `OMB_DEBUG_QUERY=1`.
pub fn query_match_status(world: &World) -> QueryResponse {
    use omoba_core::runtime::{MobaMatch, MobaMatchPhase};
    let Some(state) = world.try_fetch::<MobaMatch>() else {
        return QueryResponse {
            success: false,
            error: "no MOBA match is running".into(),
            data_json: Vec::new(),
        };
    };
    let entities = world.entities();
    let positions = world.read_storage::<Pos>();
    let properties = world.read_storage::<CProperty>();
    let vitals = |entity: Option<Entity>| -> serde_json::Value {
        match entity.filter(|e| entities.is_alive(*e)) {
            Some(e) => {
                let prop = properties.get(e);
                let pos = positions.get(e);
                json!({
                    "alive": true,
                    "hp": prop.map(|p| p.hp.to_f32_for_render()),
                    "max_hp": prop.map(|p| p.mhp.to_f32_for_render()),
                    "position": pos.map(|p| [p.0.x.to_f32_for_render(), p.0.y.to_f32_for_render()]),
                })
            }
            None => json!({ "alive": false }),
        }
    };
    let (phase, winner, finished_tick) = match state.phase {
        MobaMatchPhase::Warmup => ("warmup", None, None),
        MobaMatchPhase::Playing => ("playing", None, None),
        MobaMatchPhase::Finished { winner, tick } => ("finished", winner, Some(tick)),
    };
    let heroes: Vec<serde_json::Value> = state
        .heroes
        .iter()
        .map(|slot| {
            json!({
                "player_id": slot.player_id,
                "side": slot.side,
                "kills": slot.kills,
                "deaths": slot.deaths,
                "assists": slot.assists,
                "respawns": slot.respawns,
                "respawn_at_seconds": slot.respawn_at.map(|t| t.to_f32_for_render()),
                "hero": vitals(slot.entity),
            })
        })
        .collect();
    let mut standing = [0u32; 2];
    let lanes: Vec<serde_json::Value> = state
        .lane_tower_layers
        .iter()
        .enumerate()
        .map(|(lane, layers)| {
            let layers: Vec<serde_json::Value> = layers
                .iter()
                .enumerate()
                .map(|(layer, sides)| {
                    let side_status: Vec<serde_json::Value> = sides
                        .iter()
                        .enumerate()
                        .map(|(side, tower)| {
                            let status = vitals(*tower);
                            if status["alive"] == true { standing[side] += 1; }
                            status
                        })
                        .collect();
                    json!({ "layer": layer, "sides": side_status })
                })
                .collect();
            json!({ "lane": lane, "layers": layers })
        })
        .collect();
    let data = json!({
        "schema_version": 1,
        "scope": "authority-debug-unfiltered",
        "phase": phase,
        "winner_side": winner,
        "finished_tick": finished_tick,
        "elapsed_seconds": state.elapsed.to_f32_for_render(),
        "waves": state.waves,
        "towers_standing_by_side": standing,
        "bases": state.bases.iter().map(|base| vitals(*base)).collect::<Vec<_>>(),
        "base_unlocked_by_side": [state.base_unlocked(0), state.base_unlocked(1)],
        "base_structures": state.base_structure_rows().into_iter().map(|(team, lane, role, entity, respawn_at, unlocked)| {
            let mut row = vitals(entity);
            row["side"] = json!(team);
            row["lane"] = json!(lane);
            row["role"] = json!(format!("{role:?}"));
            row["attackable"] = json!(unlocked);
            row["respawn_at_seconds"] = json!(respawn_at.map(|t| t.to_f32_for_render()));
            row
        }).collect::<Vec<_>>(),
        "heroes": heroes,
        "lanes": lanes,
    });
    QueryResponse {
        success: true,
        error: String::new(),
        data_json: serde_json::to_vec(&data).unwrap_or_default(),
    }
}

/// Debug state queries are opt-in per server process (`OMB_DEBUG_QUERY=1`).
pub fn debug_queries_enabled() -> bool {
    std::env::var("OMB_DEBUG_QUERY").map(|value| value == "1").unwrap_or(false)
}
