use std::collections::BTreeMap;
use failure::Error;
use serde_json::json;
use specs::{World, WorldExt, Entity, Builder, storage::{WriteStorage, ReadStorage}};

use crate::comp::*;
use crate::transport::OutboundMsg;
use crate::Outcome;
use crate::Projectile;

pub struct GameProcessor;

impl GameProcessor {
    pub fn process_outcomes(ecs: &mut World, mqtx: &crossbeam_channel::Sender<OutboundMsg>) -> Result<(), Error> {
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
                    Outcome::CreepLeaked { ent } => {
                        remove_uids.push(ent);
                        Self::handle_creep_leaked(ecs, mqtx, ent)?;
                    }
                    Outcome::ApplySlow { target, factor, duration } => {
                        Self::handle_apply_slow(ecs, target, factor, duration)?;
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
        mqtx: &crossbeam_channel::Sender<OutboundMsg>,
        entity: Entity
    ) -> Result<(), Error> {
        // 只有敵方基地死亡才算玩家勝（我方基地雖有 IsBase，不觸發勝負）
        let is_enemy_base = {
            let is_bases = ecs.read_storage::<IsBase>();
            let factions = ecs.read_storage::<Faction>();
            is_bases.get(entity).is_some()
                && factions
                    .get(entity)
                    .map(|f| f.faction_id == FactionType::Enemy)
                    .unwrap_or(false)
        };

        // 若死者有 Bounty → 將金錢/經驗分給最近的友方英雄
        Self::distribute_bounty(ecs, next_outcomes, mqtx, entity);

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
            mqtx.send(OutboundMsg::new_s("td/all/res", entity_type, "D", json!({"id": entity.id()})));
        }

        if is_enemy_base {
            // 敵方基地被擊毀 → 玩家勝利
            log::info!("🏆 敵方基地 entity {:?} destroyed — emitting game.end", entity);
            mqtx.send(OutboundMsg::new_s(
                "td/all/res",
                "game",
                "end",
                json!({"winner": "player", "base_entity_id": entity.id()}),
            ));
        }
        Ok(())
    }
    
    /// 將 Bounty 分配給最近的友方英雄（MVP 以玩家陣營為友方）
    fn distribute_bounty(
        ecs: &mut World,
        next_outcomes: &mut Vec<Outcome>,
        mqtx: &crossbeam_channel::Sender<OutboundMsg>,
        dead: Entity,
    ) {
        use serde_json::json;
        let bounty = match ecs.read_storage::<Bounty>().get(dead).copied() {
            Some(b) => b,
            None => return,
        };
        let dead_pos = match ecs.read_storage::<Pos>().get(dead).map(|p| p.0) {
            Some(p) => p,
            None => return,
        };
        // 取出死者陣營（用於敵友判定，避免誤給自己人死亡獎勵）
        let dead_faction = ecs.read_storage::<Faction>().get(dead).cloned();

        // 找出最近、與死者敵對的 Player 英雄
        let (hero_e, _) = {
            let entities = ecs.entities();
            let heroes = ecs.read_storage::<Hero>();
            let factions = ecs.read_storage::<Faction>();
            let positions = ecs.read_storage::<Pos>();
            use specs::Join;
            let mut best: Option<(Entity, f32)> = None;
            for (e, _h, f, p) in (&entities, &heroes, &factions, &positions).join() {
                if f.faction_id != FactionType::Player {
                    continue;
                }
                if let Some(ref df) = dead_faction {
                    if !f.is_hostile_to(df) {
                        continue; // 同隊死亡不給賞金
                    }
                }
                let d2 = (p.0 - dead_pos).magnitude_squared();
                if d2 > 1200.0 * 1200.0 {
                    continue;
                }
                if best.map(|(_, d)| d2 < d).unwrap_or(true) {
                    best = Some((e, d2));
                }
            }
            match best {
                Some(x) => x,
                None => return,
            }
        };

        // 加金錢
        {
            let mut golds = ecs.write_storage::<Gold>();
            if let Some(g) = golds.get_mut(hero_e) {
                g.0 += bounty.gold;
            }
        }
        // 給經驗（透過 Hero::add_experience 處理升級與技能點）
        let leveled_up = {
            let mut heroes = ecs.write_storage::<Hero>();
            if let Some(h) = heroes.get_mut(hero_e) {
                let before = h.level;
                let _ = h.add_experience(bounty.exp);
                h.level != before
            } else {
                false
            }
        };

        // 廣播 hero.stats
        let lives = ecs.read_resource::<PlayerLives>().0;
        let hero_stats_payload = {
            let heroes = ecs.read_storage::<Hero>();
            let golds = ecs.read_storage::<Gold>();
            let props = ecs.read_storage::<CProperty>();
            let positions = ecs.read_storage::<Pos>();
            let h = heroes.get(hero_e);
            let g = golds.get(hero_e).map(|g| g.0).unwrap_or(0);
            let (hp, mhp) = props.get(hero_e).map(|p| (p.hp, p.mhp)).unwrap_or((0.0, 0.0));
            let p = positions.get(hero_e).map(|p| p.0).unwrap_or(vek::Vec2::zero());
            h.map(|h| {
                (
                    json!({
                        "id": hero_e.id(),
                        "level": h.level,
                        "xp": h.experience,
                        "xp_next": h.experience_to_next,
                        "skill_points": h.skill_points,
                        "ability_levels": h.ability_levels,
                        "abilities": h.abilities,
                        "gold": g,
                        "hp": hp,
                        "max_hp": mhp,
                        "lives": lives,
                    }),
                    p,
                )
            })
        };
        if let Some((payload, pos)) = hero_stats_payload {
            let _ = mqtx.send(OutboundMsg::new_s_at(
                "td/all/res", "hero", "stats", payload, pos.x, pos.y,
            ));
        }
        if leveled_up {
            log::info!("🎉 hero entity {:?} 升級！", hero_e);
        }
    }

