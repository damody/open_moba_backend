use crossbeam_channel::{Sender, Receiver};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::SystemTime;
#[cfg(feature = "kcp")]
use std::sync::Arc;
#[cfg(feature = "kcp")]
use super::metrics::KcpBytesCounter;

/// Outbound message from game logic to transport layer.
/// Replaces `MqttMsg` in game logic code.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OutboundMsg {
    pub topic: String,
    pub msg: String,
    pub time: SystemTime,
    /// Entity position in game coordinates, for viewport filtering.
    /// None = global event (heartbeat, death, etc.) that bypasses filtering.
    #[serde(skip)]
    pub entity_pos: Option<(f32, f32)>,
}

impl OutboundMsg {
    pub fn new(topic: &String, t: &String, a: &String, v: serde_json::Value) -> OutboundMsg {
        #[derive(Serialize, Deserialize)]
        struct ResData {
            t: String,
            a: String,
            d: serde_json::Value,
        }
        let res = ResData {
            t: t.clone(),
            a: a.clone(),
            d: v,
        };
        OutboundMsg {
            topic: topic.to_owned(),
            msg: json!(res).to_string(),
            time: SystemTime::now(),
            entity_pos: None,
        }
    }

    pub fn new_s<'a>(topic: &'a str, t: &'a str, a: &'a str, v: serde_json::Value) -> OutboundMsg {
        #[derive(Serialize, Deserialize)]
        struct ResData {
            t: String,
            a: String,
            d: serde_json::Value,
        }
        let res = ResData {
            t: t.to_owned(),
            a: a.to_owned(),
            d: v,
        };
        OutboundMsg {
            topic: topic.to_owned(),
            msg: json!(res).to_string(),
            time: SystemTime::now(),
            entity_pos: None,
        }
    }

    /// Create an OutboundMsg with entity position for viewport filtering.
    pub fn new_s_at(topic: &str, t: &str, a: &str, v: serde_json::Value, x: f32, y: f32) -> OutboundMsg {
        #[derive(Serialize, Deserialize)]
        struct ResData {
            t: String,
            a: String,
            d: serde_json::Value,
        }
        let res = ResData {
            t: t.to_owned(),
            a: a.to_owned(),
            d: v,
        };
        OutboundMsg {
            topic: topic.to_owned(),
            msg: json!(res).to_string(),
            time: SystemTime::now(),
            entity_pos: Some((x, y)),
        }
    }
}

impl Default for OutboundMsg {
    fn default() -> OutboundMsg {
        OutboundMsg {
            topic: "".to_owned(),
            msg: "".to_owned(),
            time: SystemTime::now(),
            entity_pos: None,
        }
    }
}

/// Inbound message from transport layer to game logic.
/// Replaces `PlayerData` in game logic code.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct InboundMsg {
    pub name: String,
    pub t: String,
    pub a: String,
    pub d: serde_json::Value,
}

/// Query request from MCP server to game loop.
#[cfg(any(feature = "grpc", feature = "kcp"))]
pub struct QueryRequest {
    pub query_type: String,
    pub player_name: String,
    pub response_tx: tokio::sync::oneshot::Sender<QueryResponse>,
}

/// Query response from game loop back to gRPC/KCP handler.
#[cfg(any(feature = "grpc", feature = "kcp"))]
pub struct QueryResponse {
    pub success: bool,
    pub error: String,
    pub data_json: Vec<u8>,
}

/// Client viewport rectangle (padded) used for spatial filtering and visibility diffs.
#[cfg(any(feature = "grpc", feature = "kcp"))]
#[derive(Copy, Clone, Debug)]
pub struct Viewport {
    pub cx: f32,
    pub cy: f32,
    pub padded_hw: f32,
    pub padded_hh: f32,
}

#[cfg(any(feature = "grpc", feature = "kcp"))]
impl Viewport {
    pub fn new(cx: f32, cy: f32, hw: f32, hh: f32) -> Self {
        Self { cx, cy, padded_hw: hw * 1.3, padded_hh: hh * 1.3 }
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        (x - self.cx).abs() <= self.padded_hw && (y - self.cy).abs() <= self.padded_hh
    }
}

/// Viewport lifecycle messages sent from transport to game loop.
#[cfg(any(feature = "grpc", feature = "kcp"))]
#[derive(Debug, Clone)]
pub enum ViewportMsg {
    Set { player_name: String, viewport: Viewport },
    Remove { player_name: String },
}

/// Handle returned by transport layer initialization.
pub struct TransportHandle {
    pub tx: Sender<OutboundMsg>,
    pub rx: Receiver<InboundMsg>,
    #[cfg(any(feature = "grpc", feature = "kcp"))]
    pub query_rx: Receiver<QueryRequest>,
    #[cfg(any(feature = "grpc", feature = "kcp"))]
    pub viewport_rx: Receiver<ViewportMsg>,
    /// Per-event byte/msg counters observed on the KCP wire.
    /// Shared with the broadcast thread so the game loop / tests can call
    /// `.snapshot()` or `.reset()` concurrently.
    #[cfg(feature = "kcp")]
    pub counter: Arc<KcpBytesCounter>,
}
