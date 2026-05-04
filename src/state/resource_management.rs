/// 資源管理器 - 負責處理遊戲資源和玩家請求

use specs::{World, Entity, WorldExt, Join};
use crossbeam_channel::{Receiver, Sender};
use failure::Error;

use crate::comp::*;
use crate::transport::{OutboundMsg, InboundMsg};
use crate::Outcome;

/// 資源管理器
pub struct ResourceManager {
    /// MQTT 發送通道
    mqtx: Sender<OutboundMsg>,
}

impl ResourceManager {
    /// 創建新的資源管理器
    pub fn new(mqtx: Sender<OutboundMsg>) -> Self {
        Self { mqtx }
    }

    /// 處理小兵波生成
    pub fn process_creep_waves(&self, _world: &mut World) -> Result<(), Error> {
        // 實現小兵波處理邏輯
        // 暫時為空實現
        Ok(())
    }

    /// 處理遊戲結果事件
    pub fn process_outcomes(&self, world: &mut World) -> Result<(), Error> {
        // 使用 GameProcessor 來處理所有的 outcomes
        crate::comp::GameProcessor::process_outcomes(world, &self.mqtx)?;
        Ok(())
    }

    /// 處理玩家資料
    pub fn process_player_data(&self, world: &mut World, mqrx: &Receiver<InboundMsg>) -> Result<(), Error> {
        // 處理所有接收到的玩家資料
        while let Ok(player_data) = mqrx.try_recv() {
            match player_data.t.as_str() {
                "tower" => {
                    self.handle_tower_request(world, player_data)?;
                }
                "player" => {
                    self.handle_player_request(world, player_data)?;
                }
                "screen" | "screen_request" => {
                    self.handle_screen_request(world, player_data)?;
                }
                _ => {
                    log::warn!("未知的玩家類型: {}", player_data.t);
                }
            }
        }
        Ok(())
    }

    /// 處理塔相關請求
    pub fn handle_tower_request(&self, world: &mut World, pd: InboundMsg) -> Result<(), Error> {
        use serde_json::json;
        
        match pd.a.as_str() {
            "create" => {
                self.create_tower(world, &pd)?;
                log::info!("創建塔: 玩家 {}", pd.name);
            }
            "upgrade" => {
                self.upgrade_tower(world, &pd)?;
                log::info!("升級塔: 玩家 {}", pd.name);
            }
            "sell" => {
                self.sell_tower(world, &pd)?;
                log::info!("出售塔: 玩家 {}", pd.name);
            }
            _ => {
                log::warn!("未知的塔操作: {}", pd.a);
            }
        }
        
        // 發送確認消息
        let response = json!({
            "action": pd.a,
            "status": "completed",
            "player": pd.name
        });
        self.mqtx.send(OutboundMsg::new_s("td/all/res", "tower", "R", response))?;
        
        Ok(())
    }

    /// 處理玩家相關請求
    pub fn handle_player_request(&self, world: &mut World, pd: InboundMsg) -> Result<(), Error> {
        use serde_json::json;
        
        match pd.a.as_str() {
            "move" => {
                self.move_player(world, &pd)?;
                log::info!("移動玩家: {}", pd.name);
            }
            "attack" => {
                self.player_attack(world, &pd)?;
                log::info!("玩家攻擊: {}", pd.name);
            }
            "skill" | "cast_ability" => {
                self.use_skill(world, &pd)?;
                log::info!("使用技能: 玩家 {}", pd.name);
            }
            "upgrade_skill" => {
                self.upgrade_skill(world, &pd)?;
            }
            "buy_item" => {
                self.buy_item(world, &pd)?;
            }
            "sell_item" => {
                self.sell_item(world, &pd)?;
            }
            "use_item" => {
                self.use_item(world, &pd)?;
            }
            "start_round" => {
                self.start_round(world)?;
            }
            _ => {
                log::warn!("未知的玩家操作: {}", pd.a);
            }
        }
        
        // 發送確認消息
        let response = json!({
            "action": pd.a,
            "status": "completed",
            "player": pd.name
        });
        self.mqtx.send(OutboundMsg::new_s("td/all/res", "player", "R", response))?;
        
        Ok(())
    }

    /// 處理畫面請求
    pub fn handle_screen_request(&self, world: &mut World, pd: InboundMsg) -> Result<(), Error> {
        use serde_json::json;
        
        match pd.a.as_str() {
            "get_area" | "get_screen_area" => {
                let area_data = self.get_screen_area_data(world, &pd)?;
                let response = json!({
                    "action": "get_area",
                    "status": "completed",
                    "player": pd.name,
                    "data": area_data
                });
                self.mqtx.send(OutboundMsg::new_s("td/all/res", "screen", "R", response))?;
                log::info!("發送畫面區域資料給玩家 {}", pd.name);
            }
            "update_view" => {
                self.update_player_view(world, &pd)?;
                log::info!("更新玩家 {} 視野", pd.name);
            }
            _ => {
                log::warn!("未知的畫面操作: {}", pd.a);
            }
        }
        
        Ok(())
    }

    // 私有實現方法
    fn create_tower(&self, world: &mut World, pd: &InboundMsg) -> Result<(), Error> {
        use vek::Vec2;
        use specs::{Builder, WorldExt, Join};

        let x = pd.d.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
        let y = pd.d.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
        let pos = Vec2::new(x, y);

        let is_td = world.read_resource::<GameMode>().is_td();
        if !is_td {
            // 舊 MOBA / debug 路徑：直接放一座預設塔（保留向後相容）
            use omoba_sim::Fixed64;
            let tower_property = TProperty::new(Fixed64::from_i32(100), 1, Fixed64::from_i32(200));
            let tower_attack = TAttack::new(
                Fixed64::from_i32(50),
                Fixed64::from_raw(1536), // 1.5
                Fixed64::from_i32(300),
                Fixed64::from_i32(800),
            );
            let _ = world.create_entity()
                .with(Pos::from_xy_f32(pos.x, pos.y))
                .with(Vel::zero())
                .with(Tower::new())
                .with(tower_property)
                .with(tower_attack)
                .build();
            let mut outcomes = world.write_resource::<Vec<Outcome>>();
            let pos_sim = omoba_sim::Vec2::new(
                Fixed64::from_raw((pos.x * 1024.0) as i64),
                Fixed64::from_raw((pos.y * 1024.0) as i64),
            );
            outcomes.push(Outcome::Tower {
                pos: pos_sim,
                td: TowerData { tpty: tower_property, tatk: tower_attack },
            });
            return Ok(());
        }

        // ===== TD 模式：unit_id + cost + 碰撞檢查 =====
        // 前端送 unit_id（例如 "tower_dart"）；從 TowerTemplateRegistry 查 template
        let kind_str = pd.d.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        let tpl = {
            let reg = world.read_resource::<crate::comp::tower_registry::TowerTemplateRegistry>();
            reg.get(kind_str).cloned()
        };
        let Some(tpl) = tpl else {
            log::warn!("未知塔 unit_id '{}'，放棄建造", kind_str);
            return Ok(());
        };

        // 找到玩家英雄（TD 地圖保證只有一個）
        let hero_entity = {
            let entities = world.entities();
            let heroes = world.read_storage::<Hero>();
            let factions = world.read_storage::<Faction>();
            let mut found = None;
            for (e, _h, f) in (&entities, &heroes, &factions).join() {
                if f.faction_id == FactionType::Player {
                    found = Some(e);
                    break;
                }
            }
            found
        };
        let Some(hero_entity) = hero_entity else {
            log::warn!("TD 蓋塔：找不到玩家英雄");
            return Ok(());
        };

        // 金幣檢查
        let has_gold = {
            let golds = world.read_storage::<Gold>();
            golds.get(hero_entity).map(|g| g.0).unwrap_or(0) >= tpl.cost
        };
        if !has_gold {
            log::info!("TD 蓋塔：金幣不足（需要 {}）", tpl.cost);
            return Ok(());
        }

        // Region 碰撞
        {
            let regions = world.read_resource::<BlockedRegions>();
            for r in regions.0.iter() {
                if crate::util::geometry::circle_hits_polygon(pos, tpl.footprint, &r.points) {
                    log::info!("TD 蓋塔：位置 ({:.0},{:.0}) 壓到 region '{}'", pos.x, pos.y, r.name);
                    return Ok(());
                }
            }
        }

        // Path 碰撞（圓 vs 線段 + path 半寬）
        const PATH_HALF_WIDTH: f32 = 64.0; // 80 × 0.8：視覺路寬縮小 20% 後對應的禁蓋緩衝
        {
            use std::collections::BTreeMap;
            let paths = world.read_resource::<BTreeMap<String, Path>>();
            let clear = tpl.footprint + PATH_HALF_WIDTH;
            let clear_sq = clear * clear;
            for (name, path) in paths.iter() {
                let cps = &path.check_points;
                for i in 0..cps.len().saturating_sub(1) {
                    let a = cps[i].pos;
                    let b = cps[i + 1].pos;
                    if crate::util::geometry::point_segment_dist_sq(pos, a, b) < clear_sq {
                        log::info!("TD 蓋塔：位置 ({:.0},{:.0}) 壓到 path '{}'", pos.x, pos.y, name);
                        return Ok(());
                    }
                }
            }
        }

        // 其他塔重疊檢查
        {
            let entities = world.entities();
            let towers = world.read_storage::<Tower>();
            let positions = world.read_storage::<Pos>();
            let radii = world.read_storage::<CollisionRadius>();
            for (_e, _t, p, r) in (&entities, &towers, &positions, &radii).join() {
                // NOTE: Searcher / spatial index uses f32 internally for instant_distance lib compat.
                let (px, py) = p.xy_f32();
                let dx = px - pos.x;
                let dy = py - pos.y;
                let d_sq = dx * dx + dy * dy;
                let min_d = tpl.footprint + r.0.to_f32_for_render();
                if d_sq < min_d * min_d {
                    log::info!("TD 蓋塔：位置 ({:.0},{:.0}) 與其他塔重疊", pos.x, pos.y);
                    return Ok(());
                }
            }
        }

        // 所有檢查通過，扣錢 + spawn 塔
        {
            let mut golds = world.write_storage::<Gold>();
            if let Some(g) = golds.get_mut(hero_entity) {
                g.0 -= tpl.cost;
            }
        }
        let tower_entity = match crate::comp::tower_template::spawn_td_tower(world, pos, &tpl.unit_id) {
            Some(e) => e,
            None => {
                log::warn!("spawn_td_tower 失敗 unit_id={}", tpl.unit_id);
                return Ok(());
            }
        };
        world.get_mut::<Searcher>().unwrap().tower.mark_dirty();
        log::info!(
            "🏗 TD 塔 '{}' 已蓋於 ({:.0},{:.0}) entity={:?} cost={}",
            tpl.label, pos.x, pos.y, tower_entity, tpl.cost
        );

        // 廣播 tower.create
        {
            let positions = world.read_storage::<Pos>();
            let properties = world.read_storage::<CProperty>();
            let radii = world.read_storage::<CollisionRadius>();
            let hp = properties.get(tower_entity).map(|p| p.hp.to_f32_for_render()).unwrap_or(tpl.hp);
            let mhp = properties.get(tower_entity).map(|p| p.mhp.to_f32_for_render()).unwrap_or(tpl.hp);
            let radius = radii.get(tower_entity).map(|r| r.0.to_f32_for_render()).unwrap_or(tpl.footprint);
            let json_fallback = serde_json::json!({
                "id": tower_entity.id(),
                "entity_id": tower_entity.id(),
                "name": tpl.label,
                "kind": tpl.unit_id,
                "position": { "x": pos.x, "y": pos.y },
                "hp": hp,
                "max_hp": mhp,
                "collision_radius": radius,
                "range": tpl.range,
                "is_base": false,
            });
            #[cfg(feature = "kcp")]
            let msg = OutboundMsg::new_typed_at(
                "td/all/res", "tower", "create",
                crate::transport::TypedOutbound::TowerCreate(proto_build::tower_create(
                    tower_entity.id(), pos.x, pos.y, hp, mhp, &tpl.unit_id, &tpl.label,
                )),
                json_fallback, pos.x, pos.y,
            );
            #[cfg(not(feature = "kcp"))]
            let msg = OutboundMsg::new_s_at(
                "td/all/res", "tower", "create", json_fallback, pos.x, pos.y,
            );
            let _ = self.mqtx.send(msg);
        }

        // 扣完金幣主動廣播 hero.stats（避免前端 HUD 滯後）
        self.push_hero_stats(world, hero_entity);

        Ok(())
    }

