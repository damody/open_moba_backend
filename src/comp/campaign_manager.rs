use std::collections::BTreeMap;
use specs::{World, WorldExt, Builder};

use crate::comp::*;
use crate::ue4::import_campaign::CampaignData;
use omoba_sim::Fixed32;

/// TODO Phase 1d: drop when Unit / CircularVision migrate to Fixed32 fully.
#[inline]
fn f32_to_fx(v: f32) -> Fixed32 {
    Fixed32::from_raw((v * omoba_sim::fixed::SCALE as f32) as i32)
}

pub struct CampaignManager;

impl CampaignManager {
    pub fn init_campaign_data(ecs: &mut World, campaign_data: &CampaignData) {
        log::info!("Initializing campaign data for: {}", campaign_data.mission.campaign.name);
        
        // 初始化英雄
        let mut heroes = ecs.get_mut::<BTreeMap<String, Hero>>().unwrap();
        for hero_data in &campaign_data.entity.heroes {
            let hero = Hero::from_campaign_data(hero_data);
            log::info!("Loading hero: {} - {}", hero.name, hero.title);
            heroes.insert(hero.id.clone(), hero);
        }
        
        // 舊 Ability ECS resource 已隨 skill_system 移除；技能 metadata
        // 改由 scripts/base_content DLL 透過 AbilityScript 註冊，可透過
        // `AbilityRegistry` resource 查詢（scripting/registry.rs）。
        log::info!(
            "Campaign has {} ability defs (metadata now served by DLL registry)",
            campaign_data.ability.abilities.len()
        );

        // 初始化敵人
        let mut enemies = ecs.get_mut::<BTreeMap<String, Enemy>>().unwrap();
        for enemy_data in &campaign_data.entity.enemies {
            let enemy = Enemy::from_campaign_data(enemy_data);
            log::info!("Loading enemy: {} ({})", enemy.name, enemy.id);
            enemies.insert(enemy.id.clone(), enemy);
        }
        
        // 創建戰役組件
        let campaign = Campaign::from_campaign_data(&campaign_data.mission.campaign);
        let campaign_entity = ecs.create_entity().with(campaign).build();
        
        // 創建關卡組件
        for stage_data in &campaign_data.mission.stages {
            let stage = Stage::from_campaign_data(stage_data, campaign_data.mission.campaign.id.clone());
            let stage_entity = ecs.create_entity().with(stage).build();
            log::info!("Loading stage: {} ({})", stage_data.name, stage_data.id);
        }
        
        log::info!("Campaign initialization completed");
    }
    
    pub fn create_campaign_scene(ecs: &mut World, campaign_data: &CampaignData) {
        log::info!("Creating campaign scene for: {}", campaign_data.mission.campaign.name);
        
        match campaign_data.mission.campaign.difficulty.as_str() {
            "tutorial" => Self::create_tutorial_scene(ecs, campaign_data),
            _ => Self::create_training_scene(ecs, campaign_data),
        }
    }
    
    fn create_tutorial_scene(ecs: &mut World, campaign_data: &CampaignData) {
        log::info!("Setting up tutorial scene");
    }
    
    fn create_training_scene(ecs: &mut World, campaign_data: &CampaignData) {
        log::info!("Setting up training scene for sniper practice");
        
        if let Some(hero_data) = campaign_data.entity.heroes.first() {
            Self::create_hero_entity(ecs, hero_data, campaign_data);
            Self::create_training_enemies(ecs, campaign_data);
            Self::create_terrain_blockers(ecs);
        }
    }
    
