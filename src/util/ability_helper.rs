use ability_system::{AbilityRequest};
use specs::Entity;
use vek::Vec2;

/// 技能助手函數，方便創建技能請求
pub struct AbilityHelper;

impl AbilityHelper {
    /// 創建無目標技能請求
    pub fn create_self_cast_request(caster: Entity, ability_id: &str, level: u8) -> AbilityRequest {
        AbilityRequest {
            caster,
            ability_id: ability_id.to_string(),
            level,
            target_position: None,
            target_entity: None,
        }
    }

    /// 創建地面目標技能請求
    pub fn create_point_cast_request(caster: Entity, ability_id: &str, level: u8, target_pos: Vec2<f32>) -> AbilityRequest {
        AbilityRequest {
            caster,
            ability_id: ability_id.to_string(),
            level,
            target_position: Some(target_pos),
            target_entity: None,
        }
    }

    /// 創建單位目標技能請求
    pub fn create_unit_cast_request(caster: Entity, ability_id: &str, level: u8, target: Entity) -> AbilityRequest {
        AbilityRequest {
            caster,
            ability_id: ability_id.to_string(),
            level,
            target_position: None,
            target_entity: Some(target),
        }
    }

    /// 創建狙擊模式切換請求
    pub fn toggle_sniper_mode(caster: Entity, level: u8) -> AbilityRequest {
        Self::create_self_cast_request(caster, "sniper_mode", level)
    }

    /// 創建雜賀眾召喚請求
    pub fn summon_saika_reinforcements(caster: Entity, level: u8, position: Vec2<f32>) -> AbilityRequest {
        Self::create_point_cast_request(caster, "saika_reinforcements", level, position)
    }

    /// 創建雨鐵炮請求
    pub fn cast_rain_iron_cannon(caster: Entity, level: u8, position: Vec2<f32>) -> AbilityRequest {
        Self::create_point_cast_request(caster, "rain_iron_cannon", level, position)
    }

    /// 創建三段擊請求
    pub fn cast_three_stage_technique(caster: Entity, level: u8, target: Entity) -> AbilityRequest {
        Self::create_unit_cast_request(caster, "three_stage_technique", level, target)
    }
}