    fn handle_projectile(
        ecs: &mut World, 
        mqtx: &crossbeam_channel::Sender<OutboundMsg>, 
        pos: vek::Vec2<f32>, 
        source: Option<Entity>, 
        target: Option<Entity>
    ) -> Result<(), Error> {
        let source_entity = source.ok_or_else(|| failure::err_msg("Missing source entity"))?;
        let target_entity = target.ok_or_else(|| failure::err_msg("Missing target entity"))?;

        // 讀取 TowerKind（TD 模式塔才有），用來決定 splash/slow 參數。
        // 非 TD 塔（英雄、MOBA 塔）沒有此 Component，走單體傷害。
        let tower_kind: Option<crate::comp::TowerKind> = {
            let kinds = ecs.read_storage::<crate::comp::TowerKind>();
            kinds.get(source_entity).copied()
        };

        let (msd, p2, atk_phys) = {
            let positions = ecs.read_storage::<Pos>();
            let tproperty = ecs.read_storage::<TAttack>();

            let _p1 = positions.get(source_entity).ok_or_else(|| failure::err_msg("Source position not found"))?;
            let p2 = positions.get(target_entity).ok_or_else(|| failure::err_msg("Target position not found"))?;
            let tp = tproperty.get(source_entity).ok_or_else(|| failure::err_msg("Source attack properties not found"))?;
            (tp.bullet_speed, p2.0, tp.atk_physic.v)
        };

        // 命中由 projectile_tick 的距離判定決定（target 接近時 step >= dist 即命中）。
        // time_left 為安全閥：flight_time_s * 3 + 3 秒，允許高速單位拖著子彈移動。
        let move_speed = msd as f32;
        let initial_dist = (p2 - pos).magnitude();
        let flight_time_s: f32 = if move_speed > 0.0 {
            (initial_dist / move_speed).max(0.01)
        } else {
            0.01
        };
        let safety_time_left = flight_time_s * 3.0 + 3.0;

        // 依塔種決定 AoE 半徑與減速（ice）
        let (splash_radius, slow_factor, slow_duration) = match tower_kind {
            Some(kind) => {
                let tpl = kind.template();
                (tpl.splash_radius, tpl.slow_factor, tpl.slow_duration)
            }
            None => (0.0, 0.0, 0.0),
        };

        let ntarget = target_entity.id();
        let e = ecs.create_entity()
            .with(Pos(pos))
            .with(Projectile {
                time_left: safety_time_left,
                owner: source_entity.clone(),
                tpos: p2,
                target: target,
                radius: splash_radius,
                msd: msd,
                damage_phys: atk_phys,
                damage_magi: 0.0,
                damage_real: 0.0,
                slow_factor,
                slow_duration,
            })
            .build();

        // 前端 flight_time_ms 用於 pursuit 動畫；damage 由後端 "H" 事件授權（不再 optimistic）。
        let flight_time_ms: u64 = (flight_time_s * 1000.0).max(1.0) as u64;
        let pjs = json!({
            "id": e.id(),
            "source_id": source_entity.id(),
            "target_id": ntarget,
            "start_pos": { "x": pos.x, "y": pos.y },
            "end_pos":   { "x": p2.x, "y": p2.y },
            "move_speed": move_speed,
            "flight_time_ms": flight_time_ms,
            "damage": atk_phys,
        });

        mqtx.try_send(OutboundMsg::new_s_at("td/all/res", "projectile", "C", pjs, pos.x, pos.y));
        Ok(())
    }
    