    fn create_hero_entity(ecs: &mut World, hero_data: &crate::ue4::import_campaign::HeroJD, campaign_data: &CampaignData) {
        use omoba_template_ids::{hero_abilities, hero_by_name, hero_stats};
        let hero = Hero::from_campaign_data(hero_data);
        // 從 templates.json 取 attack_range / base_armor — entity.json hero 條目已 slim
        // 成只剩 id，campaign-specific 的 stats 來源唯一。
        let id = hero_by_name(&hero_data.id)
            .unwrap_or_else(|| panic!("hero id '{}' not in templates.json", hero_data.id));
        let s = hero_stats(id)
            .unwrap_or_else(|| panic!("hero '{}' has no stats in templates.json", hero_data.id));
        // Phase 1c.4: CProperty / TAttack are Fixed32 (Phase 1c.2). Pass Fixed32 直送。
        let hero_properties = Self::create_hero_properties(&hero, s.base_armor);
        let hero_attack = Self::create_hero_attack(&hero, s.attack_range);
        let abilities: Vec<String> = if hero_data.abilities.is_empty() {
            hero_abilities(id).iter().map(|a| a.as_str().to_string()).collect()
        } else {
            hero_data.abilities.clone()
        };

        // TODO Phase 1[d]: max_hp / current_hp / base_damage 仍 i32 (Unit struct)；
        // 透過 to_f32_for_render() as i32 在邊界轉。
        let max_hp_i = hero.get_max_hp().to_f32_for_render() as i32;
        let base_damage_i = hero.get_base_damage().to_f32_for_render() as i32;
        let attack_range_fx = s.attack_range;
        let hero_unit = Unit {
            id: hero.id.clone(),
            name: hero.name.clone(),
            unit_type: UnitType::Hero,
            max_hp: max_hp_i,
            current_hp: max_hp_i,
            base_armor: s.base_armor,
            magic_resistance: Fixed32::ZERO,
            base_damage: base_damage_i,
            attack_range: attack_range_fx,
            move_speed: hero.get_move_speed(),
            attack_speed: hero.get_attack_speed_multiplier(),
            ai_type: unit::AiType::None,
            aggro_range: attack_range_fx + Fixed32::from_i32(200),
            abilities: abilities.clone(),
            current_target: None,
            last_attack_time: Fixed32::ZERO,
            spawn_position: (0.0, 0.0),
            exp_reward: 0,
            gold_reward: 0,
            bounty_type: BountyType::None,
        };

        let hero_faction = Faction::new(FactionType::Player, 0);
        let hero_pos = Pos::from_xy_f32(0.0, 0.0);
        let hero_vel = Vel::zero();
        // TODO Phase 1d: CircularVision::new still takes f32; drop on full migration.
        let hero_vision = CircularVision::new(
            (attack_range_fx + Fixed32::from_i32(300)).to_f32_for_render(),
            30.0,
        ).with_precision(720);

        let hero_entity = ecs.create_entity()
            .with(hero_pos)
            .with(hero_vel)
            .with(hero)
            .with(hero_unit)
            .with(hero_faction)
            .with(hero_properties)
            .with(hero_attack)
            .with(hero_vision)
            .build();

        log::info!("Created hero entity '{}' with full combat components", hero_data.id);
        Self::create_hero_abilities(ecs, hero_entity, &abilities, campaign_data);
    }

    fn create_hero_properties(hero: &Hero, base_armor: Fixed32) -> CProperty {
        let max_hp = hero.get_max_hp();
        let move_speed = hero.get_move_speed();

        CProperty {
            hp: max_hp,
            mhp: max_hp,
            msd: move_speed,
            def_physic: base_armor,
            def_magic: Fixed32::ZERO,
        }
    }

    fn create_hero_attack(hero: &Hero, attack_range: Fixed32) -> TAttack {
        let base_damage = hero.get_base_damage();
        let attack_speed_multiplier = hero.get_attack_speed_multiplier();
        // 1.0 / attack_speed_multiplier — Fixed32 division.
        let attack_interval = Fixed32::ONE / attack_speed_multiplier;

        TAttack {
            atk_physic: Vf32::new(base_damage),
            asd: Vf32::new(attack_interval),
            range: Vf32::new(attack_range),
            asd_count: Fixed32::ZERO,
            bullet_speed: Fixed32::from_i32(1000),
        }
    }
    
    fn create_hero_abilities(_ecs: &mut World, _hero_entity: specs::Entity, ability_ids: &[String], _campaign_data: &CampaignData) {
        // 舊路徑：為每個 ability 建 Ability/Skill ECS entity。
        // 新架構：hero.abilities: Vec<String> + hero.ability_levels: HashMap<String, i32>
        // 已承載玩家習得狀態；ability 邏輯由 DLL AbilityScript 執行，不需要
        // ECS Component。這個函式保留空殼以相容呼叫點。
        log::debug!(
            "[campaign_manager] skipping legacy Ability/Skill entity creation for {} abilities (handled by AbilityRegistry / AbilityScript)",
            ability_ids.len()
        );
    }
    
