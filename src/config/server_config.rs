use lazy_static::lazy_static;
use omoba_core::lockstep_timing::{LockstepTiming, LOCKSTEP_TPS};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

fn default_story() -> String {
    "MVP_1".to_string()
}

fn default_speed_mult() -> u32 {
    1
}

fn default_step_fps() -> u32 {
    LOCKSTEP_TPS
}

fn default_false() -> bool {
    false
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MatchLockstepMode {
    Legacy,
    SecureV2OptIn,
    SecureV2Required,
}

impl Default for MatchLockstepMode {
    fn default() -> Self {
        Self::SecureV2Required
    }
}

/// Explicit opt-in; Story preserves the existing TD/campaign bootstrap.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MatchGameplayMode {
    #[default]
    Story,
    SingleLane,
    /// Compiled Lua training map; same secure MOBA input boundary.
    ThreeLane,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ServerSetting {
    pub SERVER_IP: String,
    pub SERVER_PORT: String,
    pub CLIENT_ID: String,
    pub PLAYER_NAME: String,
    pub MAP: String,
    pub MAX_PLAYER: i32,
    pub RENDER_DELAY_MS: u64,
    /// Server-authoritative simulation step FPS. Supported values: 120, 90, 60.
    #[serde(default = "default_step_fps")]
    pub STEP_FPS: u32,
    /// `scripts/lua_data/{STORY}` 資料夾名稱；預設 "MVP_1" 以保留既有行為。
    /// TD 模式設為 "TD_1" 以載入塔防關卡。
    #[serde(default = "default_story")]
    pub STORY: String,
    #[serde(default)]
    pub MATCH_GAMEPLAY_MODE: MatchGameplayMode,
    /// Match-scoped opt-in. Every player must agree before registration.
    #[serde(default)]
    pub MATCH_MANA_ENABLED: bool,
    /// Compiled Lua map selection for three-lane matches; absent keeps the legacy training map.
    #[serde(default)]
    pub MATCH_MAP_ID: Option<String>,
    /// Server-owned JSON exported from a Lua recipe, never a wire request.
    #[serde(default)]
    pub MATCH_ROLE_PLAN_JSON: Option<String>,
    /// Game speed multiplier (debug only)。1 = real-time，2/4/8 = 快轉。
    /// 每個 real frame 跑 N 個 sub-tick，sim 推進 N × 固定 lockstep tick dt。
    /// Runtime 可由 stdin 指令 `:speed N` 動態切換（範圍 1..=16）。
    #[serde(default = "default_speed_mult")]
    pub SPEED_MULT: u32,
    /// Match-level protocol mode. An active `secure_v2_required` match can
    /// never be downgraded; rollback is a pre-match configuration action.
    #[serde(default)]
    pub MATCH_LOCKSTEP_MODE: MatchLockstepMode,
    #[serde(default = "default_false")]
    pub SELECTIVE_LOCKSTEP_SHADOW: bool,
    #[serde(default = "default_false")]
    pub SELECTIVE_LOCKSTEP_DOGFOOD: bool,
    /// Server-owned authentication result: player ID -> team ID. This is
    /// configuration/bootstrap input and is never accepted from the wire.
    #[serde(default)]
    pub AUTHENTICATED_TEAM_BINDINGS: BTreeMap<u32, u32>,
    /// Server-owned pre-match selection; omitted players retain the legacy hero.
    #[serde(default)]
    pub AUTHENTICATED_HERO_BINDINGS: BTreeMap<u32, String>,
}

/// `[hero_knowledge]` section in `game.toml`.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct HeroKnowledgeSetting {
    /// 是否啟用英雄知識系統。預設 true。
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// CHIMPS 模式下是否禁用加成（仍照常發 KP）。預設 true。
    #[serde(default = "default_true")]
    pub chimps_disable: bool,
    /// 每局基礎 KP 獎勵。預設 3。
    #[serde(default = "default_base_kp")]
    pub base_kp_reward: u32,
    /// 勝利額外 KP 獎勵。預設 2。
    #[serde(default = "default_win_kp")]
    pub win_kp_bonus: u32,
}

fn default_true() -> bool {
    true
}
fn default_base_kp() -> u32 {
    3
}
fn default_win_kp() -> u32 {
    2
}