    fn handle_creep_spawn(ecs: &mut World, mqtx: &crossbeam_channel::Sender<OutboundMsg>, cd: CreepData) -> Result<(), Error> {
        let display_name = cd.creep.label.clone().unwrap_or_else(|| cd.creep.name.clone());
        let creep_name = cd.creep.name.clone();
        let hp = cd.cdata.hp;
        let mhp = cd.cdata.mhp;
        let msd = cd.cdata.msd;
        let pos = cd.pos;
        // 依 creep 名稱決定獎勵（MVP 簡版）
        let bounty = match creep_name.as_str() {
            "melee_minion" => Bounty { gold: 18, exp: 55 },
            "ranged_minion" => Bounty { gold: 15, exp: 45 },
            "siege_minion" => Bounty { gold: 40, exp: 110 },
            // 我方 creep 死亡不給 bounty
            n if n.starts_with("ally_") => Bounty { gold: 0, exp: 0 },
            _ => Bounty { gold: 10, exp: 25 },
        };
        // 陣營：依 CreepData.faction_name 決定，空字串 → Enemy（舊 map 相容）
        let faction = match cd.faction_name.as_str() {
            "Player" | "player" => Faction::new(FactionType::Player, 0),
            _ => Faction::new(FactionType::Enemy, 1),
        };
        let turn_speed_rad = cd.turn_speed_deg.to_radians();
        let e = ecs.create_entity()
            .with(Pos(cd.pos))
            .with(cd.creep)
            .with(cd.cdata)
            .with(faction)
            .with(bounty)
            .with(Facing(0.0))
            .with(TurnSpeed(turn_speed_rad))
            .build();
        // Payload shape matches client expectations (top-level position/hp/max_hp)
        let payload = json!({
            "entity_id": e.id(),
            "id": e.id(),
            "name": display_name,
            "position": { "x": pos.x, "y": pos.y },
            "hp": hp,
            "max_hp": mhp,
            "move_speed": msd,
        });
        mqtx.try_send(OutboundMsg::new_s_at("td/all/res", "creep", "C", payload, pos.x, pos.y));
        Ok(())
    }

    /// TD 模式（Ice 塔命中時）：附加或刷新 SlowBuff 到目標 creep。
    /// 若目標已有 buff，取較強（factor 較小）的 factor；remaining 取較長。
    fn handle_apply_slow(
        ecs: &mut World,
        target: Entity,
        factor: f32,
        duration: f32,
    ) -> Result<(), Error> {
        let mut slow_buffs = ecs.write_storage::<SlowBuff>();
        let existing = slow_buffs.get(target).copied();
        let (new_factor, new_remaining) = match existing {
            Some(b) => (b.factor.min(factor), b.remaining.max(duration)),
            None => (factor, duration),
        };
        slow_buffs.insert(target, SlowBuff {
            factor: new_factor,
            remaining: new_remaining,
        }).ok();
        Ok(())
    }