    /// 處理 TD 模式的 `player/start_round` 指令：把 CurrentCreepWave.is_running
    /// 切成 true、記錄 wave_start_time = totaltime，並廣播 `game/round` 告訴前端。
    /// 非 TD 模式忽略（記 log 但不做事）。
    fn start_round(&self, world: &mut World) -> Result<(), Error> {
        use serde_json::json;

        let is_td = world.read_resource::<GameMode>().is_td();
        if !is_td {
            log::warn!("start_round 指令在非 TD 模式下被忽略");
            return Ok(());
        }
        let totaltime = world.read_resource::<Time>().0;
        let (round, total, already) = {
            let mut ccw = world.write_resource::<CurrentCreepWave>();
            let waves_len = world.read_resource::<Vec<CreepWave>>().len();
            let already = ccw.is_running || ccw.wave >= waves_len;
            if !already {
                ccw.is_running = true;
                ccw.wave_start_time = totaltime as f32;
                ccw.path.clear();
            }
            (ccw.wave + 1, waves_len, already)
        };
        if already {
            log::info!("start_round 忽略：波已在跑或關卡已結束");
            return Ok(());
        }
        log::info!("▶️ TD 開始第 {}/{} 波 @ t={:.1}s", round, total, totaltime);

        let payload = json!({
            "round": round,
            "total": total,
            "is_running": true,
        });
        #[cfg(feature = "kcp")]
        let msg = OutboundMsg::new_typed_all(
            "td/all/res", "game", "round",
            crate::transport::TypedOutbound::GameRound(proto_build::game_round(round as u32, total as u32, true)),
            payload,
        );
        #[cfg(all(not(feature = "kcp"), any(feature = "grpc")))]
        let msg = OutboundMsg::new_s_all("td/all/res", "game", "round", payload);
        #[cfg(not(any(feature = "grpc", feature = "kcp")))]
        let msg = OutboundMsg::new_s("td/all/res", "game", "round", payload);
        self.mqtx.send(msg)?;
        Ok(())
    }

    /// 主動廣播指定英雄的 hot 狀態（hp/gold/damage/armor/msd/range/interval/buffs）。
    /// Phase 5.2: legacy 0x02 GameEvent producer cut — body no-ops. Lockstep
    /// TickBatch (0x10) carries authoritative hero state.
    pub(crate) fn push_hero_stats(&self, world: &mut World, hero_entity: specs::Entity) {
        let _ = (world, hero_entity);
    }

    /// Phase 5.2: legacy 0x02 GameEvent producer cut — body no-ops.
    pub(crate) fn push_hero_static(&self, world: &mut World, hero_entity: specs::Entity) {
        let _ = (world, hero_entity);
    }

