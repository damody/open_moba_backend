/// 狀態初始化器 - 負責設置 ECS 世界和遊戲場景

use std::sync::Arc;
use rayon::{ThreadPool, ThreadPoolBuilder};
use specs::{World, WorldExt, Builder};
use vek::Vec2;

use crate::comp::*;
use crate::ue4::import_map::CreepWaveData;
use crate::ue4::import_campaign::CampaignData;

/// 狀態初始化器
pub struct StateInitializer;

impl StateInitializer {
    /// 創建執行緒池
    pub fn create_thread_pool() -> Arc<ThreadPool> {
        Arc::new(
            ThreadPoolBuilder::new()
                .num_threads(num_cpus::get())
                .thread_name(move |i| format!("rayon-{}", i))
                .build()
                .expect("Failed to create thread pool")
        )
    }

    /// 設置標準 ECS 世界
    pub fn setup_standard_ecs_world(thread_pool: &Arc<ThreadPool>) -> World {
        let mut ecs = World::new();
        Self::register_components(&mut ecs);
        Self::initialize_resources(&mut ecs, thread_pool);
        Self::load_terrain_heightmaps(&mut ecs);
        ecs
    }

    /// 設置戰役 ECS 世界
    pub fn setup_campaign_ecs_world(thread_pool: &Arc<ThreadPool>) -> World {
        let mut ecs = World::new();
        Self::register_components(&mut ecs);
        Self::initialize_resources(&mut ecs, thread_pool);
        Self::load_terrain_heightmaps(&mut ecs);
        Self::setup_campaign_specific_resources(&mut ecs);
        ecs
    }

    /// 初始化小兵波資料
    pub fn init_creep_wave(ecs: &mut World, cw: &CreepWaveData) {
        use std::collections::BTreeMap;

        // 根據 map.json 的 GameMode 欄位設置遊戲模式 resource
        let mode = GameMode::from_opt_str(cw.GameMode.as_deref());
        log::info!("遊戲模式: {:?}", mode);
        *ecs.write_resource::<GameMode>() = mode;
        if mode.is_td() {
            *ecs.write_resource::<PlayerLives>() = PlayerLives::td_default();
            log::info!("TD 模式啟用，玩家生命初始 {}", PlayerLives::TD_INITIAL);
            // TD 模式：等待玩家按 StartRound 才出怪
            let mut ccw = ecs.write_resource::<CurrentCreepWave>();
            ccw.is_running = false;
        }

        // 設置檢查點
        {
            let mut cps = ecs.get_mut::<BTreeMap<String, CheckPoint>>().unwrap();
            for p in cw.CheckPoint.iter() {
                cps.insert(p.Name.clone(), 
                    CheckPoint {
                        name: p.Name.clone(), 
                        class: p.Class.clone(), 
                        pos: Vec2::new(p.X, p.Y)
                    });
            }
        }

        // 設置路徑 - 完全分離的作用域
        Self::setup_paths(ecs, cw);
        
        // 設置小兵發射器
        Self::setup_creep_emiters(ecs, cw);

        // 設置小兵波
        Self::setup_creep_waves(ecs, cw);

        // 設置不可通行多邊形
        Self::setup_blocked_regions(ecs, cw);
    }

    /// 把 map.json 的 BlockedRegions 載入成 ECS resource 供移動 tick 查詢。
    fn setup_blocked_regions(ecs: &mut World, cw: &CreepWaveData) {
        let regions: Vec<BlockedRegion> = cw.BlockedRegions.iter()
            .filter(|r| r.Points.len() >= 3)
            .map(|r| BlockedRegion {
                name: r.Name.clone(),
                points: r.Points.iter().map(|p| Vec2::new(p.X, p.Y)).collect(),
            })
            .collect();
        let n = regions.len();
        *ecs.write_resource::<BlockedRegions>() = BlockedRegions(regions);
        if n > 0 {
            log::info!("載入 {} 個不可通行多邊形區域", n);
        }
    }

