/// 技能系統綜合測試
/// 
/// 測試所有英雄技能的完整功能

use ability_system::*;
use specs::{Entity, world::Generation};
use std::collections::HashMap;
use vek::Vec2;

fn create_test_entity() -> Entity {
    Entity::new(1, Generation::new(1))
}

fn create_ability_config(name: &str, target_type: TargetType) -> AbilityConfig {
    AbilityConfig {
        name: name.to_string(),
        description: "測試技能".to_string(),
        ability_type: AbilityType::Active,
        target_type,
        cast_type: CastType::Instant,
        levels: HashMap::new(),
        properties: HashMap::new(),
    }
}

fn create_level_data(cooldown: f32, mana: f32, range: f32, extra_data: HashMap<String, serde_json::Value>) -> AbilityLevelData {
    AbilityLevelData {
        cooldown,
        mana_cost: mana,
        cast_time: 0.0,
        range,
        extra: extra_data,
    }
}

#[test]
fn test_ability_processor_initialization() {
    let processor = AbilityProcessor::new();
    let registry = processor.get_registry();
    
    // 測試雜賀孫市技能註冊
    assert!(registry.get_handler("sniper_mode").is_some());
    assert!(registry.get_handler("saika_reinforcements").is_some());
    assert!(registry.get_handler("rain_iron_cannon").is_some());
    assert!(registry.get_handler("three_stage_technique").is_some());
    
    // 測試伊達政宗技能註冊
    assert!(registry.get_handler("flame_blade").is_some());
    assert!(registry.get_handler("fire_dash").is_some());
    assert!(registry.get_handler("flame_assault").is_some());
    assert!(registry.get_handler("matchlock_gun").is_some());
}

#[test]
fn test_saika_magoichi_abilities() {
    let processor = AbilityProcessor::new();
    let registry = processor.get_registry();
    let caster = create_test_entity();
    
    // 測試狙擊模式
    if let Some(handler) = registry.get_handler("sniper_mode") {
        let request = AbilityRequest {
            caster,
            ability_id: "sniper_mode".to_string(),
            level: 1,
            target_position: None,
            target_entity: None,
        };
        
        let config = create_ability_config("狙擊模式", TargetType::None);
        let mut extra = HashMap::new();
        extra.insert("range_bonus".to_string(), serde_json::Value::from(200.0));
        extra.insert("damage_bonus".to_string(), serde_json::Value::from(0.25));
        let level_data = create_level_data(0.0, 0.0, 0.0, extra);
        
        let effects = handler.execute(&request, &config, &level_data);
        assert!(effects.len() >= 1);
    }
    
    // 測試雜賀眾召喚
    if let Some(handler) = registry.get_handler("saika_reinforcements") {
        let request = AbilityRequest {
            caster,
            ability_id: "saika_reinforcements".to_string(),
            level: 2,
            target_position: Some(Vec2::new(500.0, 300.0)),
            target_entity: None,
        };
        
        let config = create_ability_config("雜賀眾", TargetType::Point);
        let mut extra = HashMap::new();
        extra.insert("summon_count".to_string(), serde_json::Value::from(2));
        extra.insert("duration".to_string(), serde_json::Value::from(30.0));
        let level_data = create_level_data(30.0, 80.0, 800.0, extra);
        
        let effects = handler.execute(&request, &config, &level_data);
        
        // 檢查是否有召喚效果
        let summon_effects: Vec<_> = effects.iter().filter(|e| {
            matches!(e, AbilityEffect::Summon { .. })
        }).collect();
        
        assert!(summon_effects.len() >= 1);
    }
}

#[test]
fn test_date_masamune_abilities() {
    let processor = AbilityProcessor::new();
    let registry = processor.get_registry();
    let caster = create_test_entity();
    
    // 測試火焰刀
    if let Some(handler) = registry.get_handler("flame_blade") {
        let request = AbilityRequest {
            caster,
            ability_id: "flame_blade".to_string(),
            level: 1,
            target_position: Some(Vec2::new(500.0, 300.0)),
            target_entity: None,
        };
        
        let config = create_ability_config("火焰刀", TargetType::Point);
        let mut extra = HashMap::new();
        extra.insert("damage".to_string(), serde_json::Value::from(200.0));
        let level_data = create_level_data(10.0, 100.0, 1100.0, extra);
        
        let effects = handler.execute(&request, &config, &level_data);
        assert!(effects.len() >= 1);
        
        // 檢查是否有區域效果或傷害效果
        let has_effect = effects.iter().any(|e| {
            matches!(e, AbilityEffect::AreaEffect { .. } | AbilityEffect::Damage { .. })
        });
        assert!(has_effect);
    }
    
    // 測試火繩銃變身
    if let Some(handler) = registry.get_handler("matchlock_gun") {
        let request = AbilityRequest {
            caster,
            ability_id: "matchlock_gun".to_string(),
            level: 2,
            target_position: None,
            target_entity: None,
        };
        
        let config = create_ability_config("火繩銃", TargetType::None);
        let mut extra = HashMap::new();
        extra.insert("duration".to_string(), serde_json::Value::from(45.0));
        extra.insert("range_bonus".to_string(), serde_json::Value::from(800.0));
        extra.insert("damage_bonus".to_string(), serde_json::Value::from(130.0));
        let level_data = create_level_data(110.0, 310.0, 800.0, extra);
        
        let effects = handler.execute(&request, &config, &level_data);
        assert!(effects.len() >= 4); // 應該有多個狀態修改效果
        
        // 檢查是否有狀態修改效果
        let status_effects: Vec<_> = effects.iter().filter(|e| {
            matches!(e, AbilityEffect::StatusModifier { .. })
        }).collect();
        
        assert!(status_effects.len() >= 4);
    }
}