impl Default for HeroKnowledgeSetting {
    fn default() -> Self {
        Self {
            enabled: true,
            chimps_disable: true,
            base_kp_reward: 3,
            win_kp_bonus: 2,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Setting {
    server: ServerSetting,
    #[serde(default)]
    content: ContentSetting,
    #[serde(default)]
    hero_knowledge: HeroKnowledgeSetting,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ContentSetting {
    /// Directory containing native script DLLs. Relative paths are resolved
    /// relative to the game.toml file.
    pub SCRIPTS_DIR: Option<String>,
    /// Frontend/local-sim DLL path. Exported for omfx compatibility when set.
    pub DLL_PATH: Option<String>,
    /// Enable runtime Lua content loading.
    pub LUA_CONTENT: Option<bool>,
    /// Runtime Lua content root. Relative paths are resolved relative to game.toml.
    pub LUA_CONTENT_ROOT: Option<String>,
    /// Enable development hot reload for runtime Lua content.
    pub LUA_HOT_RELOAD: Option<bool>,
    /// Story data root used by local clients.
    pub STORY_DATA_DIR: Option<String>,
}

impl Default for ServerSetting {
    fn default() -> Self {
        let mut setting = read_setting().unwrap_or_else(|e| panic!("{e}"));
        if let Ok(story) = std::env::var("OMB_STORY") {
            if !story.trim().is_empty() {
                setting.server.STORY = story;
            }
        }
        setting.server.validate().unwrap_or_else(|e| panic!("{e}"));
        setting.server
    }
}

fn game_toml_path() -> PathBuf {
    // omobab.exe 通常使用 cwd=omb 執行，因此相對路徑可找到 `game.toml`。
    // 其他 runtime caller 可能使用不同 cwd；OMB_GAME_TOML 讓呼叫者提供
    // 正確的絕對路徑。
    PathBuf::from(std::env::var("OMB_GAME_TOML").unwrap_or_else(|_| "game.toml".to_string()))
}

fn read_setting() -> Result<Setting, String> {
    let file_path = game_toml_path();
    let mut file = File::open(&file_path)
        .map_err(|e| format!("no such file {} exception:{}", file_path.display(), e))?;
    let mut str_val = String::new();
    file.read_to_string(&mut str_val)
        .map_err(|e| format!("Error Reading ApplicationConfig: {}", e))?;
    toml::from_str(&str_val).map_err(|e| format!("Error Parsing ApplicationConfig: {}", e))
}

/// Read-only launch preflight, using exactly the production config decoder.
#[cfg(feature = "kcp")]
pub fn validate_moba_launch_configuration(text:&str) -> Result<serde_json::Value,String> {
    let setting:Setting=toml::from_str(text).map_err(|error|format!("invalid launch TOML: {error}"))?;
    let server=setting.server;
    server.validate()?;
    let config=server.single_lane_config()?.ok_or("launch configuration must be MOBA")?;
    let bots=server.role_bot_config()?.ok_or("launch configuration requires a role plan")?;
    Ok(serde_json::json!({"schema_version":1,"scope":"configuration-only",
        "tick_rate_hz":server.STEP_FPS,"story":server.STORY,"map_id":config.map_id,
        "base_recovery_enabled":config.base_recovery_enabled,
        "mana_enabled":config.mana_enabled,
        "player_count":config.additional_players.len()+2,
        "humans":server.AUTHENTICATED_TEAM_BINDINGS.iter().map(|(&player_id,&team_id)|
            serde_json::json!({"player_id":player_id,"team_id":team_id})).collect::<Vec<_>>(),
        "bot_player_ids":bots.assignments.iter().map(|bot|bot.player_id).collect::<Vec<_>>()
    }))
}

fn resolve_config_path(base_file: &Path, value: &str) -> String {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        return path.to_string_lossy().into_owned();
    }
    base_file
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(path)
        .to_string_lossy()
        .into_owned()
}

fn set_env_if_missing(name: &str, value: String) {
    let should_set = std::env::var(name)
        .map(|v| v.trim().is_empty())
        .unwrap_or(true);
    if should_set {
        std::env::set_var(name, value);
    }
}

fn set_bool_env_if_missing(name: &str, value: Option<bool>) {
    if let Some(value) = value {
        set_env_if_missing(name, if value { "1" } else { "0" }.to_string());
    }
}

/// Apply runtime content settings from game.toml to the legacy env vars used by
/// omoba-template-ids and shared script loading code. Explicit environment
/// values still win, so ad-hoc overrides remain possible.
pub fn apply_runtime_env_from_game_toml() {
    let file_path = game_toml_path();
    let setting = match read_setting() {
        Ok(setting) => setting,
        Err(err) => {
            log::warn!("failed to read runtime content config: {}", err);
            return;
        }
    };
    let content = setting.content;
    if let Some(value) = content.SCRIPTS_DIR {
        set_env_if_missing("OMB_SCRIPTS_DIR", resolve_config_path(&file_path, &value));
    }
    if let Some(value) = content.DLL_PATH {
        set_env_if_missing("OMB_DLL_PATH", resolve_config_path(&file_path, &value));
    }
    if let Some(value) = content.LUA_CONTENT_ROOT {
        set_env_if_missing(
            "OMB_LUA_CONTENT_ROOT",
            resolve_config_path(&file_path, &value),
        );
    }
    if let Some(value) = content.STORY_DATA_DIR {
        set_env_if_missing(
            "OMB_STORY_DATA_DIR",
            resolve_config_path(&file_path, &value),
        );
    }
    set_bool_env_if_missing("OMB_LUA_CONTENT", content.LUA_CONTENT);
    set_bool_env_if_missing("OMB_LUA_HOT_RELOAD", content.LUA_HOT_RELOAD);
}

/// 讀取 `game.toml` 的 `[hero_knowledge]` section。
/// 讀取失敗時回傳 default。
pub fn read_hero_knowledge_setting() -> HeroKnowledgeSetting {
    match read_setting() {
        Ok(s) => s.hero_knowledge,
        Err(e) => {
            log::warn!(
                "failed to read hero_knowledge config: {}; using defaults",
                e
            );
            HeroKnowledgeSetting::default()
        }
    }
}

impl ServerSetting {
    pub fn validate(&self) -> Result<(), String> {
        LockstepTiming::new(self.STEP_FPS).map(|_| ())?;
        if self.SELECTIVE_LOCKSTEP_DOGFOOD && self.AUTHENTICATED_TEAM_BINDINGS.is_empty() {
            return Err("dogfood secure V2 requires authenticated team bindings".into());
        }
        self.single_lane_config()?;
        Ok(())
    }

    pub fn single_lane_config(&self) -> Result<Option<omoba_core::runtime::SingleLaneConfig>, String> {
        if self.MATCH_MANA_ENABLED && (self.MATCH_GAMEPLAY_MODE == MatchGameplayMode::Story
            || self.MATCH_LOCKSTEP_MODE != MatchLockstepMode::SecureV2Required) {
            return Err("MATCH_MANA_ENABLED requires secure_v2_required MOBA gameplay".into());
        }
        if self.MATCH_ROLE_PLAN_JSON.is_some() {
            #[cfg(feature = "kcp")]
            return self.compiled_role_match().map(|value|value.map(|(config,_)|config));
            #[cfg(not(feature = "kcp"))]
            return Err("role bots require KCP secure MOBA gameplay".into());
        }
        if self.MATCH_MAP_ID.is_some() && self.MATCH_GAMEPLAY_MODE != MatchGameplayMode::ThreeLane {
            return Err("MATCH_MAP_ID requires three_lane gameplay".into());
        }
        if self.MATCH_GAMEPLAY_MODE == MatchGameplayMode::Story {
            return Ok(None);
        }
        if self.MATCH_LOCKSTEP_MODE != MatchLockstepMode::SecureV2Required {
            return Err("single_lane requires secure_v2_required (no legacy gameplay input)".into());
        }
        for (player, hero) in &self.AUTHENTICATED_HERO_BINDINGS {
            if !self.AUTHENTICATED_TEAM_BINDINGS.contains_key(player)
                || omoba_template_ids::hero_by_name(hero).and_then(omoba_template_ids::hero_stats).is_none() {
                return Err("single_lane hero binding requires a roster player and active compiled hero".into());
            }
        }
        let selected = |player| self.AUTHENTICATED_HERO_BINDINGS.get(&player)
            .cloned().unwrap_or_else(|| "training_luminary".into());
        let mut players = [0; 2];
        let mut additional_players = Vec::new();
        let mut counts = [0; 2];
        for (player, team) in &self.AUTHENTICATED_TEAM_BINDINGS {
            let side = match *team {
                1 => 0,
                2 => 1,
                _ => return Err("single_lane requires authenticated teams 1 and 2".into()),
            };
            if *player == 0 || counts[side] >= 5 {
                return Err("single_lane requires one to five nonzero players per team".into());
            }
            counts[side] += 1;
            if players[side] == 0 { players[side] = *player; }
            else {
                additional_players.push(omoba_core::runtime::SingleLanePlayerConfig {
                    player_id: *player, team_id: *team, hero: selected(*player),
                });
            }
        }
        if players.contains(&0) {
            return Err("single_lane requires one to five nonzero players per team".into());
        }
        let map = if self.MATCH_GAMEPLAY_MODE == MatchGameplayMode::ThreeLane {
            let id = self.MATCH_MAP_ID.as_deref().unwrap_or("three_lane_training");
            Some(omoba_template_ids::moba_map_by_name(id)
                .ok_or_else(|| format!("unknown compiled MOBA map '{id}'"))?)
        } else { None };
        Ok(Some(omoba_core::runtime::SingleLaneConfig {
            mana_enabled: self.MATCH_MANA_ENABLED,
            map_id: map.map(|map| map.id.to_owned()),
            lane_length: map.map_or(omoba_sim::Fixed64::from_i32(2400), |map| omoba_sim::Fixed64::from_i32(map.lane_length)),
            players,
            heroes: players.map(selected),
            additional_players,
            seed: omoba_core::runtime::MasterSeed::default().0,
            ..Default::default()
        }))
    }

    pub fn lockstep_timing(&self) -> LockstepTiming {
        LockstepTiming::new(self.STEP_FPS)
            .expect("ServerSetting::validate should reject unsupported STEP_FPS")
    }

    #[cfg(feature = "kcp")]
    fn compiled_role_match(&self) -> Result<Option<(omoba_core::runtime::SingleLaneConfig,
        omoba_core::runtime::native::moba_match::bots::RoleBotConfig)>,String> {
        let Some(json)=self.MATCH_ROLE_PLAN_JSON.as_deref() else {return Ok(None);};
        LockstepTiming::new(self.STEP_FPS)?;
        if self.MATCH_GAMEPLAY_MODE!=MatchGameplayMode::ThreeLane
            || self.MATCH_LOCKSTEP_MODE!=MatchLockstepMode::SecureV2Required {
            return Err("role plan requires three_lane and secure_v2_required".into());
        }
        if json.len()>64*1024 {return Err("role plan JSON exceeds 64 KiB".into());}
        if !self.AUTHENTICATED_HERO_BINDINGS.is_empty() {
            return Err("role plan owns hero selection; do not mix hero bindings".into());
        }
        let plan:omoba_core::runtime::native::moba_match::bots::RoleBotMatchPlan=
            serde_json::from_str(json).map_err(|error|format!("invalid role plan JSON: {error}"))?;
        if self.MATCH_MAP_ID.as_deref().is_some_and(|id|id!=plan.map_id) {
            return Err("role plan map conflicts with MATCH_MAP_ID".into());
        }
        if plan.mana_enabled != self.MATCH_MANA_ENABLED {
            return Err("role plan mana_enabled conflicts with MATCH_MANA_ENABLED".into());
        }
        let human_teams:BTreeMap<_,_>=plan.players.iter().filter(|p|!p.bot)
            .map(|p|(p.player_id,p.team_id)).collect();
        if human_teams!=self.AUTHENTICATED_TEAM_BINDINGS {
            return Err("authenticated teams must match exactly the role plan's human controllers (never bots)".into());
        }
        plan.compile_with_tick_rate(omoba_core::runtime::MasterSeed::default().0,self.STEP_FPS)
            .map(|(mut config, bots)| { config.mana_enabled = self.MATCH_MANA_ENABLED; Some((config,bots)) })
    }

    #[cfg(feature = "kcp")]
    pub fn role_bot_config(&self) -> Result<Option<omoba_core::runtime::native::moba_match::bots::RoleBotConfig>,String> {
        self.compiled_role_match().map(|value|value.map(|(_,bots)|bots))
    }

    pub fn secure_v2_required(&self) -> bool {
        self.SELECTIVE_LOCKSTEP_DOGFOOD
            || self.MATCH_LOCKSTEP_MODE == MatchLockstepMode::SecureV2Required
    }

    pub fn selective_generation_enabled(&self) -> bool {
        self.SELECTIVE_LOCKSTEP_SHADOW
            || self.SELECTIVE_LOCKSTEP_DOGFOOD
            || self.MATCH_LOCKSTEP_MODE != MatchLockstepMode::Legacy
    }
}
/*
impl ServerSetting {
    pub fn sql_url(&self) -> String {
        let s = format!(
            "mysql://{}:{}@{}:{}/{}",
            self.MYSQL_ACCOUNT.clone(),
            self.MYSQL_PASSWORD.clone(),
            self.SQL_IP.clone(),
            self.SQL_PORT.clone(),
            self.MYSQL_DB.clone()
        );
        s
    }
    pub fn sql_log_url(&self) -> String {
        let s = format!(
            "mysql://{}:{}@{}:{}/{}",
            self.MYSQL_ACCOUNT.clone(),
            self.MYSQL_PASSWORD.clone(),
            self.SQL_IP.clone(),
            self.SQL_PORT.clone(),
            self.MYSQL_DB_LOG.clone()
        );
        s
    }
}
*/
lazy_static! {
    pub static ref CONFIG: ServerSetting = ServerSetting::default();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mana_agreement_configuration_is_explicit_and_requires_secure_moba() {
        let mut setting = parse_with_step_fps(60);
        assert!(!setting.MATCH_MANA_ENABLED);
        setting.MATCH_MANA_ENABLED = true;
        assert!(setting.single_lane_config().is_err());
        setting.MATCH_GAMEPLAY_MODE = MatchGameplayMode::SingleLane;
        setting.AUTHENTICATED_TEAM_BINDINGS = BTreeMap::from([(1,1),(2,2)]);
        assert!(setting.single_lane_config().unwrap().unwrap().mana_enabled);
        setting.MATCH_LOCKSTEP_MODE = MatchLockstepMode::Legacy;
        assert!(setting.single_lane_config().is_err());
    }

    fn parse_with_step_fps(step_fps: u32) -> ServerSetting {
        let raw = format!(
            r#"
[server]
MAP = "map.json"
MAX_PLAYER = 10000
SERVER_IP = "localhost"
SERVER_PORT = "50061"
CLIENT_ID = "omobab"
PLAYER_NAME = "player1"
RENDER_DELAY_MS = 100
STEP_FPS = {step_fps}
"#
        );
        toml::from_str::<Setting>(&raw).unwrap().server
    }

    #[test]
    #[cfg(feature = "kcp")]
    fn role_server_plan_keeps_nine_bots_out_of_human_authorization() {
        use omoba_core::runtime::native::moba_match::bots::*;
        let mut players=Vec::new();
        for team in 1..=2 {
            for (index,(role,lane)) in [(BotRole::Top,"top"),(BotRole::Mid,"mid"),(BotRole::Carry,"bottom"),
                (BotRole::Support,"bottom"),(BotRole::Jungle,"mid")].into_iter().enumerate() {
                let player_id=(team-1)*5+index as u32+1;
                players.push(RoleBotPlayerPlan {player_id,team_id:team,hero:"training_luminary".into(),role,lane:lane.into(),bot:player_id!=1});
            }
        }
        let plan=RoleBotMatchPlan {schema_version:1,map_id:"three_lane_training".into(),think_hz:5,mana_enabled:false,players,
            ability_policies:Vec::new(),ability_learning:Vec::new(),sustain:None,item_builds:Vec::new()};
        let mut setting=parse_with_step_fps(60);
        setting.MATCH_GAMEPLAY_MODE=MatchGameplayMode::ThreeLane;
        setting.MATCH_ROLE_PLAN_JSON=Some(serde_json::to_string(&plan).unwrap());
        setting.AUTHENTICATED_TEAM_BINDINGS=BTreeMap::from([(1,1)]);
        setting.validate().unwrap();
        let lane=setting.single_lane_config().unwrap().unwrap();
        assert_eq!(lane.players,[1,6]);assert_eq!(lane.additional_players.len(),8);
        assert_eq!(setting.role_bot_config().unwrap().unwrap().assignments.len(),9);
        let mut auth=crate::lockstep::LockstepState::new(lane.seed);
        for (&player,&team) in &setting.AUTHENTICATED_TEAM_BINDINGS {auth.authorize_player_team(player,team).unwrap();}
        let negotiation=omoba_core::transport::MatchCapabilityNegotiation {
            requested_protocol:omoba_core::transport::SELECTIVE_LOCKSTEP_PROTOCOL_VERSION,supported_protocols:vec![omoba_core::transport::SELECTIVE_LOCKSTEP_PROTOCOL_VERSION],secure_fog_required:true,
        };
        assert!(auth.register_secure_player(1,"human".into(),crate::lockstep::JoinRoleEnum::Player,negotiation.clone(),1).is_ok());
        for player in 2..=10 {
            assert!(auth.register_secure_player(player,"bot spoof".into(),crate::lockstep::JoinRoleEnum::Player,negotiation.clone(),1)
                .unwrap_err().contains("no authenticated team binding"));
        }
        assert_eq!(setting.AUTHENTICATED_TEAM_BINDINGS.len(),1);
        for change in 0..7 {
            let mut bad=setting.clone();
            match change {
                0=>{bad.AUTHENTICATED_TEAM_BINDINGS.insert(2,1);},
                1=>{bad.AUTHENTICATED_TEAM_BINDINGS.insert(1,2);},
                2=>{bad.AUTHENTICATED_TEAM_BINDINGS.clear();},
                3=>{bad.MATCH_GAMEPLAY_MODE=MatchGameplayMode::Story;},
                4=>{bad.MATCH_LOCKSTEP_MODE=MatchLockstepMode::SecureV2OptIn;},
                5=>{bad.AUTHENTICATED_HERO_BINDINGS.insert(1,"training_luminary".into());},
                _=>{bad.MATCH_MAP_ID=Some("three_lane_layered_training".into());},
            }
            assert!(bad.validate().is_err());
        }
        setting.STEP_FPS=90;
        assert_eq!(setting.role_bot_config().unwrap().unwrap().think_interval_ticks,18);
        setting.MATCH_ROLE_PLAN_JSON=Some("{".into());assert!(setting.validate().is_err());
        setting.MATCH_ROLE_PLAN_JSON=Some(" ".repeat(64*1024+1));assert!(setting.validate().is_err());
    }

    #[test]
    #[cfg(feature = "kcp")]
    fn mana_sustain_server_requires_recipe_and_rule_flag_agreement() {
        let mut setting=parse_with_step_fps(60);
        setting.MATCH_GAMEPLAY_MODE=MatchGameplayMode::ThreeLane;
        setting.AUTHENTICATED_TEAM_BINDINGS=BTreeMap::from([(1,1),(2,2)]);
        let mut plan=serde_json::json!({"schema_version":1,"map_id":"three_lane_training","think_hz":5,"mana_enabled":true,
            "players":[{"player_id":1,"team_id":1,"hero":"training_ranger","role":"carry","lane":"bottom","bot":false},
                {"player_id":2,"team_id":2,"hero":"training_luminary","role":"mid","lane":"mid","bot":false}],
            "sustain":{"recall_below_hp_per_mille":350,"leave_base_at_hp_per_mille":850,"threat_radius":1000,
                "mana":{"recall_below_per_mille":200,"leave_base_at_per_mille":850}}});
        setting.MATCH_ROLE_PLAN_JSON=Some(plan.to_string());
        assert!(setting.role_bot_config().unwrap_err().contains("mana_enabled conflicts"));
        setting.MATCH_MANA_ENABLED=true;
        setting.validate().unwrap();
        let config=setting.single_lane_config().unwrap().unwrap();
        assert!(config.mana_enabled && config.base_recovery_enabled);
        plan["mana_enabled"]=serde_json::json!(false);
        setting.MATCH_ROLE_PLAN_JSON=Some(plan.to_string());
        assert!(setting.role_bot_config().unwrap_err().contains("mana_enabled conflicts"));
    }

    #[test]
    #[cfg(feature = "kcp")]
    fn role_server_lua_fragment_round_trips_through_real_toml_configuration() {
        let root=Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        for (recipe,bots,humans) in [("moba_role_match.lua",10,BTreeMap::new()),
            ("moba_single_player.lua",9,BTreeMap::from([(1,1)]))] {
        let output=root.join("omb/target").join(format!("role-server-{}.toml",uuid::Uuid::new_v4()));
        let status=std::process::Command::new(root.join("tools/lua/lua.exe"))
            .arg(root.join("scripts/export_moba_role_plan.lua"))
            .arg(root.join("scripts/lua_data").join(recipe))
            .arg(&output).arg("--server-fragment").status().unwrap();
        assert!(status.success());
        let fragment=std::fs::read_to_string(&output).unwrap();
        std::fs::remove_file(&output).unwrap();
        let mut server=toml::Value::try_from(parse_with_step_fps(60)).unwrap();
        let fields:toml::Value=toml::from_str(&fragment).unwrap();
        server.as_table_mut().unwrap().extend(fields.as_table().unwrap().clone());
        let setting:ServerSetting=toml::from_str(&toml::to_string(&server).unwrap()).unwrap();
        setting.validate().unwrap();
        assert_eq!(setting.AUTHENTICATED_TEAM_BINDINGS,humans);
        assert_eq!(setting.role_bot_config().unwrap().unwrap().assignments.len(),bots);
        assert_eq!(setting.role_bot_config().unwrap().unwrap().ability_learning.len(),16);
        }
    }

    #[test]
    fn single_lane_hero_selection_is_server_owned_validated_and_player_scoped() {
        let mut setting = parse_with_step_fps(60);
        setting.MATCH_GAMEPLAY_MODE = MatchGameplayMode::SingleLane;
        setting.AUTHENTICATED_TEAM_BINDINGS = BTreeMap::from([(1,1),(2,2),(3,1)]);
        setting.AUTHENTICATED_HERO_BINDINGS = BTreeMap::from([(1,"training_apprentice".into()),(3,"date_masamune".into())]);
        let config = setting.single_lane_config().unwrap().unwrap();
        assert_eq!(config.heroes,["training_apprentice","training_luminary"]);
        assert_eq!(config.additional_players[0].hero,"date_masamune");
        for (player,hero) in [(4,"training_apprentice"),(2,"missing_hero")] {
            setting.AUTHENTICATED_HERO_BINDINGS.insert(player,hero.into());
            assert!(setting.single_lane_config().is_err());
            setting.AUTHENTICATED_HERO_BINDINGS.remove(&player);
        }
    }

    #[test]
    fn story_is_default_and_single_lane_uses_authenticated_roster_and_shared_seed() {
        let mut setting = parse_with_step_fps(120);
        assert_eq!(setting.MATCH_GAMEPLAY_MODE, MatchGameplayMode::Story);
        assert!(setting.single_lane_config().unwrap().is_none());
        setting.MATCH_GAMEPLAY_MODE = MatchGameplayMode::SingleLane;
        setting.AUTHENTICATED_TEAM_BINDINGS = BTreeMap::from([(21, 2), (35, 1)]);
        let lane = setting.single_lane_config().unwrap().unwrap();
        assert_eq!(lane.players, [35, 21]);
        assert_eq!(lane.teams, [1, 2]);
        assert_eq!(lane.seed, omoba_core::runtime::MasterSeed::default().0);
        assert!(setting.validate().is_ok());
    }

    #[test]
    fn single_lane_rejects_legacy_and_ambiguous_rosters() {
        let mut setting = parse_with_step_fps(120);
        setting.MATCH_GAMEPLAY_MODE = MatchGameplayMode::SingleLane;
        for roster in [BTreeMap::new(), BTreeMap::from([(0, 1), (2, 2)]),
            BTreeMap::from([(1, 1), (2, 1)]), BTreeMap::from([(1, 1), (2, 3)]),
            BTreeMap::from([(1, 1), (2, 2), (3, 2), (4, 2), (5, 2), (6, 2), (7, 2)])] {
            setting.AUTHENTICATED_TEAM_BINDINGS = roster;
            assert!(setting.validate().is_err());
        }
        setting.AUTHENTICATED_TEAM_BINDINGS = BTreeMap::from([(1, 1), (2, 2)]);
        for mode in [MatchLockstepMode::Legacy, MatchLockstepMode::SecureV2OptIn] {
            setting.MATCH_LOCKSTEP_MODE = mode;
            assert!(setting.validate().unwrap_err().contains("secure_v2_required"));
        }
    }

    #[test]
    fn single_lane_supports_deterministic_five_player_teams() {
        let mut setting = parse_with_step_fps(60);
        setting.MATCH_GAMEPLAY_MODE = MatchGameplayMode::SingleLane;
        setting.AUTHENTICATED_TEAM_BINDINGS = (1..=10).map(|player| (player, if player % 2 == 1 { 1 } else { 2 })).collect();
        assert!(setting.validate().is_ok());
        let lane = setting.single_lane_config().unwrap().unwrap();
        assert_eq!(lane.players, [1, 2]);
        assert_eq!(lane.additional_players.iter().map(|p| (p.player_id, p.team_id)).collect::<Vec<_>>(),
            vec![(3,1),(4,2),(5,1),(6,2),(7,1),(8,2),(9,1),(10,2)]);
    }

    #[test]
    fn three_lane_is_explicit_and_retains_secure_roster_requirements() {
        let mut setting = parse_with_step_fps(60);
        setting.MATCH_GAMEPLAY_MODE = MatchGameplayMode::ThreeLane;
        setting.AUTHENTICATED_TEAM_BINDINGS = BTreeMap::from([(1,1),(2,2)]);
        assert!(setting.validate().is_ok());
        let config = setting.single_lane_config().unwrap().unwrap();
        assert_eq!(config.map_id.as_deref(),Some("three_lane_training"));
        assert_eq!(config.players,[1,2]);
        setting.MATCH_LOCKSTEP_MODE = MatchLockstepMode::Legacy;
        assert!(setting.validate().unwrap_err().contains("secure_v2_required"));
        setting.MATCH_LOCKSTEP_MODE = MatchLockstepMode::SecureV2Required;
        setting.AUTHENTICATED_TEAM_BINDINGS = BTreeMap::from([(1,1)]);
        assert!(setting.validate().is_err());
        setting.MATCH_GAMEPLAY_MODE = MatchGameplayMode::SingleLane;
        setting.AUTHENTICATED_TEAM_BINDINGS = BTreeMap::from([(1,1),(2,2)]);
        assert!(setting.single_lane_config().unwrap().unwrap().map_id.is_none());
    }

    #[test]
    fn three_lane_compiled_map_selection_is_validated_not_name_branched() {
        let mut setting = parse_with_step_fps(60);
        setting.MATCH_GAMEPLAY_MODE = MatchGameplayMode::ThreeLane;
        setting.AUTHENTICATED_TEAM_BINDINGS = BTreeMap::from([(1,1),(2,2)]);
        setting.MATCH_MAP_ID = Some("three_lane_layered_training".into());
        assert!(setting.validate().is_ok());
        let config = setting.single_lane_config().unwrap().unwrap();
        assert_eq!(config.map_id.as_deref(),Some("three_lane_layered_training"));
        assert_eq!(config.lane_length,omoba_sim::Fixed64::from_i32(2400));
        for id in ["", "unknown_map", "three_lane_training "] {
            setting.MATCH_MAP_ID = Some(id.into());
            assert!(setting.validate().unwrap_err().contains("unknown compiled MOBA map"));
        }
        setting.MATCH_MAP_ID = Some("three_lane_layered_training".into());
        for mode in [MatchGameplayMode::Story,MatchGameplayMode::SingleLane] {
            setting.MATCH_GAMEPLAY_MODE = mode;
            assert!(setting.validate().unwrap_err().contains("requires three_lane"));
        }
    }

    #[test]
    fn accepts_supported_step_fps_values() {
        for fps in [120, 90, 60] {
            let setting = parse_with_step_fps(fps);
            assert!(setting.validate().is_ok(), "fps={fps}");
            assert_eq!(setting.lockstep_timing().step_fps(), fps);
        }
    }

    #[test]
    fn rejects_unsupported_step_fps_values() {
        let setting = parse_with_step_fps(144);
        let err = setting.validate().unwrap_err();
        assert!(err.contains("unsupported STEP_FPS=144"));
    }

    #[test]
    fn missing_step_fps_defaults_to_lockstep_tps() {
        let raw = r#"
[server]
MAP = "map.json"
MAX_PLAYER = 10000
SERVER_IP = "localhost"
SERVER_PORT = "50061"
CLIENT_ID = "omobab"
PLAYER_NAME = "player1"
RENDER_DELAY_MS = 100
"#;
        let setting = toml::from_str::<Setting>(raw).unwrap().server;
        assert_eq!(setting.STEP_FPS, LOCKSTEP_TPS);
        assert!(setting.validate().is_ok());
    }

    #[test]
    fn secure_v2_is_default_but_explicit_legacy_remains_pre_match_option() {
        let secure = parse_with_step_fps(120);
        assert_eq!(
            secure.MATCH_LOCKSTEP_MODE,
            MatchLockstepMode::SecureV2Required
        );
        let raw = r#"
[server]
MAP="map.json"
MAX_PLAYER=1
SERVER_IP="localhost"
SERVER_PORT="50061"
CLIENT_ID="omb"
PLAYER_NAME="p"
RENDER_DELAY_MS=1
MATCH_LOCKSTEP_MODE="legacy"
"#;
        let legacy = toml::from_str::<Setting>(raw).unwrap().server;
        assert_eq!(legacy.MATCH_LOCKSTEP_MODE, MatchLockstepMode::Legacy);
        assert!(!legacy.secure_v2_required());
    }
}