    /// 把每個 BlockedRegion polygon 填成一堆靜態 blocker ECS entities
    /// (Pos + CollisionRadius + RegionBlocker)，並推進 Searcher 的 `region` 索引。
    /// 之後碰撞查詢完全走 `Searcher::search_collidable`，不再迭代 polygon。
    /// 呼叫時機：在 BlockedRegions resource 載入 + 所有動態實體（hero/unit/tower/creep）
    /// 建完之後；Searcher region 索引是一次性靜態資料，之後不再重建。
    pub fn populate_region_blockers(ecs: &mut World) {
        log::warn!("▶▶ populate_region_blockers START");
        let polys: Vec<Vec<Vec2<f32>>> = {
            let regions = ecs.read_resource::<BlockedRegions>();
            log::warn!("▶▶ BlockedRegions resource 有 {} 個 polygons", regions.0.len());
            for (i, r) in regions.0.iter().enumerate() {
                log::warn!("▶▶   poly[{}] '{}' 頂點數={}", i, r.name, r.points.len());
            }
            regions.0.iter().map(|r| r.points.clone()).collect()
        };
        let mut created: Vec<(specs::Entity, Vec2<f32>)> = Vec::new();
        for poly in &polys {
            let circles = blocker_circles_for_polygon(poly);
            log::warn!("▶▶ poly 產生 {} 個 blocker circles", circles.len());
            for (p, r) in circles {
                let e = ecs.create_entity()
                    .with(Pos::from_xy_f32(p.x, p.y))
                    .with(CollisionRadius(omoba_sim::Fixed64::from_raw((r * 1024.0) as i64)))
                    .with(RegionBlocker)
                    .build();
                created.push((e, p));
            }
        }
        let n = created.len();
        {
            let mut searcher = ecs.write_resource::<Searcher>();
            searcher.region.rebuild_from(created.iter().map(|(e, p)| (*e, *p)));
            log::warn!("▶▶ searcher.region 寫入 count={} (kind={})",
                searcher.region.count(), searcher.region.kind());
        }
        log::warn!("▶▶ populate_region_blockers DONE: {} blockers created (polygons={})", n, polys.len());
        for (idx, (e, p)) in created.iter().take(3).enumerate() {
            // NOTE: log uses f32 boundary — Fixed64 has no Display.
            let r = ecs.read_storage::<CollisionRadius>().get(*e).map(|c| c.0.to_f32_for_render()).unwrap_or(0.0);
            log::warn!("▶▶   blocker[{}] entity={:?} pos=({:.1},{:.1}) r={:.1}",
                idx, e, p.x, p.y, r);
        }
    }

    /// 設置路徑資料
    fn setup_paths(ecs: &mut World, cw: &CreepWaveData) {
        use std::collections::BTreeMap;
        
        // 讀取檢查點資料並立即釋放
        let cps_clone = {
            let resource = ecs.read_resource::<BTreeMap<String, CheckPoint>>();
            resource.clone()
        };
        
        // 現在可以安全地獲取可變引用
        let mut paths = ecs.write_resource::<BTreeMap<String, Path>>();
        for p in cw.Path.iter() {
            let mut cp_in_path = vec![];
            for ps in p.Points.iter() {
                if let Some(v) = cps_clone.get(ps) {
                    cp_in_path.push(v.clone());
                }
            }
            paths.insert(p.Name.clone(), 
                Path { check_points: cp_in_path });
        }
    }

    /// 設置小兵發射器
    fn setup_creep_emiters(ecs: &mut World, cw: &CreepWaveData) {
        use std::collections::BTreeMap;
        
        let mut ces = ecs.get_mut::<BTreeMap<String, CreepEmiter>>().unwrap();
        log::info!("載入 {} 個小兵類型", cw.Creep.len());
        for cp in cw.Creep.iter() {
            log::info!("小兵類型 '{}' - HP: {}, 移動速度: {}", cp.Name, cp.HP, cp.MoveSpeed);
            ces.insert(cp.Name.clone(), CreepEmiter {
                root: Creep {
                    name: cp.Name.clone(),
                    label: cp.Label.clone(),
                    path: "".to_owned(),
                    pidx: 0,
                    block_tower: None,
                    status: CreepStatus::Walk
                },
                property: CProperty {
                    hp: omoba_sim::Fixed64::from_raw((cp.HP * 1024.0) as i64),
                    mhp: omoba_sim::Fixed64::from_raw((cp.HP * 1024.0) as i64),
                    msd: omoba_sim::Fixed64::from_raw((cp.MoveSpeed * 1024.0) as i64),
                    def_physic: omoba_sim::Fixed64::from_raw((cp.DefendPhysic * 1024.0) as i64),
                    def_magic: omoba_sim::Fixed64::from_raw((cp.DefendMagic * 1024.0) as i64),
                },
                faction_name: cp.Faction.clone().unwrap_or_default(),
                turn_speed_deg: cp.TurnSpeed.unwrap_or(90.0),
                collision_radius: cp.CollisionRadius.unwrap_or(20.0),
            });
        }
    }