#[test]
fn test_ability_effect_types() {
    let processor = AbilityProcessor::new();
    let registry = processor.get_registry();
    let caster = create_test_entity();
    
    // 測試各種效果類型
    let test_cases = vec![
        ("flame_blade", Some(Vec2::new(0.0, 0.0)), None),
        ("fire_dash", Some(Vec2::new(100.0, 100.0)), None),
        ("flame_assault", Some(Vec2::new(200.0, 200.0)), None),
        ("matchlock_gun", None, None),
        ("saika_reinforcements", Some(Vec2::new(300.0, 300.0)), None),
    ];
    
    for (ability_id, target_pos, target_entity) in test_cases {
        if let Some(handler) = registry.get_handler(ability_id) {
            let request = AbilityRequest {
                caster,
                ability_id: ability_id.to_string(),
                level: 1,
                target_position: target_pos,
                target_entity,
            };
            
            let config = create_ability_config(ability_id, TargetType::Point);
            let level_data = AbilityLevelData::default();
            
            let effects = handler.execute(&request, &config, &level_data);
            
            // 每個技能都應該產生至少一個效果
            assert!(!effects.is_empty(), "技能 {} 沒有產生任何效果", ability_id);
        }
    }
}

#[test] 
fn test_ability_handler_trait_methods() {
    let processor = AbilityProcessor::new();
    let registry = processor.get_registry();
    
    let abilities = vec![
        "sniper_mode", "saika_reinforcements", "rain_iron_cannon", "three_stage_technique",
        "flame_blade", "fire_dash", "flame_assault", "matchlock_gun"
    ];
    
    for ability_id in abilities {
        if let Some(handler) = registry.get_handler(ability_id) {
            // 測試基本方法
            assert_eq!(handler.get_ability_id(), ability_id);
            assert!(!handler.get_description().is_empty());
            
            // 測試 can_execute 方法
            let request = AbilityRequest {
                caster: create_test_entity(),
                ability_id: ability_id.to_string(),
                level: 1,
                target_position: Some(Vec2::new(0.0, 0.0)),
                target_entity: None,
            };
            
            let config = create_ability_config(ability_id, TargetType::Point);
            let state = AbilityState::default();
            
            // 在默認狀態下應該可以執行
            assert!(handler.can_execute(&request, &config, &state));
        }
    }
}

#[test]
fn test_summon_effects() {
    let processor = AbilityProcessor::new();
    let registry = processor.get_registry();
    let caster = create_test_entity();
    
    // 專門測試召喚技能
    if let Some(handler) = registry.get_handler("saika_reinforcements") {
        let request = AbilityRequest {
            caster,
            ability_id: "saika_reinforcements".to_string(),
            level: 3,
            target_position: Some(Vec2::new(500.0, 300.0)),
            target_entity: None,
        };
        
        let config = create_ability_config("雜賀眾", TargetType::Point);
        let mut extra = HashMap::new();
        extra.insert("summon_count".to_string(), serde_json::Value::from(3));
        extra.insert("duration".to_string(), serde_json::Value::from(30.0));
        extra.insert("formation_radius".to_string(), serde_json::Value::from(100.0));
        let level_data = create_level_data(30.0, 120.0, 800.0, extra);
        
        let effects = handler.execute(&request, &config, &level_data);
        
        // 檢查召喚效果
        for effect in &effects {
            if let AbilityEffect::Summon { position, unit_type, count, duration } = effect {
                assert_eq!(unit_type, "saika_gunner");
                assert_eq!(*count, 1); // 每個效果召喚1個單位
                assert_eq!(*duration, Some(30.0));
                // 位置應該在目標附近
                assert!((position.distance(Vec2::new(500.0, 300.0))) <= 100.0);
            }
        }
    }
}