    fn upgrade_tower(&self, world: &mut World, pd: &InboundMsg) -> Result<(), Error> {
        use serde_json::json;
        use specs::{Join, WorldExt};
        use omoba_core::tower_meta::{UpgradeEffect, StatOp};

        // 1. TD 模式檢查
        let is_td = world.read_resource::<GameMode>().is_td();
        if !is_td {
            log::warn!("upgrade_tower 指令在非 TD 模式下被忽略");
            return Ok(());
        }

        // 2. 解析 tower_id + path
        let tower_id_u32 = match pd.d.get("tower_id").and_then(|v| v.as_u64()) {
            Some(v) => v as u32,
            None => {
                log::warn!("TD 升級：payload 缺少 tower_id");
                return Ok(());
            }
        };
        let path = match pd.d.get("path").and_then(|v| v.as_u64()) {
            Some(v) => v as u8,
            None => {
                log::warn!("TD 升級：payload 缺少 path");
                return Ok(());
            }
        };
        if path >= 3 {
            log::warn!("TD 升級：path {} 無效（必須 0..=2）", path);
            let _ = self.mqtx.send(OutboundMsg::new_s(
                "td/all/res", "tower", "upgrade_reject",
                json!({
                    "tower_id": tower_id_u32,
                    "path": path,
                    "reason": "invalid_path",
                }),
            ));
            return Ok(());
        }

        // 3. 找塔 entity 並取 levels + unit_id
        let tower_info = {
            let entities = world.entities();
            let towers = world.read_storage::<Tower>();
            let tags = world.read_storage::<crate::scripting::ScriptUnitTag>();
            let mut found: Option<(specs::Entity, [u8; 3], String)> = None;
            for (e, t, tag) in (&entities, &towers, &tags).join() {
                if e.id() == tower_id_u32 {
                    found = Some((e, t.upgrade_levels, tag.unit_id.clone()));
                    break;
                }
            }
            found
        };
        let Some((tower_entity, levels, unit_id)) = tower_info else {
            log::warn!("TD 升級：找不到塔 id={}", tower_id_u32);
            return Ok(());
        };

        // 4. 規則驗證
        if let Err(rej) = crate::comp::tower_upgrade_rules::validate_upgrade(levels, path) {
            log::info!("TD 升級：規則拒絕 id={} path={} levels={:?} → {:?}",
                tower_id_u32, path, levels, rej);
            let reason = match rej {
                crate::comp::tower_upgrade_rules::UpgradeRejection::AlreadyMaxed => "already_maxed",
                crate::comp::tower_upgrade_rules::UpgradeRejection::TwoPrimaryPaths => "two_primary_paths",
                crate::comp::tower_upgrade_rules::UpgradeRejection::TwoSecondaryPaths => "two_secondary_paths",
                crate::comp::tower_upgrade_rules::UpgradeRejection::ThirdPathLocked => "third_path_locked",
            };
            let _ = self.mqtx.send(OutboundMsg::new_s(
                "td/all/res", "tower", "upgrade_reject",
                json!({
                    "tower_id": tower_id_u32,
                    "path": path,
                    "reason": reason,
                }),
            ));
            return Ok(());
        }
        let next_level = levels[path as usize] + 1;

        // 5. 查 UpgradeDef（clone 出來以釋放 borrow）
        let def = {
            let reg = world.read_resource::<crate::comp::tower_upgrade_registry::TowerUpgradeRegistry>();
            reg.get(&unit_id, path, next_level).cloned()
        };
        let Some(def) = def else {
            log::warn!("TD 升級：找不到 UpgradeDef kind={} path={} level={}",
                unit_id, path, next_level);
            return Ok(());
        };

        // 6. 找英雄 + 金幣檢查
        let hero_entity = {
            let entities = world.entities();
            let heroes = world.read_storage::<Hero>();
            let factions = world.read_storage::<Faction>();
            let mut found = None;
            for (e, _h, f) in (&entities, &heroes, &factions).join() {
                if f.faction_id == FactionType::Player {
                    found = Some(e);
                    break;
                }
            }
            found
        };
        let Some(hero_entity) = hero_entity else {
            log::warn!("TD 升級：找不到玩家英雄");
            return Ok(());
        };

        let has_gold = {
            let golds = world.read_storage::<Gold>();
            golds.get(hero_entity).map(|g| g.0).unwrap_or(0) >= def.cost
        };
        if !has_gold {
            log::info!("TD 升級：金幣不足（需要 {}）", def.cost);
            let _ = self.mqtx.send(OutboundMsg::new_s(
                "td/all/res", "tower", "upgrade_reject",
                json!({
                    "tower_id": tower_id_u32,
                    "path": path,
                    "reason": "insufficient_gold",
                    "cost": def.cost,
                }),
            ));
            return Ok(());
        }

        // 7. 扣錢
        {
            let mut golds = world.write_storage::<Gold>();
            if let Some(g) = golds.get_mut(hero_entity) {
                g.0 -= def.cost;
            }
        }

        // 8-9. 套 effects + 遞增 upgrade_levels（合併 Tower write 一次 open）
        let mut flags_to_add: Vec<String> = Vec::new();
        let mut stat_mods: Vec<(String, serde_json::Value)> = Vec::new();
        for (effect_idx, effect) in def.effects.iter().enumerate() {
            match effect {
                UpgradeEffect::BehaviorFlag { flag } => flags_to_add.push(flag.clone()),
                UpgradeEffect::StatMod { key, value, op: _ } => {
                    let buff_id = format!("upgrade_{}_{}_{}", path, next_level, effect_idx);
                    stat_mods.push((buff_id, json!({ key: *value })));
                }
            }
        }
        for (buff_id, payload) in stat_mods {
            let mut store = world.write_resource::<crate::ability_runtime::BuffStore>();
            // Phase 1c.3: BuffStore::add takes Fixed64 — sentinel "permanent" via raw i64::MAX.
            store.add(tower_entity, &buff_id, omoba_sim::Fixed64::from_raw(i64::MAX), payload);
        }
        let new_levels = {
            let mut towers = world.write_storage::<Tower>();
            let t = towers.get_mut(tower_entity)
                .expect("tower vanished mid-upgrade");
            for flag in flags_to_add {
                if !t.upgrade_flags.iter().any(|f| f == &flag) {
                    t.upgrade_flags.push(flag);
                }
            }
            t.upgrade_levels[path as usize] = next_level;
            t.upgrade_levels
        };

        // 10. 廣播 tower/upgrade
        let (tower_x_f, tower_y_f) = world.read_storage::<Pos>()
            .get(tower_entity).map(|p| p.xy_f32()).unwrap_or((0.0, 0.0));
        let payload = json!({
            "tower_id": tower_id_u32,
            "levels": [new_levels[0], new_levels[1], new_levels[2]],
        });
        #[cfg(feature = "kcp")]
        let msg = OutboundMsg::new_typed_at(
            "td/all/res", "tower", "upgrade",
            crate::transport::TypedOutbound::TowerUpgrade(proto_build::tower_upgrade(tower_id_u32, new_levels)),
            payload, tower_x_f, tower_y_f,
        );
        #[cfg(not(feature = "kcp"))]
        let msg = OutboundMsg::new_s_at(
            "td/all/res", "tower", "upgrade", payload, tower_x_f, tower_y_f,
        );
        let _ = self.mqtx.send(msg);

        log::info!("⬆️ TD 升級塔 id={} path={} → L{} ({}) cost={}",
            tower_id_u32, path, next_level, def.name, def.cost);

        // 11. 推 hero.stats（gold 即時更新）
        self.push_hero_stats(world, hero_entity);

        Ok(())
    }

    /// TD 模式賣塔：退 85% 建造費、刪掉塔 entity、廣播 delete。
    fn sell_tower(&self, world: &mut World, pd: &InboundMsg) -> Result<(), Error> {
        use serde_json::json;
        use specs::{Join, WorldExt};

        let is_td = world.read_resource::<GameMode>().is_td();
        if !is_td {
            log::warn!("sell_tower 指令在非 TD 模式下被忽略");
            return Ok(());
        }

        let tower_id_u32 = match pd.d.get("tower_id").and_then(|v| v.as_u64()) {
            Some(v) => v as u32,
            None => {
                log::warn!("TD 賣塔：payload 缺少 tower_id");
                return Ok(());
            }
        };

        // 找目標塔 entity
        let target_entity = {
            let entities = world.entities();
            let towers = world.read_storage::<Tower>();
            let mut found = None;
            for (e, _t) in (&entities, &towers).join() {
                if e.id() == tower_id_u32 {
                    found = Some(e);
                    break;
                }
            }
            found
        };
        let Some(target_entity) = target_entity else {
            log::warn!("TD 賣塔：找不到塔 id={}", tower_id_u32);
            return Ok(());
        };

        // 依 ScriptUnitTag → TowerTemplateRegistry.cost 算退款（85% base + 75% 升級費）
        let refund = {
            let tags = world.read_storage::<crate::scripting::ScriptUnitTag>();
            let reg = world.read_resource::<crate::comp::tower_registry::TowerTemplateRegistry>();
            let towers = world.read_storage::<Tower>();
            let ureg = world.read_resource::<crate::comp::tower_upgrade_registry::TowerUpgradeRegistry>();
            let base_refund = tags.get(target_entity)
                .and_then(|t| reg.get(&t.unit_id))
                .map(|tpl| (tpl.cost as f32 * 0.85) as i32)
                .unwrap_or(0);
            let upgrade_refund = if let (Some(t), Some(tag)) = (towers.get(target_entity), tags.get(target_entity)) {
                let mut total = 0i32;
                for path in 0..3u8 {
                    for level in 1..=t.upgrade_levels[path as usize] {
                        if let Some(def) = ureg.get(&tag.unit_id, path, level) {
                            total += (def.cost as f32 * 0.75) as i32;
                        }
                    }
                }
                total
            } else { 0 };
            base_refund + upgrade_refund
        };

        // 找英雄（TD 錢包）
        let hero_entity = {
            let entities = world.entities();
            let heroes = world.read_storage::<Hero>();
            let factions = world.read_storage::<Faction>();
            let mut found = None;
            for (e, _h, f) in (&entities, &heroes, &factions).join() {
                if f.faction_id == FactionType::Player {
                    found = Some(e);
                    break;
                }
            }
            found
        };
        if let Some(hero_entity) = hero_entity {
            let mut golds = world.write_storage::<Gold>();
            if let Some(g) = golds.get_mut(hero_entity) {
                g.0 += refund;
            }
        }

        // 清除 BuffStore 殘留（防止 upgrade_* f32::MAX 永久 buff 累積洩漏）
        {
            let mut store = world.write_resource::<crate::ability_runtime::BuffStore>();
            store.remove_all_for(target_entity);
        }

        // 刪塔（render-side scene node 由 SimWorldSnapshot.removed_entity_ids
        // 自動清理 — 1.6 重構走 Outcome::EntityRemoved，這裡先保留 raw delete
        // 等 1.6b 全 site 清查時改走 delete_entity_tracked）
        world.entities().delete(target_entity).ok();

        // 推新 hero.stats（gold 即時更新）
        if let Some(hero_entity) = hero_entity {
            self.push_hero_stats(world, hero_entity);
        }

        log::info!("🏚 TD 賣塔 id={} 退款 {}", tower_id_u32, refund);
        Ok(())
    }