    /// 設置小兵波
    fn setup_creep_waves(ecs: &mut World, cw: &CreepWaveData) {
        // Debug 開關：設 OMB_NO_CREEPS=1 完全跳過小兵波載入（碰撞除錯用）
        if std::env::var("OMB_NO_CREEPS").ok().as_deref() == Some("1") {
            log::warn!("⚠ OMB_NO_CREEPS=1：跳過 {} 個小兵波載入", cw.CreepWave.len());
            return;
        }
        let mut cws = ecs.get_mut::<Vec<CreepWave>>().unwrap();
        log::info!("載入 {} 個小兵波", cw.CreepWave.len());
        for cw_data in cw.CreepWave.iter() {
            let mut tcw = CreepWave { time: cw_data.StartTime, path_creeps: vec![] };
            let mut total_creeps = 0;
            for d in cw_data.Detail.iter() {
                let mut es = vec![];
                for cjd in d.Creeps.iter() {
                    es.push(CreepEmit { time: cjd.Time, name: cjd.Creep.clone() });
                    total_creeps += 1;
                }
                tcw.path_creeps.push(PathCreeps { creeps: es, path_name: d.Path.clone() });
            }
            log::info!("小兵波 '{}' 已載入，開始時間: {}秒，共 {} 個小兵", 
                cw_data.Name, cw_data.StartTime, total_creeps);
            cws.push(tcw);
        }
    }

    /// 初始化戰役資料
    pub fn init_campaign_data(ecs: &mut World, campaign_data: &CampaignData) {
        // 插入戰役相關資源
        ecs.insert(campaign_data.clone());
        log::info!("初始化戰役資料: {}", campaign_data.mission.campaign.name);
    }

    /// 創建測試場景
    pub fn create_test_scene(ecs: &mut World) {
        let count = 0;
        // 暫時不創建測試塔，避免與其他系統衝突
        log::info!("創建測試場景完成，實體數量: {}", count);
    }

    /// 創建戰役場景
    pub fn create_campaign_scene(ecs: &mut World, campaign_data: &CampaignData) {
        Self::create_campaign_heroes(ecs, campaign_data);
        // 優先：map.json 的 Structures（script 驅動塔/基地放置）
        let is_td = ecs.read_resource::<GameMode>().is_td();
        if !campaign_data.map.Structures.is_empty() {
            Self::spawn_structures_from_map(ecs, &campaign_data.map);
        } else if !is_td {
            // fallback：舊訓練場用的 training_enemies（B01_1 類 / DEBUG 類）
            // TD 模式下塔由玩家運行時建造，不在場景初始化時生訓練敵人
            Self::create_training_enemies(ecs, campaign_data);
        }
        Self::create_terrain_blockers(ecs);
        log::info!("創建戰役場景完成: {}", campaign_data.mission.campaign.name);
    }

    // 私有輔助方法
    fn register_components(ecs: &mut World) {
        // 註冊所有遊戲組件
        ecs.register::<Pos>();
        ecs.register::<Vel>();
        ecs.register::<TProperty>();
        ecs.register::<CProperty>();
        ecs.register::<TAttack>();
        ecs.register::<Tower>();
        ecs.register::<Creep>();
        ecs.register::<Projectile>();
        ecs.register::<Hero>();
        ecs.register::<Unit>();
        ecs.register::<Faction>();
        ecs.register::<SummonedUnit>();
        ecs.register::<CircularVision>();
        // 舊 Ability/AbilityEffect/Skill/SkillEffect 已隨 skill_system 移除。
        ecs.register::<Enemy>();
        ecs.register::<Campaign>();
        ecs.register::<Stage>();
        ecs.register::<DamageInstance>();
        ecs.register::<DamageResult>();
        ecs.register::<MoveTarget>();
        ecs.register::<Player>();
        ecs.register::<Last<Pos>>();
        ecs.register::<Last<Vel>>();
        ecs.register::<Gold>();
        ecs.register::<Inventory>();
        ecs.register::<ItemEffects>();
        ecs.register::<IsBase>();
        ecs.register::<Bounty>();
        ecs.register::<Facing>();
        ecs.register::<FacingBroadcast>();
        ecs.register::<TurnSpeed>();
        ecs.register::<CollisionRadius>();
        ecs.register::<RegionBlocker>();
        // SlowBuff component 已移除，slow 走 ability_runtime::BuffStore resource
        ecs.register::<crate::scripting::ScriptUnitTag>();
        ecs.register::<IsBuilding>();
        ecs.register::<CreepMoveBroadcast>();
    }

