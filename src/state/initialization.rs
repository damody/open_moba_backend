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
            });
        }
    }

    /// 設置小兵波
    fn setup_creep_waves(ecs: &mut World, cw: &CreepWaveData) {
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
        // MVP_1 mode：跳過 training enemies，改用 MVP 場景
        if campaign_data.mission.campaign.id == "MVP_1" {
            Self::create_mvp_scene(ecs);
        } else {
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
        ecs.register::<CircularVision>();
        ecs.register::<Ability>();
        ecs.register::<AbilityEffect>();
        ecs.register::<Enemy>();
        ecs.register::<Campaign>();
        ecs.register::<Stage>();
        ecs.register::<DamageInstance>();
        ecs.register::<DamageResult>();
        ecs.register::<Skill>();
        ecs.register::<SkillEffect>();
        ecs.register::<MoveTarget>();
        ecs.register::<Player>();
        ecs.register::<Last<Pos>>();
        ecs.register::<Last<Vel>>();
        ecs.register::<Gold>();
        ecs.register::<Inventory>();
        ecs.register::<ItemEffects>();
        ecs.register::<IsBase>();
        ecs.register::<Bounty>();
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
        ecs.insert(CurrentCreepWave { wave: 0, path: vec![] });
        ecs.insert(Vec::<crate::Outcome>::new());
        ecs.insert(Vec::<TakenDamage>::new());
        ecs.insert(SysMetrics::default());
        
        // 初始化 MQTT 通道資源
        ecs.insert(Vec::<crossbeam_channel::Sender<crate::transport::OutboundMsg>>::new());
        
        // 初始化 Searcher 資源
        ecs.insert(crate::comp::outcome::Searcher::default());

        // 載入裝備 Registry (MVP LoL item system)
        let item_reg = crate::item::ItemRegistry::load_from_path("item-configs/items.json")
            .unwrap_or_else(|e| {
                log::warn!("裝備 Registry 載入失敗（{}），使用空 registry", e);
                crate::item::ItemRegistry::default()
            });
        ecs.insert(item_reg);

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
        
        // 設置戰役特有的資源
        ecs.insert(BTreeMap::<String, Hero>::new());
        ecs.insert(BTreeMap::<String, Ability>::new());
        ecs.insert(BTreeMap::<String, Enemy>::new());
        ecs.insert(Vec::<AbilityEffect>::new());
        ecs.insert(Vec::<DamageInstance>::new());
        ecs.insert(Vec::<SkillInput>::new());
        
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

            let hero_entity = ecs.create_entity()
                .with(hero_pos)
                .with(hero_vel)
                .with(hero)
                .with(hero_faction)
                .with(hero_properties)
                .with(hero_attack)
                .with(hero_vision)
                .with(Gold(0))
                .with(Inventory::new())
                .with(ItemEffects::default())
                .build();

            log::info!("創建戰役英雄實體: {:?}（含 Gold/Inventory/ItemEffects）", hero_entity);
        }
    }

    /// MVP_1 場景（LoL 風格單線）
    ///
    /// 位置布局（左下 → 右上對角，像 LoL 下路或上路）：
    /// ```text
    ///                                              敵方基地 (2400, 2400)
    ///                                        敵方塔 (1950, 1950)
    ///                                  敵方塔 (1500, 1500)
    ///                            -- 中線 --
    ///                      我方塔 (900, 900)
    ///                 我方塔 (500, 500)
    ///         我方基地 (0, 0)（hero spawn）
    /// ```
    pub fn create_mvp_scene(ecs: &mut World) {
        use specs::Builder;

        // 我方基地
        Self::spawn_tower(ecs, Vec2::new(0.0, 0.0), FactionType::Player, 3500.0, 800.0, 80.0, 1.0, true);
        // 我方塔 1 / 2（對角前推）
        Self::spawn_tower(ecs, Vec2::new(500.0, 500.0), FactionType::Player, 1500.0, 700.0, 55.0, 0.8, false);
        Self::spawn_tower(ecs, Vec2::new(900.0, 900.0), FactionType::Player, 1500.0, 700.0, 55.0, 0.8, false);
        // 敵方塔 1 / 2
        Self::spawn_tower(ecs, Vec2::new(1500.0, 1500.0), FactionType::Enemy, 1500.0, 700.0, 55.0, 0.8, false);
        Self::spawn_tower(ecs, Vec2::new(1950.0, 1950.0), FactionType::Enemy, 1500.0, 700.0, 55.0, 0.8, false);
        // 敵方基地（IsBase；擊毀判定勝負）
        Self::spawn_tower(ecs, Vec2::new(2400.0, 2400.0), FactionType::Enemy, 3500.0, 800.0, 80.0, 1.0, true);

        log::info!("MVP_1 場景已建立：1 我方基地 + 2 我方塔 + 2 敵方塔 + 1 敵方基地（對角左下→右上）");
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
            .with(bounty);

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

                let _unit_entity = ecs.create_entity()
                    .with(unit_pos)
                    .with(unit_vel)
                    .with(unit)
                    .with(enemy_faction)
                    .with(unit_properties)
                    .with(unit_attack)
                    .with(enemy_vision)
                    .build();

                log::info!("創建訓練敵人單位 '{}' 於位置 ({}, {})", enemy_data.name, x, y);
            }
        }
    }

    fn create_terrain_blockers(ecs: &mut World) {
        // 創建地形遮擋物
        log::info!("地形遮擋物創建（新視野系統待實現）");
    }
}