    fn move_player(&self, world: &mut World, pd: &InboundMsg) -> Result<(), Error> {
        use vek::Vec2;

        // 解析目標位置
        let x = pd.d.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
        let y = pd.d.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;

        // 若 pd.d.entity_id 存在，直接對該 entity 下 MoveTarget（召喚物 / saika_gunner 用）；
        // 否則 fallback 成 Hero 名稱匹配（原行為）。
        let explicit_id = pd.d.get("entity_id").and_then(|v| v.as_u64());

        let target_entity = if let Some(eid) = explicit_id {
            // 用 entity id 找 alive entity；不限 Hero — 只要 alive 即可
            let entities = world.entities();
            let mut found = None;
            for e in (&entities).join() {
                if e.id() as u64 == eid {
                    found = Some(e);
                    break;
                }
            }
            if found.is_none() {
                log::warn!("move_player: entity_id={} 不存在或已死亡", eid);
            }
            found
        } else {
            let entities = world.entities();
            let heroes = world.read_storage::<Hero>();
            let mut found = None;
            for (e, hero) in (&entities, &heroes).join() {
                if hero.name == pd.name || pd.name.is_empty() {
                    found = Some(e);
                    break;
                }
            }
            if found.is_none() {
                for (e, _) in (&entities, &heroes).join() {
                    found = Some(e);
                    break;
                }
            }
            found
        };

        // 設定 MoveTarget
        if let Some(entity) = target_entity {
            let mut move_targets = world.write_storage::<MoveTarget>();
            let _ = move_targets.insert(entity, MoveTarget::from_xy_f32(x, y));
            if let Some(eid) = explicit_id {
                log::info!("設定 entity {} 移動目標: ({}, {})", eid, x, y);
            } else {
                log::info!("設定英雄移動目標: ({}, {})", x, y);
            }
        } else if explicit_id.is_none() {
            log::warn!("找不到英雄實體: {}", pd.name);
        }

        Ok(())
    }

    fn player_attack(&self, _world: &mut World, _pd: &InboundMsg) -> Result<(), Error> {
        // 實現玩家攻擊邏輯
        Ok(())
    }

    fn use_skill(&self, world: &mut World, pd: &InboundMsg) -> Result<(), Error> {
        use crate::scripting::event::{ScriptEvent, ScriptEventQueue, SkillTarget};

        // slot Q/W/E/R → index 0..3
        let slot = pd.d.get("slot").and_then(|v| v.as_str()).unwrap_or("");
        let idx = match Self::slot_to_index(slot) {
            Some(i) => i,
            None => {
                log::warn!("[cast_ability] invalid slot '{}'", slot);
                return Ok(());
            }
        };

        let caster = match self.find_hero_entity(world, &pd.name) {
            Some(e) => e,
            None => {
                log::warn!("[cast_ability] no hero for '{}'", pd.name);
                return Ok(());
            }
        };

        let ability_id = {
            let heroes = world.read_storage::<Hero>();
            heroes
                .get(caster)
                .and_then(|h| h.abilities.get(idx).cloned())
                .unwrap_or_default()
        };
        if ability_id.is_empty() {
            log::warn!(
                "[cast_ability] hero '{}' slot {} has no ability bound",
                pd.name, slot
            );
            return Ok(());
        }

        // Gate：必須學過 + 不在 CD（防止 client 繞過 UI 直接送命令）
        {
            let heroes = world.read_storage::<Hero>();
            let h = match heroes.get(caster) {
                Some(h) => h,
                None => return Ok(()),
            };
            if !h.can_use_ability(&ability_id) {
                log::warn!(
                    "[cast_ability] hero '{}' slot {} ability '{}' not learned (level=0)",
                    pd.name, slot, ability_id
                );
                return Ok(());
            }
            if h.is_on_cooldown(&ability_id) {
                log::warn!(
                    "[cast_ability] hero '{}' slot {} ability '{}' still on cooldown ({:.2}s remaining)",
                    pd.name, slot, ability_id, h.get_cooldown(&ability_id).to_f32_for_render()
                );
                return Ok(());
            }
        }

        // 解析 target_pos [x,y] 或 target_entity (u64)
        let target = if let Some(arr) = pd.d.get("target_pos").and_then(|v| v.as_array()) {
            if arr.len() == 2 {
                let x = arr[0].as_f64().unwrap_or(0.0) as f32;
                let y = arr[1].as_f64().unwrap_or(0.0) as f32;
                use omoba_sim::Fixed64;
                SkillTarget::Point {
                    x: Fixed64::from_raw((x * 1024.0) as i64),
                    y: Fixed64::from_raw((y * 1024.0) as i64),
                }
            } else {
                SkillTarget::None
            }
        } else if let Some(id) = pd.d.get("target_entity").and_then(|v| v.as_u64()) {
            use specs::Join;
            let entities = world.entities();
            entities
                .join()
                .find(|e| e.id() == id as u32)
                .map(SkillTarget::Entity)
                .unwrap_or(SkillTarget::None)
        } else {
            SkillTarget::None
        };

        log::info!(
            "[cast_ability] {} casts '{}' (slot {}) target={:?}",
            pd.name, ability_id, slot, target
        );
        world
            .write_resource::<ScriptEventQueue>()
            .push(ScriptEvent::SkillCast {
                caster,
                skill_id: ability_id,
                target,
            });
        Ok(())
    }

    // ===== MVP LoL: skill/item 管理 =====

    fn find_hero_entity(&self, world: &World, name: &str) -> Option<Entity> {
        let entities = world.entities();
        let heroes = world.read_storage::<Hero>();
        let mut fallback = None;
        for (e, hero) in (&entities, &heroes).join() {
            if !name.is_empty() && hero.name == name {
                return Some(e);
            }
            if fallback.is_none() {
                fallback = Some(e);
            }
        }
        fallback
    }

    fn slot_to_index(slot: &str) -> Option<usize> {
        match slot {
            // 新版快捷鍵 W/E/R/T（slot 0/1/2/3）
            "W" | "w" | "0" => Some(0),
            "E" | "e" | "1" => Some(1),
            "R" | "r" | "2" => Some(2),
            "T" | "t" | "3" => Some(3),
            // 向下相容：舊版 Q/W/E/R 仍接受（用於測試舊 client）
            "Q" | "q" => Some(0),
            _ => None,
        }
    }

    fn broadcast_hero_update(&self, world: &World, hero_entity: Entity) {
        use serde_json::json;
        let heroes = world.read_storage::<Hero>();
        let golds = world.read_storage::<Gold>();
        let invs = world.read_storage::<Inventory>();
        let props = world.read_storage::<CProperty>();
        let atks = world.read_storage::<crate::comp::TAttack>();
        let positions = world.read_storage::<Pos>();
        let buff_store = world.read_resource::<crate::ability_runtime::BuffStore>();

        let hero = heroes.get(hero_entity);
        let gold = golds.get(hero_entity).map(|g| g.0).unwrap_or(0);
        let (pos_x_f, pos_y_f) = positions.get(hero_entity).map(|p| p.xy_f32()).unwrap_or((0.0, 0.0));
        let pos_vek = vek::Vec2::new(pos_x_f, pos_y_f);
        #[cfg(not(feature = "kcp"))]
        let lives = world.read_resource::<PlayerLives>().0;
        if let Some(h) = hero {
            let prop = props.get(hero_entity);
            let (hp, mhp) = prop.map(|p| (p.hp.to_f32_for_render(), p.mhp.to_f32_for_render())).unwrap_or((0.0, 0.0));
            let (armor_b, mres_b, msd_b) = prop
                .map(|p| (p.def_physic.to_f32_for_render(), p.def_magic.to_f32_for_render(), p.msd.to_f32_for_render()))
                .unwrap_or((0.0, 0.0, 0.0));
            #[cfg(feature = "kcp")]
            {
                let (atk_dmg_b, atk_int_b, atk_rng_b) = atks.get(hero_entity)
                    .map(|a| (a.atk_physic.v.to_f32_for_render(), a.asd.v.to_f32_for_render(), a.range.v.to_f32_for_render()))
                    .unwrap_or((0.0, 0.0, 0.0));
                // P3: inventory/ability 變化時同時 push static（可能 abilities/points 改了）
                // + hot（gold 可能改了）。shim 會緩存 static 並跟後續 hot 合併。
                let static_msg = build_hero_static_msg(hero_entity, h, pos_vek);
                let _ = self.mqtx.send(static_msg);
                let hot_msg = build_hero_hot_msg(
                    hero_entity, h, gold, hp, mhp, armor_b, mres_b, msd_b,
                    atk_dmg_b, atk_int_b, atk_rng_b, &buff_store, pos_vek,
                );
                let _ = self.mqtx.send(hot_msg);
            }
            #[cfg(not(feature = "kcp"))]
            {
                let (atk_dmg_b, atk_int_b, atk_rng_b, bullet_spd) = atks.get(hero_entity)
                    .map(|a| (a.atk_physic.v.to_f32_for_render(), a.asd.v.to_f32_for_render(), a.range.v.to_f32_for_render(), a.bullet_speed.to_f32_for_render()))
                    .unwrap_or((0.0, 0.0, 0.0, 0.0));
                let payload = build_hero_stats_payload(
                    hero_entity, h, gold, hp, mhp, armor_b, mres_b, msd_b,
                    atk_dmg_b, atk_int_b, atk_rng_b, bullet_spd, lives, &buff_store,
                );
                let _ = self.mqtx.send(OutboundMsg::new_s_at(
                    "td/all/res", "hero", "stats", payload, pos_x_f, pos_y_f,
                ));
            }
        }
        if let Some(inv) = invs.get(hero_entity) {
            let slots: Vec<serde_json::Value> = inv
                .slots
                .iter()
                .map(|s| match s {
                    Some(it) => json!({"item_id": it.item_id, "cd": it.cooldown_remaining}),
                    None => json!(null),
                })
                .collect();
            let _ = self.mqtx.send(OutboundMsg::new_s_at(
                "td/all/res",
                "hero",
                "inventory",
                serde_json::json!({"id": hero_entity.id(), "slots": slots}),
                pos_x_f, pos_y_f,
            ));
        }
    }