    /// TD 模式：小兵漏怪到終點。扣 `PlayerLives` 1、廣播 `game/lives` 與 entity delete。
    /// 若生命歸零再廣播 `game/end`（遊戲結束）。
    fn handle_creep_leaked(
        ecs: &mut World,
        mqtx: &crossbeam_channel::Sender<OutboundMsg>,
        entity: Entity,
    ) -> Result<(), Error> {
        let remaining = {
            let mut lives = ecs.write_resource::<PlayerLives>();
            lives.0 = (lives.0 - 1).max(0);
            lives.0
        };
        log::info!("💔 小兵漏網！玩家生命 {} (entity={:?})", remaining, entity);

        // 廣播 entity delete 讓前端移除 creep 圖
        let _ = mqtx.try_send(OutboundMsg::new_s(
            "td/all/res",
            "creep",
            "D",
            json!({ "id": entity.id() }),
        ));

        // 廣播專用的 game/lives 事件（前端 HUD 立即更新），不需要 hero.stats
        let _ = mqtx.try_send(OutboundMsg::new_s(
            "td/all/res",
            "game",
            "lives",
            json!({ "lives": remaining }),
        ));

        if remaining <= 0 {
            let _ = mqtx.try_send(OutboundMsg::new_s(
                "td/all/res",
                "game",
                "end",
                json!({ "result": "defeat", "reason": "lives_depleted" }),
            ));
            log::warn!("☠️ TD 模式：玩家生命歸零，遊戲結束");
        }
        Ok(())
    }

    fn handle_tower_spawn(ecs: &mut World, mqtx: &crossbeam_channel::Sender<OutboundMsg>, pos: vek::Vec2<f32>, td: TowerData) -> Result<(), Error> {
        let mut cjs = json!(td);
        let e = ecs.create_entity().with(Pos(pos)).with(Tower::new()).with(td.tpty).with(td.tatk).build();
        cjs.as_object_mut().unwrap().insert("id".to_owned(), json!(e.id()));
        cjs.as_object_mut().unwrap().insert("pos".to_owned(), json!(pos));
        mqtx.try_send(OutboundMsg::new_s_at("td/all/res", "tower", "C", cjs, pos.x, pos.y));
        ecs.get_mut::<Searcher>().unwrap().tower.needsort = true;
        Ok(())
    }
    
    fn handle_creep_stop(ecs: &mut World, mqtx: &crossbeam_channel::Sender<OutboundMsg>, source: Entity, target: Entity) -> Result<(), Error> {
        let mut creeps = ecs.write_storage::<Creep>();
        let c = creeps.get_mut(target).ok_or_else(|| failure::err_msg("Creep not found"))?;
        c.block_tower = Some(source);
        c.status = CreepStatus::Stop;
        
        let positions = ecs.read_storage::<Pos>();
        let pos = positions.get(target).ok_or_else(|| failure::err_msg("Creep position not found"))?;
        
        mqtx.try_send(OutboundMsg::new_s_at("td/all/res", "creep", "M", json!({
            "id": target.id(),
            "x": pos.0.x,
            "y": pos.0.y,
        }), pos.0.x, pos.0.y));
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
        let mut hp_after = 0.0f32;
        let mut max_hp = 0.0f32;
        let mut died = false;

        {
            let mut properties = ecs.write_storage::<CProperty>();
            if let Some(target_props) = properties.get_mut(target) {
                let hp_before = target_props.hp;
                let total_damage = phys + magi + real;
                target_props.hp -= total_damage;
                hp_after = target_props.hp;
                max_hp = target_props.mhp;

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
                    hp_after = 0.0;
                    died = true;
                    log::info!("💀 {} died from damage!", target_name);
                }
            }
        }

        // Broadcast HP update to frontend
        let target_pos = ecs.read_storage::<Pos>().get(target).map(|p| p.0);
        if let Some(tp) = target_pos {
            // Determine entity type for the broadcast
            let entity_type = {
                let heroes = ecs.read_storage::<Hero>();
                let creeps = ecs.read_storage::<Creep>();
                let units = ecs.read_storage::<Unit>();
                if heroes.get(target).is_some() { "hero" }
                else if creeps.get(target).is_some() { "creep" }
                else if units.get(target).is_some() { "unit" }
                else { "entity" }
            };
            let mqtx_list = ecs.read_resource::<Vec<crossbeam_channel::Sender<OutboundMsg>>>();
            if let Some(tx) = mqtx_list.get(0) {
                // HP-only update. Action "H" keeps this separate from real move events;
                // the position in `new_s_at` is only used for viewport filtering (not the payload).
                let _ = tx.send(OutboundMsg::new_s_at("td/all/res", entity_type, "H", json!({
                    "id": target.id(),
                    "hp": hp_after,
                    "max_hp": max_hp,
                }), tp.x, tp.y));
            }
        }

        if died {
            next_outcomes.push(Outcome::Death {
                pos: pos,
                ent: target
            });
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