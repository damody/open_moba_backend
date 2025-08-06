use std::sync::Arc;
use rayon::ThreadPool;
use specs::{World, WorldExt};
use std::collections::BTreeMap;
use std::time::Instant;

use crate::comp::*;
use crate::tick::*;
use crate::ue4::import_campaign::CampaignData;

pub struct EcsSetup;

impl EcsSetup {
    pub fn setup_ecs_world(thread_pool: &Arc<ThreadPool>) -> specs::World {
        let mut ecs = specs::World::new();
        
        // Register all components.
        ecs.register::<Pos>();
        ecs.register::<Vel>();
        ecs.register::<TProperty>();
        ecs.register::<TAttack>();
        ecs.register::<CProperty>();
        ecs.register::<Tower>();
        ecs.register::<Creep>();
        ecs.register::<Projectile>();
        
        // Register unsynced resources used by the ECS.
        ecs.insert(TimeOfDay(0.0));
        ecs.insert(Time(0.0));
        ecs.insert(DeltaTime(0.0));
        ecs.insert(Tick(0));
        ecs.insert(TickStart(Instant::now()));
        ecs.insert(SysMetrics::default());
        ecs.insert(Vec::<Outcome>::new());
        ecs.insert(Vec::<TakenDamage>::new());
        ecs.insert(Vec::<CreepWave>::new());
        ecs.insert(CurrentCreepWave{wave: 0, path: vec![]});
        ecs.insert(BTreeMap::<String, Player>::new());
        ecs.insert(BTreeMap::<String, CreepEmiter>::new());
        ecs.insert(BTreeMap::<String, Path>::new());
        ecs.insert(BTreeMap::<String, CheckPoint>::new());
        ecs.insert(Searcher::default());

        // Set starting time for the server.
        ecs.write_resource::<TimeOfDay>().0 = 0.0;
        ecs
    }
    
    pub fn setup_ecs_world_with_campaign(thread_pool: &Arc<ThreadPool>) -> specs::World {
        let mut ecs = Self::setup_ecs_world(thread_pool);
        
        // 註冊戰役相關組件
        ecs.register::<Hero>();
        ecs.register::<Ability>();
        ecs.register::<AbilityEffect>();
        ecs.register::<Enemy>();
        ecs.register::<Campaign>();
        ecs.register::<Stage>();
        ecs.register::<Unit>();
        ecs.register::<Faction>();
        ecs.register::<DamageInstance>();
        ecs.register::<DamageResult>();
        ecs.register::<Skill>();
        ecs.register::<SkillEffect>();
        ecs.register::<CircularVision>();
        
        // 戰役相關資源
        ecs.insert(BTreeMap::<String, Hero>::new());
        ecs.insert(BTreeMap::<String, Ability>::new());
        ecs.insert(BTreeMap::<String, Enemy>::new());
        ecs.insert(Vec::<AbilityEffect>::new());
        ecs.insert(Vec::<DamageInstance>::new());
        ecs.insert(Vec::<SkillInput>::new());
        
        // 載入地形高度圖
        if let Ok(heightmap) = Self::load_terrain_heightmap() {
            ecs.insert(heightmap);
            log::info!("地形高度圖載入成功");
        } else {
            log::warn!("地形高度圖載入失敗，使用後備視野系統");
        }
        
        ecs
    }
    
    fn load_terrain_heightmap() -> Result<TerrainHeightMap, failure::Error> {
        use failure::err_msg;
        
        let config_path = "example/terrain_heightmap.json";
        
        let config_content = std::fs::read_to_string(config_path)
            .map_err(|e| err_msg(format!("無法讀取地形配置文件 {}: {}", config_path, e)))?;
        
        let config: crate::comp::heightmap::TerrainConfig = serde_json::from_str(&config_content)
            .map_err(|e| err_msg(format!("地形配置文件解析失敗: {}", e)))?;
        
        let heightmap = TerrainHeightMap::from_config(&config);
        
        log::info!("載入地形配置: {}x{} 米，網格大小 {} 米", 
            config.map_width, config.map_height, config.grid_size);
        log::info!("地形區域數量: {}", config.terrain_regions.len());
        
        Ok(heightmap)
    }
}