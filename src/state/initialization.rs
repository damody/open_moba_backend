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

        // 設置路徑
        {
            let cps = {
                let cps_resource = ecs.read_resource::<BTreeMap<String, CheckPoint>>();
                cps_resource.clone()
            };
            let mut paths = ecs.get_mut::<BTreeMap<String, Path>>().unwrap();
            for p in cw.Path.iter() {
                let mut cp_in_path = vec![];
                for ps in p.Points.iter() {
                    if let Some(v) = cps.get(ps) {
                        cp_in_path.push(v.clone());
                    }
                }
                paths.insert(p.Name.clone(), 
                    Path { check_points: cp_in_path });
            }
        }

        // 設置小兵發射器
        {
            let mut ces = ecs.get_mut::<BTreeMap<String, CreepEmiter>>().unwrap();
            for cp in cw.Creep.iter() {
                ces.insert(cp.Name.clone(), CreepEmiter { 
                    root: Creep {
                        name: cp.Name.clone(), 
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
                    } 
                });
            }
        }

        // 設置小兵波
        {
            let mut cws = ecs.get_mut::<Vec<CreepWave>>().unwrap();
            for cw in cw.CreepWave.iter() {
                let mut tcw = CreepWave { time: cw.StartTime, path_creeps: vec![] };
                for d in cw.Detail.iter() {
                    let mut es = vec![];
                    for cjd in d.Creeps.iter() {
                        es.push(CreepEmit { time: cjd.Time, name: cjd.Creep.clone() });
                    }
                    tcw.path_creeps.push(PathCreeps { creeps: es, path_name: d.Path.clone() });
                }
                cws.push(tcw);
            }
        }
    }

    /// 初始化戰役資料
    pub fn init_campaign_data(ecs: &mut World, campaign_data: &CampaignData) {
        // 插入戰役相關資源
        ecs.insert(campaign_data.clone());
        log::info!("初始化戰役資料: {}", campaign_data.name);
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
        Self::create_training_enemies(ecs, campaign_data);
        Self::create_terrain_blockers(ecs);
        log::info!("創建戰役場景完成: {}", campaign_data.name);
    }

    // 私有輔助方法
    fn register_components(ecs: &mut World) {
        // 註冊所有遊戲組件
        ecs.register::<Pos>();
        ecs.register::<Vel>();
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
        ecs.register::<Skill>();
        ecs.register::<Last<Pos>>();
        ecs.register::<Last<Vel>>();
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
        ecs.insert(Vec::<CreepWave>::new());
        ecs.insert(Vec::<crate::Outcome>::new());

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
        // 設置戰役特有的資源
        log::info!("設置戰役特有資源");
    }

    fn create_campaign_heroes(ecs: &mut World, campaign_data: &CampaignData) {
        // 從戰役資料創建英雄
        if let Some(hero_data) = campaign_data.entity.heroes.first() {
            let hero = Hero::from_campaign_data(hero_data);
            let hero_faction = Faction::new(FactionType::Player, 0);
            let hero_pos = Pos(Vec2::new(0.0, 0.0));
            let hero_vel = Vel(Vec2::new(0.0, 0.0));

            // 創建英雄的戰鬥屬性
            let hero_properties = CProperty {
                hp: hero.current_hp as f32,
                mhp: hero.max_hp as f32,
                msd: hero.move_speed,
                def_physic: hero.base_armor,
                def_magic: hero.magic_resistance,
            };

            let hero_attack = TAttack {
                atk_physic: Vf32::new(hero.base_damage as f32),
                asd: Vf32::new(1.0 / hero.attack_speed),
                range: Vf32::new(hero.attack_range),
                asd_count: 0.0,
                bullet_speed: 1000.0,
            };

            // 創建英雄圓形視野組件
            let hero_vision = CircularVision::new(
                hero.vision_range.unwrap_or(1200.0),
                hero.height.unwrap_or(180.0)
            ).with_precision(720); // 高精度視野

            let hero_entity = ecs.create_entity()
                .with(hero_pos)
                .with(hero_vel)
                .with(hero)
                .with(hero_faction)
                .with(hero_properties)
                .with(hero_attack)
                .with(hero_vision)
                .build();

            log::info!("創建戰役英雄實體: {:?}", hero_entity);
        }
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