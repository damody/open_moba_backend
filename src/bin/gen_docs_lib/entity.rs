//! Load hero / creep catalog data from generated story + template Rust data.

use crate::lib::model::{CreepInfo, HeroInfo};
use anyhow::{Context, Result};
use std::collections::BTreeMap;

pub struct EntityData {
    pub heroes: BTreeMap<String, HeroInfo>,
    pub creeps: BTreeMap<String, CreepInfo>,
}

pub fn load(story_id: &str) -> Result<EntityData> {
    let campaign = omobab::ue4::import_campaign::CampaignData::load_generated(story_id)
        .with_context(|| format!("loading generated story {story_id}"))?;
    let mut heroes = BTreeMap::new();
    let mut creeps = BTreeMap::new();

    for hero in &campaign.entity.heroes {
        let id = hero.id.clone();
        let hid = omoba_template_ids::hero_by_name(&id)
            .with_context(|| format!("hero template missing: {id}"))?;
        let stats = omoba_template_ids::hero_stats(hid)
            .with_context(|| format!("hero template has no stats: {id}"))?;
        let abilities = if hero.abilities.is_empty() {
            omoba_template_ids::hero_abilities(hid)
                .iter()
                .map(|id| omoba_template_ids::ability_id_str(*id).to_string())
                .filter(|id| !id.is_empty())
                .collect()
        } else {
            hero.abilities.clone()
        };
        heroes.insert(id, hero_info(hid, stats, abilities));
    }

    for enemy in &campaign.entity.enemies {
        let abilities = enemy.abilities.clone();
        insert_creep(&mut creeps, &enemy.id, abilities)?;
    }
    for creep in &campaign.entity.creeps {
        insert_creep(&mut creeps, &creep.id, Vec::new())?;
    }
    for neutral in &campaign.entity.neutrals {
        insert_creep(&mut creeps, &neutral.id, neutral.abilities.clone())?;
    }

    Ok(EntityData { heroes, creeps })
}

fn hero_info(
    id: omoba_template_ids::HeroId,
    stats: omoba_template_ids::HeroStats,
    abilities: Vec<String>,
) -> HeroInfo {
    HeroInfo {
        name: omoba_template_ids::hero_display(id).to_string(),
        title: omoba_template_ids::hero_title(id).to_string(),
        background: String::new(),
        strength: stats.strength as f32,
        agility: stats.agility as f32,
        intelligence: stats.intelligence as f32,
        primary_attribute: primary_attribute_name(stats.primary_attribute).to_string(),
        attack_range: stats.attack_range.to_f32_for_render(),
        base_damage: stats.base_damage as f32,
        base_armor: stats.base_armor.to_f32_for_render(),
        base_hp: stats.base_hp as f32,
        base_mana: stats.base_mana as f32,
        move_speed: stats.move_speed.to_f32_for_render(),
        turn_speed: stats.turn_speed.to_f32_for_render(),
        abilities,
        level_growth: serde_json::json!({
            "strength_per_level": stats.level_growth.strength_per_level.to_f32_for_render(),
            "agility_per_level": stats.level_growth.agility_per_level.to_f32_for_render(),
            "intelligence_per_level": stats.level_growth.intelligence_per_level.to_f32_for_render(),
            "damage_per_level": stats.level_growth.damage_per_level.to_f32_for_render(),
            "hp_per_level": stats.level_growth.hp_per_level.to_f32_for_render(),
            "mana_per_level": stats.level_growth.mana_per_level.to_f32_for_render(),
        }),
    }
}

fn insert_creep(
    creeps: &mut BTreeMap<String, CreepInfo>,
    id: &str,
    abilities: Vec<String>,
) -> Result<()> {
    let cid = omoba_template_ids::creep_by_name(id)
        .with_context(|| format!("creep template missing: {id}"))?;
    let stats = omoba_template_ids::creep_stats(cid)
        .with_context(|| format!("creep template has no stats: {id}"))?;
    creeps.insert(id.to_string(), creep_info(cid, stats, abilities));
    Ok(())
}

fn creep_info(
    id: omoba_template_ids::CreepId,
    stats: omoba_template_ids::CreepStats,
    abilities: Vec<String>,
) -> CreepInfo {
    CreepInfo {
        name: omoba_template_ids::creep_display(id).to_string(),
        enemy_type: enemy_type_name(stats.enemy_type).to_string(),
        hp: stats.hp.to_f32_for_render(),
        armor: stats.armor.to_f32_for_render(),
        magic_resistance: stats.magic_resistance.to_f32_for_render(),
        damage: stats.damage.to_f32_for_render(),
        attack_range: stats.attack_range.to_f32_for_render(),
        move_speed: stats.move_speed.to_f32_for_render(),
        ai_type: ai_type_name(stats.ai_type).to_string(),
        abilities,
        exp_reward: stats.exp_reward,
        gold_reward: stats.gold_reward,
    }
}

fn primary_attribute_name(value: u8) -> &'static str {
    match value {
        0 => "strength",
        1 => "agility",
        2 => "intelligence",
        _ => "unknown",
    }
}

fn enemy_type_name(value: u8) -> &'static str {
    match value {
        0 => "caster",
        1 => "melee",
        2 => "ranged",
        3 => "boss",
        _ => "unknown",
    }
}

fn ai_type_name(value: u8) -> &'static str {
    match value {
        0 => "defensive",
        1 => "aggressive",
        2 => "patrol",
        3 => "guard",
        4 => "passive",
        5 => "berserker",
        _ => "unknown",
    }
}
