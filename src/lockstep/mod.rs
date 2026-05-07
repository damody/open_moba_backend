//! 鎖步線層（二期鷹架）。
//!
//! 與傳統 30Hz GameEvent 調度程式一起運作。第 2 階段僅處理
//! 輸入緩衝和 60Hz TickBatch 廣播；類比本身仍然存在
//! 在現有的 30Hz 管道上。第 3 階段將把 sim 執行移至此
//! 鎖步循環，第 4 階段將停用舊的 GameEvent 廣播。
//!
//! 該模組位於`#[cfg(feature = "kcp")]`後面，因為它依賴於
//! prost 產生的原型類型僅在 kcp 功能下建置。

pub mod input_buffer;
pub mod tick_broadcaster;
pub mod state;
pub mod state_hash_producer;
pub mod snapshot_producer;

pub use self::input_buffer::InputBuffer;
pub use self::tick_broadcaster::{TickBroadcaster, TickBroadcasterConfig};
pub use self::state::{LockstepState, PlayerSession, JoinRoleEnum};
pub use self::state_hash_producer::compute_state_hash;
pub use self::snapshot_producer::{serialize_snapshot, WorldSnapshot, EntitySnapshot, EntityKindTag, SCHEMA_VERSION as SNAPSHOT_SCHEMA_VERSION};

// 重新導出該模組使用的原型類型，以便呼叫者不需要
// 了解 prost 生成的路径。原型類型生活在
// `crate::transport::kcp_transport::game_proto` （參見 kcp_transport.rs:18）。
pub use crate::transport::kcp_transport::game_proto::{
    PlayerInput,
    InputSubmit,
    TickBatch,
    InputForPlayer,
    StateHash,
    JoinRequest,
    JoinRole,
    GameStart,
    SnapshotReq,
    SnapshotResp,
    SimSnapshot,
    ServerEvent,
    Vec2I,
    FixedI,
    AngleI,
    NoOp,
    MoveTo,
    AttackTarget,
    CastAbility,
    UpgradeAbility,
    TowerPlace,
    TowerUpgradeInput,
    TowerSell,
    ItemUse,
    PlayerJoinEvent,
    PlayerLeaveEvent,
    WaveStartEvent,
    GameEndEvent,
};

// PlayerInput oneof 內部枚舉由 prost 產生為
// `mod player_input { pub enum Action { NoOp(...), MoveTo(...), ... } }`。
// 重新匯出為「PlayerInputEnum」以實現符合人體工學的構造。
pub use crate::transport::kcp_transport::game_proto::player_input::Action as PlayerInputEnum;

// 同樣，ServerEvent oneof 內部枚舉。
pub use crate::transport::kcp_transport::game_proto::server_event::Event as ServerEventEnum;

/// 鎖步線框變體 - 由
/// kcp_transport 線程。第 2 階段引入了 4 種幀類型：
///
/// - `TickBatch`（標籤 0x11，S→C，廣播全部）
/// - `StateHash`（標籤 0x12，S→C，廣播全部）
/// - `GameStart`（標籤 0x14，S→C，單一客戶端）
/// - `SnapshotResp`（標籤 0x16，S→C，單一客戶端）
///
/// `OutboundMsg` 在新的 `Option<LockstepFrame>` 欄位中攜帶此資訊；何時
/// 目前，kcp 廣播線程（任務 2.3）發出對應的
/// 直接建立有效負載，繞過 GameEvent 信封。遺產
/// `OutboundMsg`（類型化/JSON）流未更改。
#[derive(Clone, Debug)]
pub enum LockstepFrame {
    /// 60Hz 廣播到每個連接的鎖步客戶端。
    TickBatch(TickBatch),
    /// 定期不同步探測－向所有人廣播。
    StateHash(StateHash),
    /// 每個客戶端對 JoinRequest 的回應。 `client_session_id` 匹配
    /// kcp 傳輸的會話映射金鑰（例如「kcp_<addr>」）。
    GameStart {
        client_session_id: String,
        msg: GameStart,
    },
    /// 每個客戶端快照回覆。
    SnapshotResp {
        client_session_id: String,
        msg: SnapshotResp,
    },
}
