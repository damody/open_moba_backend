use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::time::Duration;

fn default_story() -> String {
    "MVP_1".to_string()
}

fn default_speed_mult() -> u32 { 1 }

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ServerSetting {
    pub SERVER_IP: String,
    pub SERVER_PORT: String,
    pub CLIENT_ID: String,
    pub PLAYER_NAME: String,
    pub MAP: String,
    pub MAX_PLAYER: i32,
    pub RENDER_DELAY_MS: u64,
    /// `Story/{STORY}` 資料夾名稱；預設 "MVP_1" 以保留既有行為。
    /// TD 模式設為 "TD_1" 以載入塔防關卡。
    #[serde(default = "default_story")]
    pub STORY: String,
    /// Game speed multiplier (debug only)。1 = real-time，2/4/8 = 快轉。
    /// 每個 real frame 跑 N 個 sub-tick，sim 推進 N × frame-time。
    /// Runtime 可由 stdin 指令 `:speed N` 動態切換（範圍 1..=16）。
    #[serde(default = "default_speed_mult")]
    pub SPEED_MULT: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Setting {
    server: ServerSetting,
}

impl Default for ServerSetting {
    fn default() -> Self {
        // omobab.exe runs with cwd=omb so finds "game.toml" by relative path.
        // omfx sim_runner runs in omfx process cwd, where the relative path
        // misses; OMB_GAME_TOML env var lets the caller point at the right
        // absolute path (omfx sets it to D:/omoba/omb/game.toml).
        let file_path = std::env::var("OMB_GAME_TOML")
            .unwrap_or_else(|_| "game.toml".to_string());
        let mut file = match File::open(&file_path) {
            Ok(f) => f,
            Err(e) => panic!("no such file {} exception:{}", file_path, e),
        };
        let mut str_val = String::new();
        match file.read_to_string(&mut str_val) {
            Ok(s) => s,
            Err(e) => panic!("Error Reading ApplicationConfig: {}", e),
        };
        let setting: Setting = toml::from_str(&str_val).unwrap();
        setting.server
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
