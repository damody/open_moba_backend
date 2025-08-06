/// Open MOBA Backend Library
/// 
/// Main library crate for the Open MOBA game backend

pub mod comp;
pub mod vision;
pub mod config;
pub mod mqtt;
pub mod msg;
pub mod tick;
pub mod ue4;
pub mod util;
pub mod json_preprocessor;
pub mod state;

// Re-export commonly used types
pub use crate::comp::*;
pub use crate::vision::*;
pub use crate::msg::MqttMsg;

// Define needed types for state.rs
#[derive(Debug)]
pub struct PlayerData {
    pub a: String,
    pub d: serde_json::Value,
    pub name: String,
    pub t: String,
}