    fn initialize_resources(ecs: &mut World, _thread_pool: &Arc<ThreadPool>) {
        use std::collections::BTreeMap;
        use std::time::Instant;

        // 初始化基本資源
        ecs.insert(Tick(0));
        ecs.insert(TickStart(Instant::now()));
        ecs.insert(TimeOfDay(0.0));
        ecs.insert(Time(0.0));
        ecs.insert(DeltaTime(omoba_sim::Fixed64::ZERO));
        // Phase 1c.3: master seed for deterministic SimRng streams. Phase 2 will
        // overwrite this from the GameStart message; for now use a fixed default.
        ecs.insert(crate::comp::MasterSeed::default());

        // Phase 3.4: pending lockstep player inputs. Filled by the omfx
        // sim_runner from each TickBatch (or by host-side test code) and
        // drained by `tick::player_input_tick::Sys` every dispatcher tick.
        // Inserted unconditionally so the consumer system's `Write<>` always
        // resolves; non-kcp builds use the empty unit-struct variant.
        ecs.insert(crate::comp::PendingPlayerInputs::default());

        // Phase 5.3: latest serialized world snapshot for observer rejoin.
        // Refreshed every SNAPSHOT_INTERVAL_TICKS (= 30 s @ 30 Hz) by the
        // dispatcher tick loop; consumed by the KCP transport's 0x16
        // SnapshotResp handler via a shared `Arc<Mutex<SnapshotStore>>`.
        // Empty (`tick=0`, `bytes=[]`) until the first save fires.
        ecs.insert(crate::comp::SnapshotStore::default());

        // 初始化集合資源
        ecs.insert(BTreeMap::<String, CheckPoint>::new());
        ecs.insert(BTreeMap::<String, Path>::new());
        ecs.insert(BTreeMap::<String, CreepEmiter>::new());
        let mut player_map = BTreeMap::<String, Player>::new();
        let player_name = crate::config::server_config::CONFIG.PLAYER_NAME.clone();
        let mut p = Player { name: player_name.clone(), cost: 100., towers: vec![] };
        p.towers.push(TowerData {
            tpty: TProperty::new(omoba_sim::Fixed64::from_i32(10), 1, omoba_sim::Fixed64::from_i32(100)),
            tatk: TAttack::new(
                omoba_sim::Fixed64::from_i32(3),
                omoba_sim::Fixed64::from_raw(307), // ≈ 0.3
                omoba_sim::Fixed64::from_i32(300),
                omoba_sim::Fixed64::from_i32(100),
            ),
        });
        player_map.insert(player_name.clone(), p);
        log::info!("自動建立預設玩家: {}", player_name);
        ecs.insert(player_map);
        ecs.insert(Vec::<CreepWave>::new());
        // 非 TD 模式預設 is_running=true，沿用時間觸發；TD 模式在 init_creep_wave
        // 讀到 GameMode::TowerDefense 時改為 false，等待 StartRound 指令。
        ecs.insert(CurrentCreepWave { wave: 0, path: vec![], is_running: true, wave_start_time: 0.0 });
        ecs.insert(Vec::<crate::Outcome>::new());
        ecs.insert(Vec::<TakenDamage>::new());
        ecs.insert(SysMetrics::default());
        ecs.insert(crate::comp::TickProfile::default());
        
        // 初始化 MQTT 通道資源
        ecs.insert(Vec::<crossbeam_channel::Sender<crate::transport::OutboundMsg>>::new());
        
        // 初始化 Searcher 資源
        ecs.insert(crate::comp::outcome::Searcher::default());

        // 初始化不可通行多邊形區域（由 init_creep_wave 載入 map.json 時填入）
        ecs.insert(BlockedRegions::default());

        // 遊戲模式 / 玩家生命（由 init_creep_wave 依 map.json 覆寫）
        ecs.insert(GameMode::default());
        ecs.insert(PlayerLives::default());

        // 載入裝備 Registry (MVP LoL item system)
        let item_reg = crate::item::ItemRegistry::load_from_path("item-configs/items.json")
            .unwrap_or_else(|e| {
                log::warn!("裝備 Registry 載入失敗（{}），使用空 registry", e);
                crate::item::ItemRegistry::default()
            });
        ecs.insert(item_reg);

        // 腳本事件佇列（由 tick 系統推入、ScriptDispatchSystem 於本 tick 尾端抽乾）
        ecs.insert(crate::scripting::ScriptEventQueue::default());

        // Buff 系統資源（取代舊的 SlowBuff component）— creep_tick / buff_tick 都會讀
        ecs.insert(crate::ability_runtime::BuffStore::new());

        log::info!("ECS 基本資源初始化完成");
    }

    fn load_terrain_heightmaps(ecs: &mut World) {
        // 載入地形高度圖
        log::info!("載入地形高度圖...");
        
        // 暫時使用預設地形設置
        // 實際實現時應從檔案載入高度圖資料
        
        log::info!("地形高度圖載入完成");
    }

    fn setup_campaign_specific_resources(ecs: &mut World) {
        use std::collections::BTreeMap;
        
        // 設置戰役特有的資源（舊 Ability BTreeMap / AbilityEffect / SkillInput
        // 已隨 skill_system 移除；技能 metadata 由 AbilityRegistry resource 承載）
        ecs.insert(BTreeMap::<String, Hero>::new());
        ecs.insert(BTreeMap::<String, Enemy>::new());
        ecs.insert(Vec::<DamageInstance>::new());
        
        log::info!("設置戰役特有資源");
    }

