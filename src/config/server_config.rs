use lazy_static::lazy_static;
use serde_derive::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::time::Duration;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ServerSetting {
    pub SERVER_IP: String,
    pub SERVER_PORT: String,
    pub CLIENT_ID: String,
    pub MAP: String,
    pub MAX_PLAYER: i32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Setting {
    server: ServerSetting,
}

impl Default for ServerSetting {
    fn default() -> Self {
        let file_path = "game.toml";
        let mut file = match File::open(file_path) {
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
