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
                    .with(Pos(p))
                    .with(CollisionRadius(r))
                    .with(RegionBlocker)
                    .build();
                created.push((e, p));
            }
        }
        let n = created.len();
        {
            use voracious_radix_sort::RadixSort;
            let mut searcher = ecs.write_resource::<Searcher>();
            searcher.region.xpos.clear();
            searcher.region.ypos.clear();
            for (e, p) in &created {
                searcher.region.xpos.push(PosXIndex { e: *e, p: *p });
                searcher.region.ypos.push(PosYIndex { e: *e, p: *p });
            }
            searcher.region.xpos.voracious_mt_sort(4);
            searcher.region.ypos.voracious_mt_sort(4);
            log::warn!("▶▶ searcher.region 寫入 xpos={} ypos={}",
                searcher.region.xpos.len(), searcher.region.ypos.len());
        }
        log::warn!("▶▶ populate_region_blockers DONE: {} blockers created (polygons={})", n, polys.len());
        for (idx, (e, p)) in created.iter().take(3).enumerate() {
            let r = ecs.read_storage::<CollisionRadius>().get(*e).map(|c| c.0).unwrap_or(0.0);
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
                    hp: cp.HP,
                    mhp: cp.HP,
                    msd: cp.MoveSpeed,
                    def_physic: cp.DefendPhysic,
                    def_magic: cp.DefendMagic
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
        ecs.register::<TurnSpeed>();
        ecs.register::<CollisionRadius>();
        ecs.register::<RegionBlocker>();
        // SlowBuff component 已移除，slow 走 ability_runtime::BuffStore resource
        ecs.register::<crate::scripting::ScriptUnitTag>();
    }

    fn initialize_resources(ecs: &mut World, _thread_pool: &Arc<ThreadPool>) {
        use std::collections::BTreeMap;
        use std::time::Instant;

        // 初始化基本資源
        ecs.insert(Tick(0));
        ecs.insert(TickStart(Instant::now()));
        ecs.insert(TimeOfDay(0.0));
        ecs.insert(Time(0.0));
        ecs.insert(DeltaTime(0.0));

        // 初始化集合資源
        ecs.insert(BTreeMap::<String, CheckPoint>::new());
        ecs.insert(BTreeMap::<String, Path>::new());
        ecs.insert(BTreeMap::<String, CreepEmiter>::new());
        let mut player_map = BTreeMap::<String, Player>::new();
        let player_name = crate::config::server_config::CONFIG.PLAYER_NAME.clone();
        let mut p = Player { name: player_name.clone(), cost: 100., towers: vec![] };
        p.towers.push(TowerData { tpty: TProperty::new(10., 1, 100.), tatk: TAttack::new(3., 0.3, 300., 100.) });
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
            let hero_pos = Pos(Vec2::new(0.0, 0.0));
            let hero_vel = Vel(Vec2::new(0.0, 0.0));

            // 創建英雄的戰鬥屬性 (基於英雄等級和屬性計算)
            let base_hp = 500.0 + (hero.level as f32 * hero.level_growth.hp_per_level);
            let base_damage = 50.0 + (hero.level as f32 * hero.level_growth.damage_per_level);
            
            let hero_properties = CProperty {
                hp: base_hp,
                mhp: base_hp,
                msd: 350.0, // 基礎移動速度
                def_physic: hero.strength as f32 * 0.2, // 基於力量的物理防禦
                def_magic: hero.intelligence as f32 * 0.15, // 基於智力的魔法防禦
            };

            let hero_attack = TAttack {
                atk_physic: Vf32::new(base_damage),
                asd: Vf32::new(1.0 / 1.7), // 攻擊間隔（攻擊速度的倒數）
                range: Vf32::new(hero_data.attack_range), // 使用 JSON 中的攻擊範圍
                asd_count: 0.0,
                bullet_speed: 1000.0,
            };

            // 創建英雄圓形視野組件
            let hero_vision = CircularVision::new(
                1200.0, // 英雄視野範圍
                180.0   // 英雄高度
            ).with_precision(720); // 高精度視野

            let hero_turn_rad = hero_data.turn_speed.unwrap_or(180.0).to_radians();
            let hero_radius = hero_data.collision_radius.unwrap_or(30.0);
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
                .with(Facing(0.0))
                .with(TurnSpeed(hero_turn_rad))
                .with(CollisionRadius(hero_radius))
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

        for s in cw.Structures.iter() {
            let Some(tpl) = tower_templates.get(s.Tower.as_str()) else {
                log::warn!("Structure 未知 Tower 模板 '{}'，跳過", s.Tower);
                continue;
            };
            let faction_type = match s.Faction.as_str() {
                "Player" | "player" => FactionType::Player,
                _ => FactionType::Enemy,
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
            // Structure 實例可覆寫碰撞半徑，否則用模板的，再否則用預設 50
            let radius = s.CollisionRadius
                .or(tpl.CollisionRadius)
                .unwrap_or(50.0);
            Self::spawn_tower(
                ecs,
                Vec2::new(s.X, s.Y),
                faction_type,
                hp,
                range,
                atk,
                asd,
                s.IsBase,
                turn_deg,
                radius,
            );
        }
        log::info!("已依 map.json 放置 {} 個 Structure", cw.Structures.len());
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
        let prop = TProperty::new(hp, 0, 120.0);
        let atk_c = TAttack::new(atk, asd, range, 1200.0);
        // Team id 0 for Player, 1 for Enemy (matches create_campaign_heroes convention)
        let team_id = if faction_type == FactionType::Player { 0 } else { 1 };
        let faction = Faction::new(faction_type.clone(), team_id);
        let vision = CircularVision::new(range + 200.0, 40.0).with_precision(180);
        // 傷害處理讀 CProperty.hp，所以塔也要有 CProperty
        let cprop = CProperty {
            hp,
            mhp: hp,
            msd: 0.0,
            def_physic: 0.0,
            def_magic: 0.0,
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
            .with(Pos(pos))
            .with(Tower::new())
            .with(prop)
            .with(cprop)
            .with(atk_c)
            .with(faction)
            .with(vision)
            .with(bounty)
            .with(Facing(0.0))
            .with(TurnSpeed(turn_speed_deg.to_radians()))
            .with(CollisionRadius(collision_radius));

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
                let unit_pos = Pos(Vec2::new(*x, *y));
                let unit_vel = Vel(Vec2::new(0.0, 0.0));

                let unit_properties = CProperty {
                    hp: unit.current_hp as f32,
                    mhp: unit.max_hp as f32,
                    msd: unit.move_speed,
                    def_physic: unit.base_armor,
                    def_magic: unit.magic_resistance,
                };

                let unit_attack = TAttack {
                    atk_physic: Vf32::new(unit.base_damage as f32),
                    asd: Vf32::new(1.0 / unit.attack_speed),
                    range: Vf32::new(unit.attack_range),
                    asd_count: 0.0,
                    bullet_speed: 800.0,
                };

                let enemy_vision = CircularVision::new(
                    unit.attack_range + 150.0,
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
                    .with(CollisionRadius(20.0))
                    .with(crate::scripting::ScriptUnitTag { unit_id: unit_uid.clone() })
                    .build();
                ecs.write_resource::<crate::scripting::ScriptEventQueue>()
                    .push(crate::scripting::ScriptEvent::Spawn { e: _unit_entity });

                log::info!("創建訓練敵人單位 '{}' 於位置 ({}, {})", enemy_data.name, x, y);
            }
        }
    }

    fn create_terrain_blockers(ecs: &mut World) {
        // 創建地形遮擋物
        log::info!("地形遮擋物創建（新視野系統待實現）");
    }
}