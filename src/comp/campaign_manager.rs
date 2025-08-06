use std::collections::BTreeMap;
use specs::{World, WorldExt, Builder};

use crate::comp::*;
use crate::ue4::import_campaign::CampaignData;

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
        
        // 初始化技能
        let mut abilities = ecs.get_mut::<BTreeMap<String, Ability>>().unwrap();
        for (ability_id, ability_data) in &campaign_data.ability.abilities {
            let ability = Ability::from_campaign_data(ability_data);
            log::info!("Loading ability: {} ({})", ability.name, ability.key_binding);
            abilities.insert(ability_id.clone(), ability);
        }
        
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
        let hero = Hero::from_campaign_data(hero_data);
        let hero_properties = Self::create_hero_properties(&hero, hero_data);
        let hero_attack = Self::create_hero_attack(&hero, hero_data);
        
        let hero_unit = Unit {
            id: hero.id.clone(),
            name: hero.name.clone(),
            unit_type: UnitType::Hero,
            max_hp: hero.get_max_hp() as i32,
            current_hp: hero.get_max_hp() as i32,
            base_armor: hero_data.base_armor,
            magic_resistance: 0.0,
            base_damage: hero.get_base_damage() as i32,
            attack_range: hero_data.attack_range,
            move_speed: hero.get_move_speed(),
            attack_speed: hero.get_attack_speed_multiplier(),
            ai_type: unit::AiType::None,
            aggro_range: hero_data.attack_range + 200.0,
            abilities: hero_data.abilities.clone(),
            current_target: None,
            last_attack_time: 0.0,
            spawn_position: (0.0, 0.0),
            exp_reward: 0,
            gold_reward: 0,
            bounty_type: BountyType::None,
        };
        
        let hero_faction = Faction::new(FactionType::Player, 0);
        let hero_pos = Pos(vek::Vec2::new(0.0, 0.0));
        let hero_vel = Vel(vek::Vec2::new(0.0, 0.0));
        let hero_vision = CircularVision::new(hero_data.attack_range + 300.0, 30.0).with_precision(720);

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
            
        log::info!("Created hero entity '{}' with full combat components", hero_data.name);
        Self::create_hero_abilities(ecs, hero_entity, &hero_data.abilities, campaign_data);
    }
    
    fn create_hero_properties(hero: &Hero, hero_data: &crate::ue4::import_campaign::HeroJD) -> CProperty {
        let max_hp = hero.get_max_hp();
        let move_speed = hero.get_move_speed();
        
        CProperty {
            hp: max_hp,
            mhp: max_hp,
            msd: move_speed,
            def_physic: hero_data.base_armor,
            def_magic: 0.0,
        }
    }
    
    fn create_hero_attack(hero: &Hero, hero_data: &crate::ue4::import_campaign::HeroJD) -> TAttack {
        let base_damage = hero.get_base_damage();
        let attack_speed_multiplier = hero.get_attack_speed_multiplier();
        let attack_interval = 1.0 / attack_speed_multiplier;
        
        TAttack {
            atk_physic: Vf32::new(base_damage),
            asd: Vf32::new(attack_interval),
            range: Vf32::new(hero_data.attack_range),
            asd_count: 0.0,
            bullet_speed: 1000.0,
        }
    }
    
    fn create_hero_abilities(ecs: &mut World, hero_entity: specs::Entity, ability_ids: &[String], campaign_data: &CampaignData) {
        for ability_id in ability_ids {
            if let Some(ability_data) = campaign_data.ability.abilities.get(ability_id) {
                let mut ability = Ability::from_campaign_data(ability_data);
                
                let initial_level = if ability_id == "sniper_mode" || ability_id == "saika_reinforcements" {
                    1
                } else {
                    0
                };
                ability.current_level = initial_level;
                
                let ability_entity = ecs.create_entity()
                    .with(ability)
                    .build();
                    
                let mut skill = Skill::new(ability_id.clone(), hero_entity);
                skill.current_level = initial_level;
                skill.level_up();
                
                let skill_entity = ecs.create_entity()
                    .with(skill)
                    .build();
                    
                log::info!("Created ability '{}' and skill instance for hero", ability_data.name);
            }
        }
    }
    
    fn create_training_enemies(ecs: &mut World, campaign_data: &CampaignData) {
        let enemy_positions = [(800.0, 0.0), (1000.0, 100.0), (1200.0, -50.0)];
        
        for (i, (x, y)) in enemy_positions.iter().enumerate() {
            if let Some(enemy_data) = campaign_data.entity.enemies.get(i % campaign_data.entity.enemies.len()) {
                let unit = Unit::from_enemy_data(enemy_data);
                let enemy_faction = Faction::new(FactionType::Enemy, 1);
                let unit_pos = Pos(vek::Vec2::new(*x, *y));
                let unit_vel = Vel(vek::Vec2::new(0.0, 0.0));
                
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
                
                let enemy_vision = CircularVision::new(unit.attack_range + 150.0, 20.0).with_precision(360);

                let unit_entity = ecs.create_entity()
                    .with(unit_pos)
                    .with(unit_vel)
                    .with(unit)
                    .with(enemy_faction)
                    .with(unit_properties)
                    .with(unit_attack)
                    .with(enemy_vision)
                    .build();
                    
                log::info!("Created training enemy unit '{}' at position ({}, {})", enemy_data.name, x, y);
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
                let unit_pos = Pos(vek::Vec2::new(*x, *y));
                let unit_vel = Vel(vek::Vec2::new(0.0, 0.0));
                
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
                    bullet_speed: 600.0,
                };
                
                let unit_entity = ecs.create_entity()
                    .with(unit_pos)
                    .with(unit_vel)
                    .with(unit)
                    .with(creep_faction)
                    .with(unit_properties)
                    .with(unit_attack)
                    .build();
                    
                log::info!("Created training creep unit '{}' at position ({}, {})", creep_data.name, x, y);
            }
        }
    }
    
    fn create_terrain_blockers(ecs: &mut World) {
        log::info!("Terrain blockers creation skipped (old system), will be implemented in new circular vision system");
    }
}