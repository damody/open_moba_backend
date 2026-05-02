use crossbeam_channel::{Sender, Receiver};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::SystemTime;
#[cfg(feature = "kcp")]
use std::sync::Arc;
#[cfg(feature = "kcp")]
use super::metrics::KcpBytesCounter;

/// P2 binary-protocol migration: typed prost payload carried alongside the
/// legacy JSON `msg` string. When `OutboundMsg.typed` is `Some(_)` the KCP
/// broadcast thread builds `GameEvent.typed_payload` directly and leaves
/// `data_json` empty — so the wire carries ONLY the prost variant.
///
/// The JSON `msg` field is retained for dedupe/router introspection in the
/// broadcast thread. It does NOT go on the wire in the typed path.
///
/// Available only under `kcp` because the prost types live in
/// `kcp_transport::game_proto`.
#[cfg(feature = "kcp")]
#[derive(Clone, Debug)]
pub enum TypedOutbound {
    Heartbeat(super::kcp_transport::game_proto::HeartbeatTick),
    // P2 full migration: typed variants for the high-volume events.
    ProjectileCreate(super::kcp_transport::game_proto::ProjectileCreate),
    ProjectileDestroy(super::kcp_transport::game_proto::ProjectileDestroy),
    CreepCreate(super::kcp_transport::game_proto::CreepCreate),
    CreepMove(super::kcp_transport::game_proto::CreepMove),
    CreepHp(super::kcp_transport::game_proto::CreepHp),
    CreepSlow(super::kcp_transport::game_proto::CreepSlow),
    CreepStall(super::kcp_transport::game_proto::CreepStall),
    EntityFacing(super::kcp_transport::game_proto::EntityFacing),
    EntityDeath(super::kcp_transport::game_proto::EntityDeath),
    TowerCreate(super::kcp_transport::game_proto::TowerCreate),
    TowerUpgrade(super::kcp_transport::game_proto::TowerUpgrade),
    BuffAdd(super::kcp_transport::game_proto::BuffAdd),
    BuffRemove(super::kcp_transport::game_proto::BuffRemove),
    HeroStatic(super::kcp_transport::game_proto::HeroStatic),
    HeroHot(super::kcp_transport::game_proto::HeroHot),
    HeroCreate(super::kcp_transport::game_proto::HeroCreate),     // P9
    UnitCreate(super::kcp_transport::game_proto::UnitCreate),     // P9
    GameRound(super::kcp_transport::game_proto::GameRound),
    GameLives(super::kcp_transport::game_proto::GameLives),
    GameEnd(super::kcp_transport::game_proto::GameEnd),
    GameExplosion(super::kcp_transport::game_proto::GameExplosion),
    LegacyJson(super::kcp_transport::game_proto::LegacyJson),     // P9 catch-all
}

