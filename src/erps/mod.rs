//! Optional standalone-ERPS game-server adapter. No matchmaking state lives in `omb`.
mod client;
mod instance_adapter;
pub use client::{ErpsGameServerClient, ErpsServerConfig};
pub use instance_adapter::{
    GameInstanceAdapter, InstanceReport, LaunchAssignment, LaunchReady, MatchCompletion,
};
