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
    /// Game speed multiplier (debug only)。1 = real-time，2/4/8 = 快轉。
    /// 每個 real frame 跑 N 個 sub-tick，sim 推進 N × 固定 lockstep tick dt。
    /// Runtime 可由 stdin 指令 `:speed N` 動態切換（範圍 1..=16）。
    #[serde(default = "default_speed_mult")]
    pub SPEED_MULT: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Setting {
    server: ServerSetting,
    #[serde(default)]
    content: ContentSetting,
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

impl ServerSetting {
    pub fn validate(&self) -> Result<(), String> {
        LockstepTiming::new(self.STEP_FPS).map(|_| ())
    }

    pub fn lockstep_timing(&self) -> LockstepTiming {
        LockstepTiming::new(self.STEP_FPS)
            .expect("ServerSetting::validate should reject unsupported STEP_FPS")
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
}