    fn create_campaign_heroes(ecs: &mut World, campaign_data: &CampaignData) {
        // 從戰役資料創建英雄
        if let Some(hero_data) = campaign_data.entity.heroes.first() {
            let hero = Hero::from_campaign_data(hero_data);
            let hero_faction = Faction::new(FactionType::Player, 0);
            let hero_pos = Pos::from_xy_f32(0.0, 0.0);
            let hero_vel = Vel::zero();

            // 創建英雄的戰鬥屬性 (基於英雄等級和屬性計算)
            use omoba_sim::Fixed64;
            let base_hp = Fixed64::from_i32(500) + Fixed64::from_i32(hero.level) * hero.level_growth.hp_per_level;
            let base_damage = Fixed64::from_i32(50) + Fixed64::from_i32(hero.level) * hero.level_growth.damage_per_level;

            let hero_properties = CProperty {
                hp: base_hp,
                mhp: base_hp,
                msd: Fixed64::from_i32(350), // 基礎移動速度
                def_physic: Fixed64::from_i32(hero.strength) * Fixed64::from_raw(205), // ≈ 0.2 = 205/1024
                def_magic: Fixed64::from_i32(hero.intelligence) * Fixed64::from_raw(154), // ≈ 0.15 = 154/1024
            };

            // 從 templates.json 取 hero stats（attack_range / turn_speed / 等）。
            // entity.json hero 條目已 slim 成只剩 id，無 attack_range / turn_speed / collision_radius。
            let hero_template_stats = omoba_template_ids::hero_by_name(&hero_data.id)
                .and_then(|hid| omoba_template_ids::hero_stats(hid))
                .unwrap_or_else(|| panic!("hero '{}' not in templates.json", hero_data.id));

            let hero_attack = TAttack {
                atk_physic: Vf32::new(base_damage),
                asd: Vf32::new(Fixed64::from_raw(602)), // 1/1.7 ≈ 0.588 (= 602/1024)
                range: Vf32::new(hero_template_stats.attack_range),
                asd_count: Fixed64::ZERO,
                bullet_speed: Fixed64::from_i32(1000),
            };

            // 創建英雄圓形視野組件
            let hero_vision = CircularVision::new(
                1200.0, // 英雄視野範圍
                180.0   // 英雄高度
            ).with_precision(720); // 高精度視野

            // hero_template_stats.turn_speed is Fixed64 in degrees; convert to radians (f32) for omb internal.
            let hero_turn_rad = hero_template_stats.turn_speed.to_f32_for_render() * std::f32::consts::PI / 180.0;
            // Hero collision_radius 暫定 30（之前由 entity.json optional override，
            // 簡化後固定）。
            let hero_radius = 30.0_f32;
            // Hero 統一掛 ScriptUnitTag（預設全單位腳本化）；unit_id = "hero_{HeroJD.id}"
            // 若 registry 無對應腳本，dispatch 會 silent skip，host hero_tick 仍跑預設 auto-attack
            let unit_id = format!("hero_{}", hero_data.id);
            let hero_entity = ecs.create_entity()
                .with(hero_pos)
                .with(hero_vel)
                .with(hero)
                .with(hero_faction)
                .with(hero_properties)
                .with(hero_attack)
                .with(hero_vision)
                .with(Gold(10000))
                .with(Inventory::new())
                .with(ItemEffects::default())
                .with(Facing(omoba_sim::Angle::ZERO))
                .with(FacingBroadcast(None))
                .with(TurnSpeed(omoba_sim::Fixed64::from_raw((hero_turn_rad * 1024.0) as i64)))
                .with(CollisionRadius(omoba_sim::Fixed64::from_raw((hero_radius * 1024.0) as i64)))
                .with(crate::scripting::ScriptUnitTag { unit_id: unit_id.clone() })
                .build();

            // 排 on_spawn 事件，讓可能存在的 hero unit script 初始化
            ecs.write_resource::<crate::scripting::ScriptEventQueue>()
                .push(crate::scripting::ScriptEvent::Spawn { e: hero_entity });

            log::info!(
                "創建戰役英雄實體: {:?} unit_id={}（含 Gold/Inventory/ItemEffects + ScriptUnitTag）",
                hero_entity, unit_id
            );
        }
    }