    fn upgrade_skill(&self, world: &mut World, pd: &InboundMsg) -> Result<(), Error> {
        let slot = pd.d.get("slot").and_then(|v| v.as_str()).unwrap_or("");
        let idx = match Self::slot_to_index(slot) {
            Some(i) => i,
            None => {
                log::warn!("upgrade_skill: 未知 slot '{}'", slot);
                return Ok(());
            }
        };
        let hero_e = match self.find_hero_entity(world, &pd.name) {
            Some(e) => e,
            None => return Ok(()),
        };
        // 成功升級後要 push SkillLearn event；先用 Option 暫存
        let mut learn_info: Option<(String, u8)> = None;
        {
            let mut heroes = world.write_storage::<Hero>();
            if let Some(hero) = heroes.get_mut(hero_e) {
                if hero.skill_points <= 0 {
                    log::info!("upgrade_skill: 無可用技能點 (slot {})", slot);
                    return Ok(());
                }
                let ability_id = match hero.abilities.get(idx) {
                    Some(a) => a.clone(),
                    None => {
                        log::warn!("upgrade_skill: 英雄無 slot {} 技能", slot);
                        return Ok(());
                    }
                };
                let cur = hero.ability_levels.get(&ability_id).copied().unwrap_or(0);
                // Bug fix: 之前用 (cur+1).min(5)，cur=5 時 new_lvl 也是 5
                // 但 skill_points 還是被扣 1（無實際效果）。改成已滿級就拒絕。
                if cur >= 5 {
                    log::info!(
                        "upgrade_skill: slot {} ({}) 已達滿級 5",
                        slot, ability_id
                    );
                    return Ok(());
                }
                let new_lvl = cur + 1;
                hero.ability_levels.insert(ability_id.clone(), new_lvl);
                hero.skill_points -= 1;
                log::info!(
                    "⬆️  技能升級：slot {} ({}) → {} (剩餘技能點 {})",
                    slot, ability_id, new_lvl, hero.skill_points
                );
                learn_info = Some((ability_id, new_lvl.max(1) as u8));
            }
        }
        // 推 SkillLearn event，讓 Passive 技能在 dispatch 時套 on_learn 鉤子
        if let Some((ability_id, new_level)) = learn_info {
            let mut queue = world.write_resource::<crate::scripting::ScriptEventQueue>();
            queue.push(crate::scripting::ScriptEvent::SkillLearn {
                caster: hero_e,
                skill_id: ability_id,
                new_level,
            });
        }
        self.broadcast_hero_update(world, hero_e);
        Ok(())
    }

    fn buy_item(&self, world: &mut World, pd: &InboundMsg) -> Result<(), Error> {
        let item_id = pd.d.get("item_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if item_id.is_empty() {
            return Ok(());
        }
        let hero_e = match self.find_hero_entity(world, &pd.name) {
            Some(e) => e,
            None => return Ok(()),
        };
        // 取出 item config（Arc clone）
        let item_cfg = {
            let reg = world.read_resource::<crate::item::ItemRegistry>();
            match reg.get(&item_id) {
                Some(c) => c,
                None => {
                    log::warn!("buy_item: 未知 item_id '{}'", item_id);
                    return Ok(());
                }
            }
        };
        // 基地範圍檢查（距離 (0,0) < 800）— MVP 簡化
        {
            let positions = world.read_storage::<Pos>();
            if let Some(p) = positions.get(hero_e) {
                // NOTE: distance check at non-sim path (UI proximity); lossy f32 acceptable.
                let (px, py) = p.xy_f32();
                if px * px + py * py > 800.0 * 800.0 {
                    log::info!("buy_item: 不在基地範圍內");
                    return Ok(());
                }
            }
        }
        // 扣金錢 + 消耗組件 + 加到背包
        let mut golds = world.write_storage::<Gold>();
        let mut invs = world.write_storage::<Inventory>();
        let mut effs = world.write_storage::<ItemEffects>();
        let gold_entry = golds.get_mut(hero_e);
        let inv = invs.get_mut(hero_e);
        let eff = effs.get_mut(hero_e);
        if let (Some(gold), Some(inv), Some(eff)) = (gold_entry, inv, eff) {
            // 先尋找並消耗組件槽（若 recipe 不為空）
            let mut to_consume: Vec<usize> = Vec::new();
            for req_id in item_cfg.recipe.iter() {
                if let Some(slot_i) = inv.slots.iter().enumerate().find_map(|(i, s)| match s {
                    Some(inst) if inst.item_id == *req_id && !to_consume.contains(&i) => Some(i),
                    _ => None,
                }) {
                    to_consume.push(slot_i);
                }
            }
            let recipe_satisfied = to_consume.len() == item_cfg.recipe.len();
            if !recipe_satisfied {
                log::info!("buy_item: 組件不足 ({} 需要 {:?})", item_cfg.id, item_cfg.recipe);
                return Ok(());
            }
            if gold.0 < item_cfg.cost {
                log::info!("buy_item: 金錢不足 ({}/{})", gold.0, item_cfg.cost);
                return Ok(());
            }
            for idx in to_consume {
                inv.slots[idx] = None;
            }
            let slot_i = match inv.first_free_slot() {
                Some(i) => i,
                None => {
                    log::info!("buy_item: 背包已滿");
                    return Ok(());
                }
            };
            gold.0 -= item_cfg.cost;
            inv.slots[slot_i] = Some(ItemInstance {
                item_id: item_cfg.id.clone(),
                cooldown_remaining: 0.0,
            });
            eff.dirty = true;
            log::info!(
                "🛒 買入 {} (slot {}) — 剩餘金錢 {}",
                item_cfg.name, slot_i, gold.0
            );
        }
        drop(golds);
        drop(invs);
        drop(effs);
        self.broadcast_hero_update(world, hero_e);
        Ok(())
    }

    fn sell_item(&self, world: &mut World, pd: &InboundMsg) -> Result<(), Error> {
        let slot_i = pd.d.get("slot").and_then(|v| v.as_u64()).unwrap_or(99) as usize;
        if slot_i >= INVENTORY_SLOTS {
            return Ok(());
        }
        let hero_e = match self.find_hero_entity(world, &pd.name) {
            Some(e) => e,
            None => return Ok(()),
        };
        let refund = {
            let invs = world.read_storage::<Inventory>();
            let reg = world.read_resource::<crate::item::ItemRegistry>();
            if let Some(inv) = invs.get(hero_e) {
                if let Some(Some(inst)) = inv.slots.get(slot_i) {
                    reg.get(&inst.item_id).map(|c| crate::item::sell_price(c.cost))
                } else {
                    None
                }
            } else {
                None
            }
        };
        if let Some(refund) = refund {
            let mut golds = world.write_storage::<Gold>();
            let mut invs = world.write_storage::<Inventory>();
            let mut effs = world.write_storage::<ItemEffects>();
            if let (Some(g), Some(inv), Some(eff)) =
                (golds.get_mut(hero_e), invs.get_mut(hero_e), effs.get_mut(hero_e))
            {
                inv.slots[slot_i] = None;
                g.0 += refund;
                eff.dirty = true;
                log::info!("💰 賣出 slot {}，退還 {} 金錢，餘 {}", slot_i, refund, g.0);
            }
        }
        self.broadcast_hero_update(world, hero_e);
        Ok(())
    }

