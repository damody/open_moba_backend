//! 將軍知識（General Knowledge）系統。
//!
//! - `loader`：讀取 `scripts/lua_data/knowledge_tree.json`，驗證節點。
//! - `player_profile`：KP 累積、節點解鎖、持久化 JSON profile。
//! - `kp_reward`：對局結束 KP 發放。
//!
//! Host 端 ECS resource `KnowledgeBonusResource` 定義在 `omoba-core`，
//! 供 `game_processor.rs` 在塔生成時套入加成。

pub mod kp_reward;
pub mod loader;
pub mod player_profile;

pub use kp_reward::KpRewardConfig;
pub use loader::{build_bonus_map, load_knowledge_tree};
pub use player_profile::{load_profile, save_profile, unlock_node, PlayerProfile};