    /// MVP_1 場景（LoL 風格單線）
    ///
    /// 依 map.json 的 `Structures` 清單放置塔/基地。
    /// 每筆 Structure 指定 Tower 模板名稱 + 陣營 + 位置 + 是否為基地，
    /// 模板屬性（Hp/Range/AttackSpeed/Physic）從 `Tower` 清單查。
    pub fn spawn_structures_from_map(ecs: &mut World, cw: &CreepWaveData) {
        use std::collections::HashMap;
        if cw.Structures.is_empty() {
            return;
        }
        // 建立 Tower 模板查表
        let tower_templates: HashMap<&str, &crate::ue4::import_map::TowerJD> =
            cw.Tower.iter().map(|t| (t.Name.as_str(), t)).collect();

        let mut script_count = 0usize;
        let mut dumb_count = 0usize;
        let total = cw.Structures.len();

        for s in cw.Structures.iter() {
            let pos = Vec2::new(s.X, s.Y);
            let faction_type = match s.Faction.as_str() {
                "Player" | "player" => FactionType::Player,
                _ => FactionType::Enemy,
            };

            // 優先嘗試 script-driven 塔：如果 template name 對得上 TowerTemplateRegistry
            // 註冊過的 unit_id（"tower_dart" / "tower_ice" / "tower_bomb" / "tower_tack"），
            // 走 spawn_td_tower 路徑 — 自動掛 ScriptUnitTag、push Spawn event、由腳本 on_tick 驅動。
            // 只對玩家方非基地實體做（敵塔目前沒有對應腳本）。
            if faction_type == FactionType::Player && !s.IsBase {
                let has_script = ecs
                    .read_resource::<crate::comp::tower_registry::TowerTemplateRegistry>()
                    .get(s.Tower.as_str())
                    .is_some();
                if has_script {
                    if crate::comp::tower_template::spawn_td_tower(ecs, pos, &s.Tower).is_some() {
                        script_count += 1;
                        continue;
                    }
                }
            }

            // Fallback：走 map.json Tower 模板的 dumb tower 路徑（無腳本）
            let Some(tpl) = tower_templates.get(s.Tower.as_str()) else {
                log::warn!("Structure 未知 Tower 模板 '{}'，跳過", s.Tower);
                continue;
            };
            let hp = tpl.Property.Hp as f32;
            let range = tpl.Attack.Range;
            let atk = tpl.Attack.Physic;
            let asd = if tpl.Attack.AttackSpeed > 0.0 {
                tpl.Attack.AttackSpeed
            } else {
                1.0
            };
            let turn_deg = tpl.TurnSpeed.unwrap_or(45.0);
            let radius = s.CollisionRadius
                .or(tpl.CollisionRadius)
                .unwrap_or(50.0);
            Self::spawn_tower(
                ecs,
                pos,
                faction_type,
                hp,
                range,
                atk,
                asd,
                s.IsBase,
                turn_deg,
                radius,
            );
            dumb_count += 1;
        }
        log::info!("已依 map.json 放置 {} 個 Structure (script-driven={}, dumb={})",
            total, script_count, dumb_count);
    }

    fn spawn_tower(
        ecs: &mut World,
        pos: Vec2<f32>,
        faction_type: FactionType,
        hp: f32,
        range: f32,
        atk: f32,
        asd: f32,
        is_base: bool,
        turn_speed_deg: f32,
        collision_radius: f32,
    ) {
        use omoba_sim::Fixed64;
        let hp_fx = Fixed64::from_raw((hp * 1024.0) as i64);
        let range_fx = Fixed64::from_raw((range * 1024.0) as i64);
        let atk_fx = Fixed64::from_raw((atk * 1024.0) as i64);
        let asd_fx = Fixed64::from_raw((asd * 1024.0) as i64);
        let prop = TProperty::new(hp_fx, 0, Fixed64::from_i32(120));
        let atk_c = TAttack::new(atk_fx, asd_fx, range_fx, Fixed64::from_i32(1200));
        // Team id 0 for Player, 1 for Enemy (matches create_campaign_heroes convention)
        let team_id = if faction_type == FactionType::Player { 0 } else { 1 };
        let faction = Faction::new(faction_type.clone(), team_id);
        let vision = CircularVision::new(range + 200.0, 40.0).with_precision(180);
        // 傷害處理讀 CProperty.hp，所以塔也要有 CProperty
        let cprop = CProperty {
            hp: hp_fx,
            mhp: hp_fx,
            msd: Fixed64::ZERO,
            def_physic: Fixed64::ZERO,
            def_magic: Fixed64::ZERO,
        };

        // 擊毀獎勵：一般塔 150g / 200xp；基地 300g / 500xp；我方被擊毀不給獎勵
        let bounty = if faction_type == FactionType::Player {
            Bounty { gold: 0, exp: 0 }
        } else if is_base {
            Bounty { gold: 300, exp: 500 }
        } else {
            Bounty { gold: 150, exp: 200 }
        };

        let mut builder = ecs
            .create_entity()
            .with(Pos::from_xy_f32(pos.x, pos.y))
            .with(Tower::new())
            .with(prop)
            .with(cprop)
            .with(atk_c)
            .with(faction)
            .with(vision)
            .with(bounty)
            .with(Facing(omoba_sim::Angle::ZERO))
            .with(FacingBroadcast(None))
            .with(TurnSpeed(omoba_sim::Fixed64::from_raw((turn_speed_deg.to_radians() * 1024.0) as i64)))
            .with(CollisionRadius(omoba_sim::Fixed64::from_raw((collision_radius * 1024.0) as i64)));

        // 雙方基地都標記 IsBase（前端依此顯示「基地」名稱）；
        // 勝負判定在 handle_death 裡還要檢查 faction，只有敵方基地死亡才觸發玩家勝
        if is_base {
            builder = builder.with(IsBase);
        }
        let e = builder.build();
        let side = if faction_type == FactionType::Player { "我方" } else { "敵方" };
        log::info!("{}{}已生成於 ({:.0}, {:.0}) entity={:?}",
            side, if is_base { "基地" } else { "塔" }, pos.x, pos.y, e);
    }