    fn use_item(&self, world: &mut World, pd: &InboundMsg) -> Result<(), Error> {
        let slot_i = pd.d.get("slot").and_then(|v| v.as_u64()).unwrap_or(99) as usize;
        if slot_i >= INVENTORY_SLOTS {
            return Ok(());
        }
        let hero_e = match self.find_hero_entity(world, &pd.name) {
            Some(e) => e,
            None => return Ok(()),
        };

        // 取出裝備 config（需要 active）
        let (item_cfg, can_use) = {
            let invs = world.read_storage::<Inventory>();
            let reg = world.read_resource::<crate::item::ItemRegistry>();
            if let Some(inv) = invs.get(hero_e) {
                if let Some(Some(inst)) = inv.slots.get(slot_i) {
                    let cfg = reg.get(&inst.item_id);
                    let ready = inst.cooldown_remaining <= 0.0;
                    (cfg, ready)
                } else {
                    (None, false)
                }
            } else {
                (None, false)
            }
        };
        let cfg = match item_cfg {
            Some(c) => c,
            None => return Ok(()),
        };
        if !can_use {
            log::info!("use_item: slot {} CD 中", slot_i);
            return Ok(());
        }
        let active = match &cfg.active {
            Some(a) => a.clone(),
            None => {
                log::info!("use_item: slot {} 裝備無主動效果", slot_i);
                return Ok(());
            }
        };

        // 套用效果（MVP 直接操作屬性）
        {
            let mut props = world.write_storage::<CProperty>();
            if let Some(p) = props.get_mut(hero_e) {
                match &active {
                    crate::item::ActiveEffect::Shield { amount, .. } => {
                        let amt_fx = omoba_sim::Fixed64::from_raw((*amount * 1024.0) as i64);
                        let summed = p.hp + amt_fx;
                        p.hp = if summed > p.mhp { p.mhp } else { summed };
                        log::info!("🛡️ 護盾主動 +{} HP", amount);
                    }
                    crate::item::ActiveEffect::RestoreMana { amount } => {
                        // mp 非 CProperty 欄位，改為記錄（未實作 mp tick 的話以 HP 代回簡化）
                        log::info!("💙 回魔主動 +{} MP (MVP 暫未串接 mp)", amount);
                    }
                    crate::item::ActiveEffect::SprintBuff { ms_bonus, duration } => {
                        let bonus_fx = omoba_sim::Fixed64::from_raw((*ms_bonus * 1024.0) as i64);
                        p.msd += bonus_fx;
                        log::info!("💨 疾跑 +{} ms，持續 {}s (MVP 無 buff 結束回收)", ms_bonus, duration);
                    }
                    crate::item::ActiveEffect::DamageReduce { percent, duration } => {
                        log::info!("🛡️ 減傷 {}% {}s (MVP buff 管道尚未接)", percent * 100.0, duration);
                    }
                    crate::item::ActiveEffect::HeadshotNext { bonus_damage } => {
                        log::info!("🎯 下次攻擊 +{} 傷害 (MVP 尚未 hook 到 projectile)", bonus_damage);
                    }
                }
            }
        }

        // 啟動 CD
        {
            let mut invs = world.write_storage::<Inventory>();
            if let Some(inv) = invs.get_mut(hero_e) {
                if let Some(Some(inst)) = inv.slots.get_mut(slot_i) {
                    inst.cooldown_remaining = cfg.cooldown;
                }
            }
        }
        self.broadcast_hero_update(world, hero_e);
        Ok(())
    }

    fn get_screen_area_data(&self, _world: &mut World, _pd: &InboundMsg) -> Result<serde_json::Value, Error> {
        use serde_json::json;
        
        // 實現畫面區域資料獲取邏輯
        // 暫時返回空資料
        Ok(json!({
            "entities": [],
            "terrain": {},
            "effects": []
        }))
    }

    fn update_player_view(&self, _world: &mut World, _pd: &InboundMsg) -> Result<(), Error> {
        // 實現玩家視野更新邏輯
        Ok(())
    }

    /// 獲取資源統計信息
    pub fn get_resource_stats(&self, world: &World) -> ResourceStats {
        let outcomes = world.read_resource::<Vec<Outcome>>();
        
        ResourceStats {
            pending_outcomes: outcomes.len(),
            total_entities: world.entities().join().count(),
            active_systems: 0, // 需要實際統計
        }
    }

    /// 清理過期資源
    pub fn cleanup_expired_resources(&self, _world: &mut World) -> Result<(), Error> {
        // 實現資源清理邏輯
        Ok(())
    }
}

/// 資源統計信息
#[derive(Debug, Clone)]
pub struct ResourceStats {
    /// 待處理結果數量
    pub pending_outcomes: usize,
    /// 總實體數量
    pub total_entities: usize,
    /// 活躍系統數量
    pub active_systems: usize,
}

/// 供多個 payload 廣播 site 共用的 hero.stats JSON builder。
/// 會把 `BuffStore` 身上的 `_bonus` / `_multiplier` 聚合回到 base 值上，讓前端
/// 看到的攻擊力/射程/移速/攻速/護甲/魔抗 都是「實際生效值」。同時附上 buffs
/// 陣列（id + 剩餘秒 + payload）供 UI 顯示。
/// P2 binary-protocol helper: build a prost `HeartbeatTick` for the KCP path.
///
/// Input `hp_snapshot` is a pre-filtered slice of `(entity_id, hp)` pairs
/// (viewport filtering happens at the caller — the full-scan vs per-player
/// logic already lives in `core::send_heartbeat`). HP values are quantized
/// via `fixed_quant` (scale 0.1) to match the shared wire scale.
#[cfg(feature = "kcp")]
pub(crate) fn build_heartbeat_tick(
    tick: u64,
    game_time: f64,
    entity_count: u32,
    hero_count: u32,
    unit_count: u32,
    creep_count: u32,
    render_delay_ms: u32,
    hp_snapshot: &[(u32, f32)],
    pos_snapshot: &[(u32, f32, f32)],
    in_flight_projectiles: &[u32],
) -> crate::transport::kcp_transport::game_proto::HeartbeatTick {
    use crate::transport::kcp_transport::game_proto::{
        Fixed16, HeartbeatEntry, HeartbeatPosEntry, HeartbeatTick, Position16,
    };
    use omoba_core::quant::{fixed_quant, pos_quant};

    let entries: Vec<HeartbeatEntry> = hp_snapshot
        .iter()
        .map(|&(id, hp)| HeartbeatEntry {
            id: id as u64,
            hp: Some(Fixed16 { v_q: fixed_quant(hp) }),
        })
        .collect();

    let pos_entries: Vec<HeartbeatPosEntry> = pos_snapshot
        .iter()
        .map(|&(id, x, y)| HeartbeatPosEntry {
            id: id as u64,
            pos: Some(Position16 {
                x_q: pos_quant(x),
                y_q: pos_quant(y),
            }),
        })
        .collect();

    HeartbeatTick {
        tick,
        game_time,
        entity_count,
        hero_count,
        unit_count,
        creep_count,
        render_delay_ms,
        hp_snapshot: entries,
        pos_snapshot: pos_entries,
        in_flight_projectiles: in_flight_projectiles.to_vec(),
    }
}

// ========================================================================
// P2 full-migration helpers: prost builders for high-volume game events.
// All gated behind the `kcp` feature so non-kcp builds stay cdep-free.
// ========================================================================

#[cfg(feature = "kcp")]
pub(crate) mod proto_build {
    use crate::transport::kcp_transport::game_proto::*;
    use omoba_core::quant::{facing_quant, fixed_quant, pos_quant};

    // P9: re-export EntityKind so call sites can do `proto_build::EntityKind::Hero`.
    pub use crate::transport::kcp_transport::game_proto::EntityKind;

    pub fn pos16(x: f32, y: f32) -> Position16 {
        Position16 { x_q: pos_quant(x), y_q: pos_quant(y) }
    }

    pub fn fx16(v: f32) -> Fixed16 {
        Fixed16 { v_q: fixed_quant(v) }
    }

    pub fn projectile_create(
        id: u32,
        target_id: u32,
        start_x: f32,
        start_y: f32,
        end_x: f32,
        end_y: f32,
        flight_time_ms: u64,
        directional: bool,
        splash_radius: f32,
        hit_radius: f32,
        // Template id from `omoba-template-ids` `ProjectileKindId.0` (sequential u16
        // per projectile_kinds declaration order in Story/templates.json; 0 = UNSPECIFIED).
        // Replaced FNV-1a u32 hash — wire saving ~2 B per event under varint.
        kind_id: u16,
        // P7: pre-declared single-target damage (splash_radius == 0 only);
        // 0 => server will emit creep.H normally on impact.
        damage: f32,
    ) -> ProjectileCreate {
        ProjectileCreate {
            id: id as u64,
            target_id: target_id as u64,
            start_pos: Some(pos16(start_x, start_y)),
            end_pos: Some(pos16(end_x, end_y)),
            flight_time_ms: flight_time_ms.min(u32::MAX as u64) as u32,
            directional,
            splash_radius: Some(fx16(splash_radius)),
            hit_radius: Some(fx16(hit_radius)),
            kind_id: kind_id as u32,
            damage: Some(fx16(damage)),
        }
    }

    pub fn projectile_destroy(id: u32) -> ProjectileDestroy {
        ProjectileDestroy { id: id as u64 }
    }

    pub fn creep_create(
        id: u32,
        x: f32,
        y: f32,
        hp: f32,
        max_hp: f32,
        move_speed: f32,
        // Internal template id ("training_mage", not the Chinese display "訓練法師").
        // Looked up in omoba-template-ids via `creep_by_name` → CreepId (u16).
        // Unknown ids fall back to 0 (UNSPECIFIED); client logs a warning and
        // renders entity_type as the label.
        internal_name: &str,
    ) -> CreepCreate {
        let name_id = omoba_template_ids::creep_by_name(internal_name)
            .map(|c| c.0 as u32)
            .unwrap_or_else(|| {
                log::warn!("creep_create: unknown template id {:?} — emit UNSPECIFIED", internal_name);
                0
            });
        CreepCreate {
            id: id as u64,
            pos: Some(pos16(x, y)),
            hp: Some(fx16(hp)),
            max_hp: Some(fx16(max_hp)),
            move_speed: Some(fx16(move_speed)),
            name_id,
        }
    }

