/// 創建相關事件處理

use specs::{Entity, World, WorldExt, Builder};
use vek::Vec2;
use crate::comp::*;
use crate::transport::OutboundMsg;
use crossbeam_channel::Sender;
use serde_json::json;
use log::{info, warn, error};

/// 創建事件處理器
pub struct CreationEventHandler;

impl CreationEventHandler {
    /// 處理小兵創建事件
    /// 在遊戲世界中創建小兵實體，並發送 MQTT 消息通知前端
    pub fn handle_creep_creation(
        world: &mut World,
        mqtx: &Sender<OutboundMsg>,
        cd: CreepData,
    ) -> Vec<Outcome> {
        // NOTE: log uses f32 boundary — Fixed64 has no Display.
        let pos_x_f = cd.pos.x.to_f32_for_render();
        let pos_y_f = cd.pos.y.to_f32_for_render();
        info!("創建小兵於位置: ({}, {})", pos_x_f, pos_y_f);

        let display_name = cd.creep.label.clone().unwrap_or_else(|| cd.creep.name.clone());
        let creep_name = cd.creep.name.clone();
        let hp = cd.cdata.hp;
        let mhp = cd.cdata.mhp;
        let msd = cd.cdata.msd;
        let pos = cd.pos;
        let radius = cd.collision_radius;

        // Creep 統一掛 ScriptUnitTag（預設全單位腳本化）
        let unit_id = format!("creep_{}", creep_name);
        // 創建小兵實體
        let entity = world.create_entity()
            .with(Pos(pos)) // SimVec2 直接內嵌
            .with(cd.creep)
            .with(cd.cdata)
            .with(CollisionRadius(radius))
            .with(crate::scripting::ScriptUnitTag { unit_id: unit_id.clone() })
            .build();
        world.write_resource::<crate::scripting::ScriptEventQueue>()
            .push(crate::scripting::ScriptEvent::Spawn { e: entity });
        let hp_f = hp.to_f32_for_render();
        let mhp_f = mhp.to_f32_for_render();
        let msd_f = msd.to_f32_for_render();
        let radius_f = radius.to_f32_for_render();

        // Payload shape matches client expectations (top-level position/hp/max_hp/name)
        let payload = json!({
            "entity_id": entity.id(),
            "id": entity.id(),
            "name": display_name,
            "position": { "x": pos_x_f, "y": pos_y_f },
            "hp": hp_f,
            "max_hp": mhp_f,
            "move_speed": msd_f,
            "collision_radius": radius_f,
        });

        // Phase 5.2: legacy 0x02 GameEvent producer cut. ECS spawn above is
        // authoritative; lockstep clients hydrate via render_bridge.
        let _ = (mqtx, payload, hp_f, mhp_f, msd_f, radius_f, creep_name);

        // 小兵創建成功，無需產生額外事件
        Vec::new()
    }

    /// 處理塔創建事件
    /// 在遊戲世界中創建塔實體，更新搜尋索引，並發送 MQTT 消息
    pub fn handle_tower_creation(
        world: &mut World,
        mqtx: &Sender<OutboundMsg>,
        pos: omoba_sim::Vec2,
        td: TowerData,
    ) -> Vec<Outcome> {
        // NOTE: log uses f32 boundary — Fixed64 has no Display.
        let pos_x_f = pos.x.to_f32_for_render();
        let pos_y_f = pos.y.to_f32_for_render();
        info!("創建塔於位置: ({}, {})", pos_x_f, pos_y_f);

        // 序列化塔資料為 JSON
        let mut cjs = json!(td);

        // 創建塔實體
        let entity = world.create_entity()
            .with(Pos(pos))
            .with(Tower::new())
            .with(td.tpty)
            .with(td.tatk)
            .build();

        // 在 JSON 中添加實體 ID 和位置
        if let Some(obj) = cjs.as_object_mut() {
            obj.insert("id".to_owned(), json!(entity.id()));
            obj.insert("pos".to_owned(), json!({"x": pos_x_f, "y": pos_y_f}));
        }

        // Phase 5.2: legacy 0x02 GameEvent producer cut.
        let _ = (mqtx, cjs);


        // 標記塔搜尋索引需要重新排序
        if let Some(mut searcher) = world.try_fetch_mut::<Searcher>() {
            searcher.tower.mark_dirty();
            info!("標記塔搜尋索引需要重新排序");
        } else {
            warn!("無法獲取 Searcher 資源，跳過索引更新");
        }
        
        // 塔創建成功，無需產生額外事件
        Vec::new()
    }