    fn create_training_enemies(ecs: &mut World, campaign_data: &CampaignData) {
        // 創建訓練用敵人單位
        let enemy_positions = [
            (800.0, 0.0),
            (1000.0, 100.0),
            (1200.0, -50.0),
        ];

        for (i, (x, y)) in enemy_positions.iter().enumerate() {
            if let Some(enemy_data) = campaign_data.entity.enemies.get(i % campaign_data.entity.enemies.len()) {
                let unit = Unit::from_enemy_data(enemy_data);
                let enemy_faction = Faction::new(FactionType::Enemy, 1);
                let unit_pos = Pos::from_xy_f32(*x, *y);
                let unit_vel = Vel::zero();

                let unit_properties = CProperty {
                    // NOTE: Unit.{current_hp, max_hp, base_damage} are i32 by design (integer game values).
                    hp: omoba_sim::Fixed64::from_i32(unit.current_hp),
                    mhp: omoba_sim::Fixed64::from_i32(unit.max_hp),
                    msd: unit.move_speed,
                    def_physic: unit.base_armor,
                    def_magic: unit.magic_resistance,
                };

                let unit_attack = TAttack {
                    atk_physic: Vf32::new(omoba_sim::Fixed64::from_i32(unit.base_damage)),
                    // NOTE: Fixed64::ONE / attack_speed exercises Fixed64 division at spawn boundary; sim-side reads asd.v directly.
                    asd: Vf32::new(omoba_sim::Fixed64::ONE / unit.attack_speed),
                    range: Vf32::new(unit.attack_range),
                    asd_count: omoba_sim::Fixed64::ZERO,
                    bullet_speed: omoba_sim::Fixed64::from_i32(800),
                };

                let enemy_vision = CircularVision::new(
                    // NOTE: CircularVision is client-side render hint (fog of war); per-tick rebuild from authoritative Pos keeps it cross-client consistent.
                    unit.attack_range.to_f32_for_render() + 150.0,
                    20.0
                ).with_precision(360);

                // MOBA 訓練敵人也一併掛 ScriptUnitTag（統一規則）
                let unit_uid = format!("unit_{}", enemy_data.id);
                let _unit_entity = ecs.create_entity()
                    .with(unit_pos)
                    .with(unit_vel)
                    .with(unit)
                    .with(enemy_faction)
                    .with(unit_properties)
                    .with(unit_attack)
                    .with(enemy_vision)
                    .with(CollisionRadius(omoba_sim::Fixed64::from_i32(20)))
                    .with(crate::scripting::ScriptUnitTag { unit_id: unit_uid.clone() })
                    .build();
                ecs.write_resource::<crate::scripting::ScriptEventQueue>()
                    .push(crate::scripting::ScriptEvent::Spawn { e: _unit_entity });

                log::info!("創建訓練敵人單位 '{}' 於位置 ({}, {})", enemy_data.id, x, y);
            }
        }
    }

    fn create_terrain_blockers(ecs: &mut World) {
        // 創建地形遮擋物
        log::info!("地形遮擋物創建（新視野系統待實現）");
    }
}

// =====================================================================
// Phase 3 omfx-side helpers
//
// Expose a slim, transport-free bootstrap path that produces a fully
// initialized ECS World for the omfx sim_runner worker. The legacy
// `State::new_with_campaign` path also uses these same building blocks,
// so the omfx-side simulator stays in sync with omobab.exe.
//
// Notes:
//   * The world has an empty `Vec<Sender<OutboundMsg>>` inserted (by
//     `setup_campaign_ecs_world`); systems that try to push outbound
//     messages will silently drop them which is exactly what the
//     deterministic sim wants — wire emit is the host's job, not the
//     replica simulator's.
//   * `MasterSeed` is left at the default; the caller (sim_runner)
//     overwrites it once the GameStart message arrives.
//   * Script registries (Tower / Ability / Tower Upgrade) are filled
//     here so unit tick can spawn / dispatch correctly.
// =====================================================================