    /// Legacy helper: kept for call sites that don't yet carry velocity info
    /// (e.g. the `handle_creep_stop` "freeze at pos" case). Emits zeros for
    /// the P4 fields, which the client treats as "lerp-only, no extrapolation".
    pub fn creep_move(id: u32, x: f32, y: f32, facing: f32) -> CreepMove {
        CreepMove {
            id: id as u64,
            target: Some(pos16(x, y)),
            facing_q: facing_quant(facing),
            velocity: Some(fx16(0.0)),
            arrival_tick: 0,
            start_pos: Some(pos16(x, y)),
            start_tick: 0,
        }
    }

    /// P4 full builder: includes velocity + arrival_tick + start_pos + start_tick
    /// for client-side extrapolation. `tick_dt` is the server tick duration
    /// (1.0 / TPS). `arrival_tick` is computed relative to `start_tick`; if
    /// `velocity` is zero or the distance is zero we return `start_tick`
    /// (client will lock at target immediately).
    pub fn creep_move_full(
        id: u32,
        target_x: f32,
        target_y: f32,
        facing: f32,
        velocity: f32,
        start_x: f32,
        start_y: f32,
        start_tick: u64,
        tick_dt: f32,
    ) -> CreepMove {
        let dx = target_x - start_x;
        let dy = target_y - start_y;
        let dist = (dx * dx + dy * dy).sqrt();
        let arrival_tick = if velocity > f32::EPSILON && dist > f32::EPSILON && tick_dt > f32::EPSILON {
            start_tick + ((dist / velocity / tick_dt).ceil() as u64)
        } else {
            start_tick
        };
        CreepMove {
            id: id as u64,
            target: Some(pos16(target_x, target_y)),
            facing_q: facing_quant(facing),
            velocity: Some(fx16(velocity)),
            arrival_tick,
            start_pos: Some(pos16(start_x, start_y)),
            start_tick,
        }
    }

    /// Default `creep_hp` — kind defaults to `Creep` (the most common case).
    /// Use `creep_hp_with_kind` when emitting for hero / unit / generic entity.
    pub fn creep_hp(id: u32, hp: f32) -> CreepHp {
        CreepHp {
            id: id as u64,
            hp: Some(fx16(hp)),
            kind: EntityKind::Creep as i32,
        }
    }

    /// P9: explicit-kind variant for HP updates (hero / unit / entity).
    pub fn creep_hp_with_kind(id: u32, hp: f32, kind: EntityKind) -> CreepHp {
        CreepHp {
            id: id as u64,
            hp: Some(fx16(hp)),
            kind: kind as i32,
        }
    }

    pub fn creep_slow(id: u32, move_speed: f32) -> CreepSlow {
        CreepSlow {
            id: id as u64,
            move_speed: Some(fx16(move_speed)),
        }
    }

    pub fn creep_stall(id: u32, x: f32, y: f32, facing: f32) -> CreepStall {
        CreepStall {
            id: id as u64,
            pos: Some(pos16(x, y)),
            facing_q: facing_quant(facing),
        }
    }

    pub fn entity_facing(id: u32, facing: f32) -> EntityFacing {
        EntityFacing {
            id: id as u64,
            facing_q: facing_quant(facing),
        }
    }

    /// Default `entity_death` — kind defaults to `Creep` (most common emit site).
    /// Use `entity_death_with_kind` when emitting for hero/unit/tower/projectile.
    pub fn entity_death(id: u32) -> EntityDeath {
        EntityDeath {
            id: id as u64,
            kind: EntityKind::Creep as i32,
        }
    }

    /// P9: explicit-kind variant for death events.
    pub fn entity_death_with_kind(id: u32, kind: EntityKind) -> EntityDeath {
        EntityDeath {
            id: id as u64,
            kind: kind as i32,
        }
    }

    /// P9: minimal hero create — for visibility-diff spawn. Hero static + hot
    /// payloads should be pushed alongside (or shortly after) so omfx can
    /// hydrate the rest.
    pub fn hero_create(
        id: u32,
        x: f32,
        y: f32,
        hp: f32,
        max_hp: f32,
        name: &str,
        title: &str,
    ) -> HeroCreate {
        HeroCreate {
            id: id as u64,
            pos: Some(pos16(x, y)),
            hp: Some(fx16(hp)),
            max_hp: Some(fx16(max_hp)),
            name: name.to_string(),
            title: title.to_string(),
        }
    }

    /// P9: generic unit create.
    pub fn unit_create(
        id: u32,
        x: f32,
        y: f32,
        hp: f32,
        max_hp: f32,
        name: &str,
    ) -> UnitCreate {
        UnitCreate {
            id: id as u64,
            pos: Some(pos16(x, y)),
            hp: Some(fx16(hp)),
            max_hp: Some(fx16(max_hp)),
            name: name.to_string(),
        }
    }

    /// P9: build a `LegacyJson` wrapper for low-frequency irregular events
    /// (init/ack/reject/inventory). Steady-state hot paths must NOT use this —
    /// migrate to a typed variant instead.
    pub fn legacy_json(t: &str, a: &str, v: &serde_json::Value) -> LegacyJson {
        LegacyJson {
            msg_type: t.to_string(),
            action: a.to_string(),
            data_json: serde_json::to_vec(v).unwrap_or_default(),
        }
    }

    pub fn tower_create(
        id: u32,
        x: f32,
        y: f32,
        hp: f32,
        max_hp: f32,
        kind: &str,
        name: &str,
    ) -> TowerCreate {
        TowerCreate {
            id: id as u64,
            pos: Some(pos16(x, y)),
            hp: Some(fx16(hp)),
            max_hp: Some(fx16(max_hp)),
            kind: kind.to_string(),
            name: name.to_string(),
        }
    }

    pub fn tower_upgrade(id: u32, levels: [u8; 3]) -> TowerUpgrade {
        TowerUpgrade {
            id: id as u64,
            levels: vec![levels[0] as u32, levels[1] as u32, levels[2] as u32],
        }
    }

    pub fn buff_add(
        entity_id: u32,
        buff_id: &str,
        duration_sec: f32,
        payload: &serde_json::Value,
    ) -> BuffAdd {
        let remaining_ms = if duration_sec.is_infinite() || duration_sec > 65.535 {
            0xFFFF
        } else {
            (duration_sec * 1000.0).clamp(0.0, 65535.0) as u32
        };
        BuffAdd {
            entity_id: entity_id as u64,
            buff_id: buff_id.to_string(),
            remaining_ms,
            payload_json: payload.to_string(),
        }
    }

    pub fn buff_remove(entity_id: u32, buff_id: &str) -> BuffRemove {
        BuffRemove {
            entity_id: entity_id as u64,
            buff_id: buff_id.to_string(),
        }
    }

    pub fn game_round(round: u32, total: u32, is_running: bool) -> GameRound {
        GameRound { round, total, is_running }
    }

    pub fn game_lives(lives: i32) -> GameLives {
        GameLives { lives }
    }

    pub fn game_end(winner: &str) -> GameEnd {
        GameEnd { winner: winner.to_string() }
    }

    pub fn game_explosion(x: f32, y: f32, radius: f32, duration_sec: f32) -> GameExplosion {
        let duration_ms = (duration_sec * 1000.0).clamp(0.0, u32::MAX as f32) as u32;
        GameExplosion {
            pos: Some(pos16(x, y)),
            radius: Some(fx16(radius)),
            duration_ms,
        }
    }

    /// P3: build `HeroStatic` prost — cold hero metadata (name/title/base_stats/
    /// abilities/level/xp/skill_points/ability_levels). Pushed on create /
    /// level up / ability learn (低頻）。
    pub fn hero_static(
        hero_entity: specs::Entity,
        h: &crate::comp::Hero,
    ) -> HeroStatic {
        let ability_ids: Vec<String> = h.abilities.iter().cloned().collect();
        // 固定順序輸出 ability_levels：照 abilities[] 順序，避免 HashMap 不決定性。
        let ability_levels: Vec<AbilityLevelPair> = h.abilities.iter().map(|id| {
            let cur = *h.ability_levels.get(id).unwrap_or(&0);
            // Hero::learn_ability 中 cap 在 4；R-前綴 ult 額外受 (level/6).max(1) 限
            let max = if id.starts_with('R') { ((h.level as u32) / 6).max(1) } else { 4 };
            AbilityLevelPair { cur: cur.max(0) as u32, max }
        }).collect();

        HeroStatic {
            id: hero_entity.id() as u64,
            name: h.name.clone(),
            title: h.title.clone(),
            base_str: h.strength.max(0) as u32,
            base_agi: h.agility.max(0) as u32,
            base_int: h.intelligence.max(0) as u32,
            ability_ids,
            level: h.level.max(0) as u32,
            xp: h.experience.max(0) as u32,
            xp_next: h.experience_to_next.max(0) as u32,
            skill_points: h.skill_points.max(0) as u32,
            ability_levels,
        }
    }

