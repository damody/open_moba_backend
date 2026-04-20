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
        use specs::{Builder, WorldExt};
        
        // 從 PlayerData.d 中解析位置信息
        if let Ok(data) = serde_json::from_value::<serde_json::Map<String, serde_json::Value>>(pd.d.clone()) {
            if let (Some(x_val), Some(y_val)) = (data.get("x"), data.get("y")) {
                if let (Some(x), Some(y)) = (x_val.as_f64(), y_val.as_f64()) {
                    // 創建塔的基本屬性
                    let tower_pos = Pos(Vec2::new(x as f32, y as f32));
                    let tower_vel = Vel(Vec2::new(0.0, 0.0));
                    
                    // 創建塔組件
                    let tower = Tower::new();
                    let tower_property = TProperty::new(100.0, 1, 200.0); // HP, 等級, 建造成本
                    let tower_attack = TAttack::new(50.0, 1.5, 300.0, 800.0); // 攻擊力, 攻速, 射程, 彈速
                    
                    // 創建塔實體
                    let _tower_entity = world.create_entity()
                        .with(tower_pos)
                        .with(tower_vel)
                        .with(tower)
                        .with(tower_property)
                        .with(tower_attack)
                        .build();
                        
                    // 添加到結果中通知其他系統
                    let mut outcomes = world.write_resource::<Vec<Outcome>>();
                    outcomes.push(Outcome::Tower { 
                        pos: Vec2::new(x as f32, y as f32), 
                        td: TowerData { 
                            tpty: tower_property, 
                            tatk: tower_attack 
                        } 
                    });
                }
            }
        }
        
        Ok(())
    }

    fn upgrade_tower(&self, _world: &mut World, _pd: &InboundMsg) -> Result<(), Error> {
        // 實現塔升級邏輯
        Ok(())
    }

    fn sell_tower(&self, _world: &mut World, _pd: &InboundMsg) -> Result<(), Error> {
        // 實現塔出售邏輯
        Ok(())
    }

    fn move_player(&self, world: &mut World, pd: &InboundMsg) -> Result<(), Error> {
        use vek::Vec2;

        // 解析目標位置
        let x = pd.d.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
        let y = pd.d.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;

        // 找到匹配的英雄實體
        let target_entity = {
            let entities = world.entities();
            let heroes = world.read_storage::<Hero>();
            let mut found = None;
            for (e, hero) in (&entities, &heroes).join() {
                if hero.name == pd.name || pd.name.is_empty() {
                    found = Some(e);
                    break;
                }
            }
            // 若沒找到名稱匹配的，取第一個英雄
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
            let _ = move_targets.insert(entity, MoveTarget(Vec2::new(x, y)));
            log::info!("設定英雄移動目標: ({}, {})", x, y);
        } else {
            log::warn!("找不到英雄實體: {}", pd.name);
        }

        Ok(())
    }

    fn player_attack(&self, _world: &mut World, _pd: &InboundMsg) -> Result<(), Error> {
        // 實現玩家攻擊邏輯
        Ok(())
    }

    fn use_skill(&self, _world: &mut World, _pd: &InboundMsg) -> Result<(), Error> {
        // 實現技能使用邏輯
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
        let positions = world.read_storage::<Pos>();

        let hero = heroes.get(hero_entity);
        let gold = golds.get(hero_entity).map(|g| g.0).unwrap_or(0);
        let pos = positions.get(hero_entity).map(|p| p.0).unwrap_or(vek::Vec2::zero());
        if let Some(h) = hero {
            let (hp, mhp) = props
                .get(hero_entity)
                .map(|p| (p.hp, p.mhp))
                .unwrap_or((0.0, 0.0));
            let payload = json!({
                "id": hero_entity.id(),
                "level": h.level,
                "xp": h.experience,
                "xp_next": h.experience_to_next,
                "skill_points": h.skill_points,
                "ability_levels": h.ability_levels,
                "abilities": h.abilities,
                "gold": gold,
                "hp": hp,
                "max_hp": mhp,
            });
            let _ = self.mqtx.send(OutboundMsg::new_s_at(
                "td/all/res", "hero", "stats", payload, pos.x, pos.y,
            ));
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
                pos.x, pos.y,
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
            let new_lvl = (cur + 1).min(5);
            hero.ability_levels.insert(ability_id.clone(), new_lvl);
            hero.skill_points -= 1;
            log::info!(
                "⬆️  技能升級：slot {} ({}) → {} (剩餘技能點 {})",
                slot, ability_id, new_lvl, hero.skill_points
            );
        }
        drop(heroes);
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
                if p.0.distance_squared(vek::Vec2::new(0.0, 0.0)) > 800.0 * 800.0 {
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
                        p.hp = (p.hp + amount).min(p.mhp);
                        log::info!("🛡️ 護盾主動 +{} HP", amount);
                    }
                    crate::item::ActiveEffect::RestoreMana { amount } => {
                        // mp 非 CProperty 欄位，改為記錄（未實作 mp tick 的話以 HP 代回簡化）
                        log::info!("💙 回魔主動 +{} MP (MVP 暫未串接 mp)", amount);
                    }
                    crate::item::ActiveEffect::SprintBuff { ms_bonus, duration } => {
                        p.msd += ms_bonus;
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