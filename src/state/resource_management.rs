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
        // TowerCreate broadcast 已砍 — omfx snapshot.entities 自然包含
        // 新 spawn 的 Tower entity，render-side TD build menu 從
        // tower_templates Arc 拿 metadata（sim_runner.rs:88）

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

        // GameRound broadcast 已砍 — omfx HUD 從 SimWorldSnapshot.round /
        // total_rounds / round_is_running 讀取（sim_runner.rs:57-67）
        let _ = (round, total);
        Ok(())
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

        // 10. TowerUpgrade broadcast 已砍 — omfx 從
        // EntityRenderData.upgrade_levels: Option<[u8; 3]> 讀
        // (sim_runner.rs:180)，render-side pip 自動反映新 levels
        let _ = new_levels;

        log::info!("⬆️ TD 升級塔 id={} path={} → L{} ({}) cost={}",
            tower_id_u32, path, next_level, def.name, def.cost);

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

        // 刪塔走 Outcome::EntityRemoved 通道 — process_outcomes 統一處理
        // entities().delete() + RemovedEntitiesQueue push，render 端從
        // snapshot.removed_entity_ids 自動清理 scene node
        world.write_resource::<Vec<crate::comp::Outcome>>()
            .push(crate::comp::Outcome::EntityRemoved { entity: target_entity });

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

    /// Hero broadcast 已砍 — omfx 從 SimWorldSnapshot.entities[].hero_ext
    /// 讀完整 HeroStatsExt（armor/atk/range/msd/asd/inventory/ability_levels
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

}