    /// P3: build `HeroHot` prost — hot hero state (HP/mana/damage/armor/
    /// resists/speed/range/interval + gold + buff snapshot). Pushed every 0.3s.
    ///
    /// `buff_store` iter_for(hero_entity) 會把 hero 身上所有 buff 打包；
    /// `remaining.is_infinite()` → 0xFFFF sentinel（與 `buff_add` builder 對齊）。
    pub fn hero_hot(
        hero_entity: specs::Entity,
        _h: &crate::comp::Hero,
        gold: i32,
        hp: f32,
        mhp: f32,
        armor_base: f32,
        magic_resist_base: f32,
        move_speed_base: f32,
        attack_damage_base: f32,
        attack_interval_base: f32,
        attack_range_base: f32,
        buff_store: &crate::ability_runtime::BuffStore,
    ) -> HeroHot {
        // 與 build_hero_stats_payload 一致的聚合路徑（讓前端看到實際生效值）
        // Phase 1c.3: UnitStats final_* now returns Fixed64; wire format remains f32.
        let stats = crate::ability_runtime::UnitStats::from_refs(buff_store, false);
        let attack_damage_base_fx = omoba_sim::Fixed64::from_raw((attack_damage_base * 1024.0) as i64);
        let attack_range_base_fx = omoba_sim::Fixed64::from_raw((attack_range_base * 1024.0) as i64);
        let move_speed_base_fx = omoba_sim::Fixed64::from_raw((move_speed_base * 1024.0) as i64);
        let armor_base_fx = omoba_sim::Fixed64::from_raw((armor_base * 1024.0) as i64);
        let magic_resist_base_fx = omoba_sim::Fixed64::from_raw((magic_resist_base * 1024.0) as i64);
        let atk_dmg_eff = stats.final_atk(attack_damage_base_fx, hero_entity).to_f32_for_render();
        let atk_rng_eff = stats.final_attack_range(attack_range_base_fx, hero_entity).to_f32_for_render();
        let asd_mult = stats.final_attack_speed_mult(hero_entity).to_f32_for_render();
        let atk_int_eff = if asd_mult > 0.0 { attack_interval_base / asd_mult } else { attack_interval_base };
        let msd_eff = stats.final_move_speed(move_speed_base_fx, hero_entity).to_f32_for_render();
        let armor_eff = stats.final_armor(armor_base_fx, hero_entity).to_f32_for_render();
        let magic_resist_eff = stats.final_magic_resist(magic_resist_base_fx, hero_entity).to_f32_for_render();

        let buffs: Vec<BuffSnapshot> = buff_store
            .iter_for(hero_entity)
            .map(|(id, entry)| {
                // Phase 1c.3: BuffEntry.remaining is Fixed64 — wire as ms u32 sentinel.
                let remaining_f = entry.remaining.to_f32_for_render();
                let remaining_ms = if remaining_f.is_infinite() || remaining_f > 65.535 {
                    0xFFFF
                } else {
                    (remaining_f * 1000.0).clamp(0.0, 65535.0) as u32
                };
                BuffSnapshot {
                    buff_id: id.to_string(),
                    remaining_ms,
                    payload_json: entry.payload.to_string(),
                }
            })
            .collect();

        HeroHot {
            id: hero_entity.id() as u64,
            hp: Some(fx16(hp)),
            max_hp: Some(fx16(mhp)),
            // 目前 omb 尚未實裝 hero mana tracking — 以 0 暫代；前端 omfx hero.stats 也沒讀取 mana
            mana: Some(fx16(0.0)),
            max_mana: Some(fx16(0.0)),
            gold: gold.max(0) as u32,
            attack_damage: Some(fx16(atk_dmg_eff)),
            armor: Some(fx16(armor_eff)),
            magic_resist: Some(fx16(magic_resist_eff)),
            move_speed: Some(fx16(msd_eff)),
            attack_range: Some(fx16(atk_rng_eff)),
            attack_interval: Some(fx16(atk_int_eff)),
            buffs,
        }
    }
}

/// P3: JSON 版本（非 kcp 傳輸用）— 原 hero.stats 的全 20 欄位 payload。
/// kcp path 改走 `build_hero_hot_msg` / `build_hero_static_msg` 直接 prost 編碼。
#[cfg(not(feature = "kcp"))]
pub(crate) fn build_hero_stats_payload(
    hero_entity: specs::Entity,
    h: &crate::comp::Hero,
    gold: i32,
    hp: f32,
    mhp: f32,
    armor_base: f32,
    magic_resist_base: f32,
    move_speed_base: f32,
    attack_damage_base: f32,
    attack_interval_base: f32,
    attack_range_base: f32,
    bullet_speed: f32,
    lives: i32,
    buff_store: &crate::ability_runtime::BuffStore,
) -> serde_json::Value {
    use serde_json::json;
    // Phase 1c.3: UnitStats final_* now Fixed64; wire format remains f32.
    let stats = crate::ability_runtime::UnitStats::from_refs(buff_store, false);
    let attack_damage_base_fx = omoba_sim::Fixed64::from_raw((attack_damage_base * 1024.0) as i64);
    let attack_range_base_fx = omoba_sim::Fixed64::from_raw((attack_range_base * 1024.0) as i64);
    let move_speed_base_fx = omoba_sim::Fixed64::from_raw((move_speed_base * 1024.0) as i64);
    let armor_base_fx = omoba_sim::Fixed64::from_raw((armor_base * 1024.0) as i64);
    let magic_resist_base_fx = omoba_sim::Fixed64::from_raw((magic_resist_base * 1024.0) as i64);
    let atk_dmg_eff = stats.final_atk(attack_damage_base_fx, hero_entity).to_f32_for_render();
    let atk_rng_eff = stats.final_attack_range(attack_range_base_fx, hero_entity).to_f32_for_render();
    let asd_mult = stats.final_attack_speed_mult(hero_entity).to_f32_for_render();
    let atk_int_eff = if asd_mult > 0.0 { attack_interval_base / asd_mult } else { attack_interval_base };
    let msd_eff = stats.final_move_speed(move_speed_base_fx, hero_entity).to_f32_for_render();
    let armor_eff = stats.final_armor(armor_base_fx, hero_entity).to_f32_for_render();
    let magic_resist_eff = stats.final_magic_resist(magic_resist_base_fx, hero_entity).to_f32_for_render();

    let buffs: Vec<serde_json::Value> = buff_store
        .iter_for(hero_entity)
        .map(|(id, entry)| {
            // Phase 1c.3: BuffEntry.remaining is Fixed64; treat raw i32::MAX as infinite.
            let remaining = if entry.remaining.raw() == i32::MAX {
                -1.0_f32
            } else {
                entry.remaining.to_f32_for_render()
            };
            json!({
                "id": id,
                "remaining": remaining,
                "payload": entry.payload,
            })
        })
        .collect();

    json!({
        "id": hero_entity.id(),
        "name": h.name,
        "title": h.title,
        "level": h.level,
        "xp": h.experience,
        "xp_next": h.experience_to_next,
        "skill_points": h.skill_points,
        "ability_levels": h.ability_levels,
        "abilities": h.abilities,
        "strength": h.strength,
        "agility": h.agility,
        "intelligence": h.intelligence,
        "primary_attribute": format!("{:?}", h.primary_attribute).to_lowercase(),
        "gold": gold,
        "hp": hp,
        "max_hp": mhp,
        "armor": armor_eff,
        "magic_resist": magic_resist_eff,
        "move_speed": msd_eff,
        "attack_damage": atk_dmg_eff,
        "attack_interval": atk_int_eff,
        "attack_range": atk_rng_eff,
        "bullet_speed": bullet_speed,
        "lives": lives,
        "buffs": buffs,
    })
}

/// P3 (kcp only): 建構 `HeroHot` prost OutboundMsg — 0.3s tick + 狀態變化事件共用。
#[cfg(feature = "kcp")]
pub(crate) fn build_hero_hot_msg(
    hero_entity: specs::Entity,
    h: &crate::comp::Hero,
    gold: i32,
    hp: f32,
    mhp: f32,
    armor_base: f32,
    magic_resist_base: f32,
    move_speed_base: f32,
    attack_damage_base: f32,
    attack_interval_base: f32,
    attack_range_base: f32,
    buff_store: &crate::ability_runtime::BuffStore,
    pos: vek::Vec2<f32>,
) -> OutboundMsg {
    let hot = proto_build::hero_hot(
        hero_entity, h, gold, hp, mhp, armor_base, magic_resist_base, move_speed_base,
        attack_damage_base, attack_interval_base, attack_range_base, buff_store,
    );
    // json_fallback 僅供 dedupe 的 (t,a,id) 識別；真實 payload 走 typed。
    let fb = serde_json::json!({ "id": hero_entity.id() });
    OutboundMsg::new_typed_at(
        "td/all/res", "hero", "hot",
        crate::transport::TypedOutbound::HeroHot(hot), fb, pos.x, pos.y,
    )
}

/// P3 (kcp only): 建構 `HeroStatic` prost OutboundMsg — create / level up / ability learn。
#[cfg(feature = "kcp")]
pub(crate) fn build_hero_static_msg(
    hero_entity: specs::Entity,
    h: &crate::comp::Hero,
    pos: vek::Vec2<f32>,
) -> OutboundMsg {
    let st = proto_build::hero_static(hero_entity, h);
    let fb = serde_json::json!({ "id": hero_entity.id() });
    OutboundMsg::new_typed_at(
        "td/all/res", "hero", "static",
        crate::transport::TypedOutbound::HeroStatic(st), fb, pos.x, pos.y,
    )
}