    /// 處理彈道創建事件
    /// 創建投射物實體，用於視覺效果顯示和傷害處理
    pub fn handle_projectile_creation(
        world: &mut World,
        mqtx: &Sender<OutboundMsg>,
        pos: omoba_sim::Vec2,
        source: Entity,
        target: Entity,
        damage_phys: Option<f32>,
        damage_magi: Option<f32>,
        damage_real: Option<f32>,
    ) -> Vec<Outcome> {
        use omoba_sim::{Fixed64, Vec2 as SimVec2};
        // NOTE: log uses f32 boundary — Fixed64 has no Display.
        let pos_x_f = pos.x.to_f32_for_render();
        let pos_y_f = pos.y.to_f32_for_render();
        info!("創建彈道從實體 {} 到實體 {} 於位置 ({}, {})",
              source.id(), target.id(), pos_x_f, pos_y_f);

        // 獲取來源和目標的位置資訊
        let (source_pos, target_pos): (SimVec2, SimVec2) = {
            let positions = world.read_storage::<Pos>();

            let source_pos: SimVec2 = match positions.get(source) {
                Some(p) => p.0,
                None => {
                    warn!("無法找到來源實體 {} 的位置，使用預設位置", source.id());
                    pos
                }
            };

            let target_pos: SimVec2 = match positions.get(target) {
                Some(p) => p.0,
                None => {
                    warn!("無法找到目標實體 {} 的位置，使用預設位置", target.id());
                    pos
                }
            };

            (source_pos, target_pos)
        }; // positions 在這裡被釋放

        // 從來源實體獲取攻擊屬性來計算傷害值。
        // 入口參數仍是 Option<f32>（callers in legacy paths）；轉成 Fixed64 在邊界。
        let (phys_damage, magi_damage, real_damage): (Fixed64, Fixed64, Fixed64) = {
            let attacks = world.read_storage::<TAttack>();
            let to_fx = |v: f32| Fixed64::from_raw((v * omoba_sim::fixed::SCALE as f32) as i64);
            if let Some(attack) = attacks.get(source) {
                (
                    damage_phys.map(to_fx).unwrap_or(attack.atk_physic.v),
                    damage_magi.map(to_fx).unwrap_or(Fixed64::ZERO),
                    damage_real.map(to_fx).unwrap_or(Fixed64::ZERO),
                )
            } else {
                // 如果沒有攻擊組件，使用傳入的數值或預設值
                (
                    damage_phys.map(to_fx).unwrap_or(Fixed64::from_i32(25)),
                    damage_magi.map(to_fx).unwrap_or(Fixed64::ZERO),
                    damage_real.map(to_fx).unwrap_or(Fixed64::ZERO),
                )
            }
        };

        // 創建投射物實體（用於視覺效果和傷害處理）
        let projectile_entity = world.create_entity()
            .with(Pos(source_pos))
            .with(Projectile {
                time_left: Fixed64::from_i32(2),     // 彈道存活時間
                owner: source,                       // 擁有者
                target: Some(target),                // 目標實體
                tpos: target_pos,                    // 目標位置
                radius: Fixed64::from_i32(5),        // 碰撞半徑
                msd: Fixed64::from_i32(500),         // 移動速度
                damage_phys: phys_damage,
                damage_magi: magi_damage,
                damage_real: real_damage,
                slow_factor: Fixed64::ZERO,
                slow_duration: Fixed64::ZERO,
                hit_radius: Fixed64::ZERO,
                stun_duration: Fixed64::ZERO,
            })
            .build();

        // 前端自管子彈動畫：提供 target_id / move_speed / flight_time_ms，
        // 由前端用 pursuit 公式 lerp 到目標當下位置，保證命中時剛好落到 creep 身上。
        // Wire format f32 (Phase 2 boundary).
        let source_x_f = source_pos.x.to_f32_for_render();
        let source_y_f = source_pos.y.to_f32_for_render();
        let target_x_f = target_pos.x.to_f32_for_render();
        let target_y_f = target_pos.y.to_f32_for_render();
        let move_speed: f32 = 500.0;
        let dx = target_x_f - source_x_f;
        let dy = target_y_f - source_y_f;
        let initial_dist = (dx * dx + dy * dy).sqrt();
        let flight_time_ms: u64 = if move_speed > 0.0 {
            (initial_dist / move_speed * 1000.0).max(1.0) as u64
        } else {
            0
        };

        let projectile_data = json!({
            "id": projectile_entity.id(),
            "target_id": target.id(),
            "start_pos": {
                "x": source_x_f,
                "y": source_y_f
            },
            "end_pos": {
                "x": target_x_f,
                "y": target_y_f
            },
            "move_speed": move_speed,
            "flight_time_ms": flight_time_ms,
        });

        // P7: non-AOE (splash=0) single-target → pre-declared physical damage.
        let predeclared_dmg: f32 = (phys_damage + magi_damage + real_damage).to_f32_for_render();
        // Mirror damage into JSON so non-kcp path also supplies it to omfx shim.
        let mut projectile_data_with_dmg = projectile_data.clone();
        if let Some(obj) = projectile_data_with_dmg.as_object_mut() {
            obj.insert("damage".into(), json!(predeclared_dmg));
            obj.insert("splash_radius".into(), json!(0.0));
            obj.insert("hit_radius".into(), json!(0.0));
            obj.insert("directional".into(), json!(false));
            obj.insert("kind".into(), json!(""));
        }
        // Phase 5.2: legacy 0x02 GameEvent producer cut.
        let _ = (mqtx, projectile_data_with_dmg, source_x_f, source_y_f, target_x_f, target_y_f, flight_time_ms, predeclared_dmg, target);

        // 彈道創建成功，無需產生額外事件
        Vec::new()
    }