/// P5 broadcast policy — declares who should receive this event. The
/// transport's broadcast thread uses this to select target sessions BEFORE
/// cloning the encoded frame via `Arc<[u8]>`. When `policy` is `None`, legacy
/// topic-based routing applies (back-compat for un-migrated emit sites).
#[cfg(any(feature = "grpc", feature = "kcp"))]
#[derive(Clone, Debug)]
pub enum BroadcastPolicy {
    /// Reaches every connected player. Use for game-wide state
    /// (round/lives/end/tower_templates/map_data).
    All,
    /// Filtered to players whose viewport contains (x, y).
    /// The big bucket: creep events, projectile events, entity.F, tower events.
    AoiPoint(f32, f32),
    /// Same as AoiPoint but the coordinates come from looking up `entity_id`'s
    /// current Pos via the AoiGrid registry. Use when caller has id but not pos
    /// cheaply at hand (e.g. hero.stats hot tick).
    AoiEntity(u64),
    /// Single target — player-specific events like hero.inventory,
    /// creep visibility diffs (current `td/{player}/res` topics).
    PlayerOnly(String),
}

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
    /// P2 binary migration: when Some, the transport emits GameEvent with
    /// `typed_payload` set and `data_json` left empty. `msg` still carries a
    /// JSON copy for dedupe / router introspection (kept in-memory only).
    #[cfg(feature = "kcp")]
    #[serde(skip)]
    pub typed: Option<TypedOutbound>,
    /// P5 explicit broadcast policy. `None` = legacy topic-based routing.
    /// When Some, the broadcast thread ignores `topic`-based heuristics and
    /// targets sessions according to the policy variant.
    #[cfg(any(feature = "grpc", feature = "kcp"))]
    #[serde(skip)]
    pub policy: Option<BroadcastPolicy>,
    /// Phase 2 lockstep wire frame. When `Some`, the kcp transport's
    /// broadcast thread emits the corresponding lockstep tag (0x11 / 0x12 /
    /// 0x14 / 0x16) directly, bypassing the GameEvent envelope. When `None`,
    /// the legacy `typed` / JSON path is used.
    ///
    /// The other `OutboundMsg` fields (`topic`, `msg`, `typed`, `policy`)
    /// are ignored when `lockstep_frame` is set; the lockstep frame carries
    /// its own routing (broadcast All for Tick/Hash, per-client for
    /// GameStart/SnapshotResp).
    #[cfg(feature = "kcp")]
    #[serde(skip)]
    pub lockstep_frame: Option<crate::lockstep::LockstepFrame>,
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
            #[cfg(feature = "kcp")]
            typed: None,
            #[cfg(any(feature = "grpc", feature = "kcp"))]
            policy: None,
            #[cfg(feature = "kcp")]
            lockstep_frame: None,
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
            #[cfg(feature = "kcp")]
            typed: None,
            #[cfg(any(feature = "grpc", feature = "kcp"))]
            policy: None,
            #[cfg(feature = "kcp")]
            lockstep_frame: None,
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
            #[cfg(feature = "kcp")]
            typed: None,
            // P5: entity_pos sites default to AoiPoint so legacy callers opt-in
            // to AOI filtering without per-site migration. They can still
            // override via `.with_policy(...)` for All / PlayerOnly / AoiEntity.
            #[cfg(any(feature = "grpc", feature = "kcp"))]
            policy: Some(BroadcastPolicy::AoiPoint(x, y)),
            #[cfg(feature = "kcp")]
            lockstep_frame: None,
        }
    }

    /// P2 binary migration constructor. `typed` is a pre-built prost message;
    /// `json_fallback` is the legacy `d` field used to build the `msg` string
    /// for dedupe / router introspection (the JSON form does NOT go on the
    /// wire when `typed` is Some — only the prost variant is emitted).
    #[cfg(feature = "kcp")]
    pub fn new_typed(
        topic: &str,
        t: &str,
        a: &str,
        typed: TypedOutbound,
        json_fallback: serde_json::Value,
    ) -> OutboundMsg {
        OutboundMsg {
            topic: topic.to_owned(),
            msg: json!({ "t": t, "a": a, "d": json_fallback }).to_string(),
            time: SystemTime::now(),
            entity_pos: None,
            typed: Some(typed),
            policy: None,
            lockstep_frame: None,
        }
    }

    /// Same as `new_typed` but carries an entity position for viewport filtering.
    #[cfg(feature = "kcp")]
    pub fn new_typed_at(
        topic: &str,
        t: &str,
        a: &str,
        typed: TypedOutbound,
        json_fallback: serde_json::Value,
        x: f32,
        y: f32,
    ) -> OutboundMsg {
        OutboundMsg {
            topic: topic.to_owned(),
            msg: json!({ "t": t, "a": a, "d": json_fallback }).to_string(),
            time: SystemTime::now(),
            entity_pos: Some((x, y)),
            typed: Some(typed),
            // P5: default to AoiPoint so the broadcast thread filters by
            // viewport without per-site migration. Callers who need All /
            // AoiEntity / PlayerOnly override via `.with_policy(...)`.
            policy: Some(BroadcastPolicy::AoiPoint(x, y)),
            lockstep_frame: None,
        }
    }

    // ===== P5 policy helpers =====

    /// Attach a `BroadcastPolicy` to this message (builder style).
    #[cfg(any(feature = "grpc", feature = "kcp"))]
    pub fn with_policy(mut self, policy: BroadcastPolicy) -> Self {
        self.policy = Some(policy);
        self
    }

    /// Shortcut: build a `new_typed` msg with `BroadcastPolicy::All`.
    /// Use for game-wide events (round/lives/end/tower_templates/map_data).
    #[cfg(feature = "kcp")]
    pub fn new_typed_all(
        topic: &str,
        t: &str,
        a: &str,
        typed: TypedOutbound,
        json_fallback: serde_json::Value,
    ) -> OutboundMsg {
        OutboundMsg {
            topic: topic.to_owned(),
            msg: json!({ "t": t, "a": a, "d": json_fallback }).to_string(),
            time: SystemTime::now(),
            entity_pos: None,
            typed: Some(typed),
            policy: Some(BroadcastPolicy::All),
            lockstep_frame: None,
        }
    }

    /// Shortcut: build a `new_typed` msg with `BroadcastPolicy::AoiEntity(id)`.
    /// Use when the emit site holds an `entity_id` but not its current position
    /// cheaply at hand (e.g. hero.hot tick, entity death, HP updates without pos).
    #[cfg(feature = "kcp")]
    pub fn new_typed_aoi_entity(
        topic: &str,
        t: &str,
        a: &str,
        typed: TypedOutbound,
        json_fallback: serde_json::Value,
        entity_id: u64,
    ) -> OutboundMsg {
        OutboundMsg {
            topic: topic.to_owned(),
            msg: json!({ "t": t, "a": a, "d": json_fallback }).to_string(),
            time: SystemTime::now(),
            entity_pos: None,
            typed: Some(typed),
            policy: Some(BroadcastPolicy::AoiEntity(entity_id)),
            lockstep_frame: None,
        }
    }

    /// Phase 2 lockstep: build an OutboundMsg carrying a lockstep wire frame.
    /// The kcp transport's broadcast thread reads `lockstep_frame` and emits
    /// the appropriate tag (0x11 / 0x12 / 0x14 / 0x16) to either all sessions
    /// (TickBatch / StateHash) or one session (GameStart / SnapshotResp).
    /// All other `OutboundMsg` fields are ignored on this path.
    #[cfg(feature = "kcp")]
    pub fn lockstep_frame(frame: crate::lockstep::LockstepFrame) -> OutboundMsg {
        OutboundMsg {
            topic: String::new(),
            msg: String::new(),
            time: SystemTime::now(),
            entity_pos: None,
            typed: None,
            policy: Some(BroadcastPolicy::All),
            lockstep_frame: Some(frame),
        }
    }

    /// Shortcut: JSON-only (non-kcp) `All` policy. For non-kcp builds we still
    /// want to tag game-wide events so the grpc broadcast thread sees them.
    #[cfg(any(feature = "grpc", feature = "kcp"))]
    pub fn new_s_all(topic: &str, t: &str, a: &str, v: serde_json::Value) -> OutboundMsg {
        let mut m = OutboundMsg::new_s(topic, t, a, v);
        m.policy = Some(BroadcastPolicy::All);
        m
    }
}