/// Build a fully-initialized ECS World from a campaign scene path
/// (e.g. "Story/MVP_1"). Inserts campaign + scripts + tower / ability
/// registries. Used by Phase 3 omfx sim_runner; mirrors what
/// `State::new_with_campaign` does minus all the transport / heartbeat
/// plumbing.
pub fn create_world_for_scene(scene_path: &std::path::Path) -> Result<World, failure::Error> {
    use failure::err_msg;
    let scene_str = scene_path
        .to_str()
        .ok_or_else(|| err_msg("scene_path is not valid UTF-8"))?;

    log::info!("[create_world_for_scene] loading campaign from {}", scene_str);
    let campaign_data = CampaignData::load_from_path(scene_str)
        .map_err(|e| err_msg(format!("CampaignData::load_from_path({}) failed: {}", scene_str, e)))?;
    if let Err(err) = campaign_data.validate() {
        return Err(err_msg(format!("Campaign data validation failed: {}", err)));
    }

    let thread_pool = StateInitializer::create_thread_pool();
    let mut ecs = StateInitializer::setup_campaign_ecs_world(&thread_pool);

    // Load scripts BEFORE scene init so spawn_td_tower can resolve
    // unit_id → script template.
    let dir_str = std::env::var("OMB_SCRIPTS_DIR").unwrap_or_else(|_| "./scripts".to_string());
    let dir = std::path::Path::new(&dir_str);
    let registry = crate::scripting::loader::load_scripts_dir(dir);
    populate_tower_template_registry(&mut ecs, &registry);
    populate_tower_upgrade_registry(&mut ecs);
    populate_ability_registry(&mut ecs, &registry);
    ecs.insert(registry);

    // Apply campaign / map data.
    StateInitializer::init_campaign_data(&mut ecs, &campaign_data);
    StateInitializer::init_creep_wave(&mut ecs, &campaign_data.map);
    StateInitializer::create_campaign_scene(&mut ecs, &campaign_data);
    StateInitializer::populate_region_blockers(&mut ecs);

    log::info!("[create_world_for_scene] ECS world ready");
    Ok(ecs)
}

/// Phase 3 omfx-side helper: populate `TowerTemplateRegistry` from a
/// `ScriptRegistry`. Mirror of the private method in `state::core::State`.
pub fn populate_tower_template_registry(
    ecs: &mut World,
    registry: &crate::scripting::ScriptRegistry,
) {
    use abi_stable::std_types::RSome;
    use crate::comp::tower_registry::{TowerTemplate as RuntimeTpl, TowerTemplateRegistry};
    let mut reg = TowerTemplateRegistry::default();
    for (uid, script) in registry.iter_ordered() {
        let meta = match script.tower_metadata() {
            RSome(m) => m,
            _ => continue,
        };
        reg.insert(RuntimeTpl {
            unit_id: uid.to_string(),
            label: meta.label.to_string(),
            atk: meta.atk.to_f32_for_render(),
            asd_interval: meta.asd_interval.to_f32_for_render(),
            range: meta.range.to_f32_for_render(),
            bullet_speed: meta.bullet_speed.to_f32_for_render(),
            splash_radius: meta.splash_radius.to_f32_for_render(),
            hit_radius: meta.hit_radius.to_f32_for_render(),
            slow_factor: meta.slow_factor.to_f32_for_render(),
            slow_duration: meta.slow_duration.to_f32_for_render(),
            cost: meta.cost,
            footprint: meta.footprint.to_f32_for_render(),
            hp: meta.hp.to_f32_for_render(),
            turn_speed_deg: meta.turn_speed_deg.to_f32_for_render(),
        });
    }
    log::info!("[tower_registry] {} templates loaded", reg.templates.len());
    ecs.insert(reg);
}

/// Phase 3 omfx-side helper: build the static 48-tower upgrade table.
pub fn populate_tower_upgrade_registry(ecs: &mut World) {
    let reg = crate::comp::tower_upgrade_registry::TowerUpgradeRegistry::new();
    ecs.insert(reg);
}

/// Phase 3 omfx-side helper: copy ability metadata from script registry
/// into ECS-side `AbilityRegistry` resource.
pub fn populate_ability_registry(
    ecs: &mut World,
    registry: &crate::scripting::ScriptRegistry,
) {
    use crate::ability_runtime::AbilityRegistry;
    let mut reg = AbilityRegistry::new();
    for (_id, def, _script) in registry.iter_abilities() {
        reg.register(def.clone());
    }
    log::info!("[ability_registry] {} abilities loaded", reg.len());
    ecs.insert(reg);
}