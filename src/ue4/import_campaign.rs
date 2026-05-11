pub use omoba_core::runtime::scene::import_campaign::*;

use crate::json_preprocessor::JsonPreprocessor;

/// 從 active template content 載入戰役資料。預設為 generated Rust data；
/// `runtime-lua-content` feature + `OMB_LUA_CONTENT=1` 時改讀 Lua-loaded snapshot。
pub fn load_generated(story_id: &str) -> Result<CampaignData, Box<dyn std::error::Error>> {
    omoba_template_ids::ensure_runtime_lua_content().map_err(|e| {
        Box::new(std::io::Error::new(std::io::ErrorKind::Other, e)) as Box<dyn std::error::Error>
    })?;
    let story = omoba_template_ids::active_story_by_name(story_id)
        .ok_or_else(|| format!("unknown active story '{}'", story_id))?;
    CampaignData::from_generated_story(story)
}

/// 僅用於移轉工具的舊版 JSON 載入器。運行時應使用 `load_generated`。
pub fn load_from_path(campaign_path: &str) -> Result<CampaignData, Box<dyn std::error::Error>> {
    let entity_path = format!("{}/entity.json", campaign_path);
    let ability_path = format!("{}/ability.json", campaign_path);
    let mission_path = format!("{}/mission.json", campaign_path);
    let map_path = format!("{}/map.json", campaign_path);

    let entity: EntityData = JsonPreprocessor::read_json_with_comments(&entity_path)?;
    let ability: AbilityData = JsonPreprocessor::read_json_with_comments(&ability_path)?;
    let mission: MissionData = JsonPreprocessor::read_json_with_comments(&mission_path)?;
    let map: super::import_map::CreepWaveData =
        JsonPreprocessor::read_json_with_comments(&map_path)?;

    Ok(CampaignData {
        entity,
        ability,
        mission,
        map,
    })
}