    fn create_training_enemies(ecs: &mut World, campaign_data: &CampaignData) {
        let enemy_positions = [(800.0, 0.0), (1000.0, 100.0), (1200.0, -50.0)];
        
        for (i, (x, y)) in enemy_positions.iter().enumerate() {
            if let Some(enemy_data) = campaign_data.entity.enemies.get(i % campaign_data.entity.enemies.len()) {
                let unit = Unit::from_enemy_data(enemy_data);
                let enemy_faction = Faction::new(FactionType::Enemy, 1);
                let unit_pos = Pos::from_xy_f32(*x, *y);
                let unit_vel = Vel::zero();

                // Phase 1c.4: CProperty / TAttack 全 Fixed32；Unit.{current_hp,max_hp,base_damage}
                // 仍 i32 (Phase 1d)。在邊界用 Fixed32::from_i32 轉。
                let unit_properties = CProperty {
                    hp: Fixed32::from_i32(unit.current_hp),
                    mhp: Fixed32::from_i32(unit.max_hp),
                    msd: unit.move_speed,
                    def_physic: unit.base_armor,
                    def_magic: unit.magic_resistance,
                };

                let unit_attack = TAttack {
                    atk_physic: Vf32::new(Fixed32::from_i32(unit.base_damage)),
                    asd: Vf32::new(Fixed32::ONE / unit.attack_speed),
                    range: Vf32::new(unit.attack_range),
                    asd_count: Fixed32::ZERO,
                    bullet_speed: Fixed32::from_i32(800),
                };

                // TODO Phase 1d: CircularVision::new still takes f32.
                let enemy_vision = CircularVision::new(
                    (unit.attack_range + Fixed32::from_i32(150)).to_f32_for_render(),
                    20.0,
                ).with_precision(360);

                let unit_entity = ecs.create_entity()
                    .with(unit_pos)
                    .with(unit_vel)
                    .with(unit)
                    .with(enemy_faction)
                    .with(unit_properties)
                    .with(unit_attack)
                    .with(enemy_vision)
                    .build();

                log::info!("Created training enemy unit '{}' at position ({}, {})", enemy_data.id, x, y);
            }
        }
        
        Self::create_training_creeps(ecs, campaign_data);
    }
    
    fn create_training_creeps(ecs: &mut World, campaign_data: &CampaignData) {
        let creep_positions = [(600.0, 50.0), (1500.0, 0.0), (1300.0, 150.0)];
        
        for (i, (x, y)) in creep_positions.iter().enumerate() {
            if let Some(creep_data) = campaign_data.entity.creeps.get(i % campaign_data.entity.creeps.len()) {
                let unit = Unit::from_creep_data(creep_data);
                let creep_faction = Faction::new(FactionType::Enemy, 2);
                let unit_pos = Pos::from_xy_f32(*x, *y);
                let unit_vel = Vel::zero();
                
                // Phase 1c.4: CProperty / TAttack 全 Fixed32；Unit.{current_hp,max_hp,base_damage}
                // 仍 i32 (Phase 1d)。在邊界用 Fixed32::from_i32 轉。
                let unit_properties = CProperty {
                    hp: Fixed32::from_i32(unit.current_hp),
                    mhp: Fixed32::from_i32(unit.max_hp),
                    msd: unit.move_speed,
                    def_physic: unit.base_armor,
                    def_magic: unit.magic_resistance,
                };

                let unit_attack = TAttack {
                    atk_physic: Vf32::new(Fixed32::from_i32(unit.base_damage)),
                    asd: Vf32::new(Fixed32::ONE / unit.attack_speed),
                    range: Vf32::new(unit.attack_range),
                    asd_count: Fixed32::ZERO,
                    bullet_speed: Fixed32::from_i32(600),
                };
                
                let unit_entity = ecs.create_entity()
                    .with(unit_pos)
                    .with(unit_vel)
                    .with(unit)
                    .with(creep_faction)
                    .with(unit_properties)
                    .with(unit_attack)
                    .build();
                    
                log::info!("Created training creep unit '{}' at position ({}, {})", creep_data.id, x, y);
            }
        }
    }
    
    fn create_terrain_blockers(ecs: &mut World) {
        log::info!("Terrain blockers creation skipped (old system), will be implemented in new circular vision system");
    }
}