impl Default for OutboundMsg {
    fn default() -> OutboundMsg {
        OutboundMsg {
            topic: "".to_owned(),
            msg: "".to_owned(),
            time: SystemTime::now(),
            entity_pos: None,
            #[cfg(feature = "kcp")]
            typed: None,
            #[cfg(any(feature = "grpc", feature = "kcp"))]
            policy: None,
            #[cfg(feature = "kcp")]
            lockstep_frame: None,
        }
    }
}

/// P6 two-tier batch window: classifies outbound events by latency budget.
///
/// `Urgent` events flush immediately (sub-10ms UX budget). `Normal` events
/// benefit from batching + dedupe within a 10~33ms window.
#[cfg(any(feature = "grpc", feature = "kcp"))]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Urgency {
    /// Flush ASAP. Death/skill cast/explosion/spawn/projectile/buff/game state.
    Urgent,
    /// Batch up to 33ms. HP/facing/move ticks, heartbeats, everything else.
    Normal,
}

/// P6: classify `(msg_type, action)` into an `Urgency` class. Mirrors the shape
/// of `is_dedupable` — centralised in one place so both policies evolve
/// together.
///
/// Rationale per class:
/// - `*.D` / `.death` — death feedback: hero needs to see immediately.
/// - `*.C` / `.create` — spawn: positional/visual pops need to be immediate.
/// - `projectile.*` — all projectile events (create, destroy) are visual and
///   brief; delaying them breaks hit/miss perception.
/// - `game.explosion/.end/.round` — game-state transitions, non-batchable.
/// - `creep.stall` — one-shot collision feedback.
/// - `tower.create/.upgrade` — rare but UX-critical.
/// - `buff.*` — add/remove: players must see buffs appear/disappear crisply.
///
/// Everything else (creep.M/.H/.S, entity.F, hero.hot, heartbeat.tick) falls
/// through to `Normal` and enjoys the dedupe window.
#[cfg(any(feature = "grpc", feature = "kcp"))]
pub fn urgency(msg_type: &str, action: &str) -> Urgency {
    // Action-level: any destroy / creation is urgent regardless of kind.
    match action {
        "D" | "death" | "C" | "create" => return Urgency::Urgent,
        _ => {}
    }
    match (msg_type, action) {
        ("game", "explosion" | "end" | "round") => Urgency::Urgent,
        ("projectile", _) => Urgency::Urgent,
        ("tower", "upgrade") => Urgency::Urgent,
        ("creep", "stall") => Urgency::Urgent,
        ("buff", _) => Urgency::Urgent,
        _ => Urgency::Normal,
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
    /// P5: shared AOI broadphase grid. Game loop calls `.lock().rebuild(..)`
    /// per tick from the same pre-gather that heartbeat uses; broadcast thread
    /// calls `.lock().lookup_pos(id)` to resolve `BroadcastPolicy::AoiEntity`.
    /// Mutex contention is minimal — both hold the lock for microseconds.
    #[cfg(feature = "kcp")]
    pub aoi: Arc<std::sync::Mutex<crate::aoi::AoiGrid>>,
}

#[cfg(test)]
#[cfg(any(feature = "grpc", feature = "kcp"))]
mod urgency_tests {
    use super::{urgency, Urgency};

    #[test]
    fn death_is_urgent_any_kind() {
        assert_eq!(urgency("creep", "D"), Urgency::Urgent);
        assert_eq!(urgency("hero", "D"), Urgency::Urgent);
        assert_eq!(urgency("tower", "D"), Urgency::Urgent);
        assert_eq!(urgency("entity", "death"), Urgency::Urgent);
    }

    #[test]
    fn create_is_urgent_any_kind() {
        assert_eq!(urgency("creep", "C"), Urgency::Urgent);
        assert_eq!(urgency("tower", "create"), Urgency::Urgent);
    }

    #[test]
    fn projectile_all_urgent() {
        // Every projectile event — create, destroy, and any future variant
        // — is urgent regardless of action (visual sub-10ms budget).
        assert_eq!(urgency("projectile", "C"), Urgency::Urgent);
        assert_eq!(urgency("projectile", "D"), Urgency::Urgent);
        assert_eq!(urgency("projectile", "hit"), Urgency::Urgent);
    }

    #[test]
    fn game_state_transitions_urgent() {
        assert_eq!(urgency("game", "explosion"), Urgency::Urgent);
        assert_eq!(urgency("game", "end"), Urgency::Urgent);
        assert_eq!(urgency("game", "round"), Urgency::Urgent);
    }

    #[test]
    fn buff_events_urgent() {
        assert_eq!(urgency("buff", "add"), Urgency::Urgent);
        assert_eq!(urgency("buff", "remove"), Urgency::Urgent);
        assert_eq!(urgency("buff", "buff_add"), Urgency::Urgent);
    }

    #[test]
    fn tower_upgrade_urgent() {
        assert_eq!(urgency("tower", "upgrade"), Urgency::Urgent);
    }

    #[test]
    fn creep_stall_urgent() {
        assert_eq!(urgency("creep", "stall"), Urgency::Urgent);
    }

    #[test]
    fn hot_path_normal() {
        // The bandwidth-heavy streams MUST be Normal so they benefit from
        // the 10~33ms dedupe window. If any of these regresses to Urgent we
        // lose the dedupe savings P1 paid for.
        assert_eq!(urgency("creep", "M"), Urgency::Normal);
        assert_eq!(urgency("creep", "H"), Urgency::Normal);
        assert_eq!(urgency("creep", "S"), Urgency::Normal);
        assert_eq!(urgency("entity", "F"), Urgency::Normal);
        assert_eq!(urgency("hero", "hot"), Urgency::Normal);
        assert_eq!(urgency("heartbeat", "tick"), Urgency::Normal);
    }

    #[test]
    fn unknown_defaults_to_normal() {
        // Forward-compat: a new (msg_type, action) we haven't categorised
        // falls through to Normal. Safer than Urgent (which would defeat
        // batching for events we didn't mean to flush immediately).
        assert_eq!(urgency("weird", "thing"), Urgency::Normal);
        assert_eq!(urgency("", ""), Urgency::Normal);
    }
}