    /// 處理單位生成事件
    /// 根據單位類型和陣營創建相應實體
    pub fn handle_unit_spawn(
        world: &mut World,
        mqtx: &Sender<OutboundMsg>,
        pos: omoba_sim::Vec2,
        unit: Unit,
        faction: Faction,
        duration: Option<omoba_sim::Fixed64>,
    ) -> Vec<Outcome> {
        // NOTE: log uses f32 boundary — Fixed64 has no Display.
        let pos_x_f = pos.x.to_f32_for_render();
        let pos_y_f = pos.y.to_f32_for_render();
        info!("生成單位於位置 ({}, {})，陣營: {:?}", pos_x_f, pos_y_f, faction);

        let faction_clone = faction.clone(); // 克隆供後續使用

        let entity_builder = world.create_entity()
            .with(Pos(pos))
            .with(unit)
            .with(faction);

        // 如果有持續時間，添加臨時單位組件
        if let Some(duration) = duration {
            // 這裡需要一個 TemporaryUnit 組件來處理有時間限制的單位
            info!("單位將在 {:.1} 秒後消失", duration.to_f32_for_render());
            // entity_builder = entity_builder.with(TemporaryUnit { remaining_time: duration });
        }

        let entity = entity_builder.build();

        // Phase 5.2: legacy 0x02 GameEvent producer cut.
        let _ = (mqtx, entity, duration, faction_clone, pos_x_f, pos_y_f);

        Vec::new()
    }
}