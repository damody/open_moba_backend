use crossbeam_channel::{bounded, Sender, Receiver};
use failure::Error;
use log::*;
use std::collections::HashMap;
use std::sync::Arc;
use std::thread;
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::sync::Mutex;

use tokio_kcp::{KcpConfig, KcpListener, KcpNoDelayConfig, KcpStream};

use super::types::{BroadcastPolicy, InboundMsg, OutboundMsg, TransportHandle, QueryRequest, QueryResponse, TypedOutbound, Viewport, ViewportMsg, Urgency, urgency};
use std::sync::atomic::{AtomicU64, Ordering};
use super::metrics::KcpBytesCounter;
use crate::aoi::AoiGrid;

// Include the generated proto code
pub mod game_proto {
    include!(concat!(env!("OUT_DIR"), "/game.rs"));
}

use game_proto::*;
use prost::Message;

// Framing tag constants (same protocol as omoba-core).
// KEEP IN SYNC with omoba-core::kcp::framing — frame format MUST match byte-for-byte.
const TAG_PLAYER_COMMAND: u8 = 0x01;
const TAG_GAME_EVENT: u8 = 0x02;
const TAG_COMMAND_ACK: u8 = 0x03;
const TAG_SUBSCRIBE_REQUEST: u8 = 0x04;
const TAG_GAME_STATE_REQUEST: u8 = 0x05;
const TAG_GAME_STATE_RESPONSE: u8 = 0x06;
const TAG_VIEWPORT_UPDATE: u8 = 0x07;

// Phase 2 lockstep tags.
const TAG_INPUT_SUBMIT: u8 = 0x10;
const TAG_TICK_BATCH: u8 = 0x11;
const TAG_STATE_HASH: u8 = 0x12;
const TAG_JOIN_REQUEST: u8 = 0x13;
const TAG_GAME_START: u8 = 0x14;
const TAG_SNAPSHOT_REQ: u8 = 0x15;
const TAG_SNAPSHOT_RESP: u8 = 0x16;
const TAG_PING_REQ: u8 = 0x17;
const TAG_PING_RESP: u8 = 0x18;

/// High bit of the tag — set when the framed payload is LZ4-compressed.
/// Base tags 0x01~0x07 never use this bit so it is always free as a flag.
const COMPRESSION_FLAG: u8 = 0x80;

/// Minimum payload size before we bother trying LZ4 compression.
const LZ4_THRESHOLD: usize = 128;

/// Write a framed message: [1 byte tag][4 bytes len (big-endian)][N bytes payload]
/// When payload ≥ LZ4_THRESHOLD and LZ4 shrinks it, the payload is replaced with
/// a size-prepended LZ4 block and COMPRESSION_FLAG is OR'd into the tag.
/// KEEP IN SYNC with omoba-core::kcp::framing::write_framed.
async fn write_framed<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    tag: u8,
    payload: &[u8],
) -> std::io::Result<()> {
    debug_assert!(tag & COMPRESSION_FLAG == 0, "base tag must not use high bit; got 0x{:02x}", tag);
    let (out_tag, out_payload): (u8, &[u8]);
    let compressed_holder;
    if payload.len() >= LZ4_THRESHOLD {
        let c = lz4_flex::block::compress_prepend_size(payload);
        if c.len() < payload.len() {
            out_tag = tag | COMPRESSION_FLAG;
            compressed_holder = c;
            out_payload = &compressed_holder;
        } else {
            out_tag = tag;
            out_payload = payload;
        }
    } else {
        out_tag = tag;
        out_payload = payload;
    }
    let len = out_payload.len() as u32;
    writer.write_u8(out_tag).await?;
    writer.write_u32(len).await?;
    writer.write_all(out_payload).await?;
    writer.flush().await?;
    Ok(())
}

/// Read a framed message, returns (tag, decompressed_payload, wire_bytes).
/// If COMPRESSION_FLAG is set on the wire tag, the payload is decompressed and
/// the returned tag has the flag stripped (callers see only 0x01~0x07).
/// `wire_bytes` = 1 (tag) + 4 (length) + N (raw on-wire bytes).
/// KEEP IN SYNC with omoba-core::kcp::framing::read_framed.
async fn read_framed<R: AsyncReadExt + Unpin>(
    reader: &mut R,
) -> std::io::Result<Option<(u8, Vec<u8>, usize)>> {
    let tag_raw = match reader.read_u8().await {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    };
    let len = reader.read_u32().await? as usize;
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    let wire_bytes = 1 + 4 + len;
    if tag_raw & COMPRESSION_FLAG != 0 {
        let base_tag = tag_raw & 0x7F;
        let decompressed = lz4_flex::block::decompress_size_prepended(&buf)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Ok(Some((base_tag, decompressed, wire_bytes)))
    } else {
        Ok(Some((tag_raw, buf, wire_bytes)))
    }
}

/// Build a wire-framed byte buffer matching `write_framed`'s on-wire layout
/// (`[1B tag][4B len BE][N bytes payload]`) with optional LZ4 compression
/// when the payload is ≥ LZ4_THRESHOLD AND compresses smaller. Used by the
/// broadcast thread to assemble lockstep frames once and Arc-share them
/// across all recipient sessions.
fn build_framed_bytes(tag: u8, payload: &[u8]) -> Vec<u8> {
    debug_assert!(tag & COMPRESSION_FLAG == 0, "base tag must not use high bit; got 0x{:02x}", tag);
    let (out_tag, out_payload): (u8, std::borrow::Cow<'_, [u8]>) = if payload.len() >= LZ4_THRESHOLD {
        let c = lz4_flex::block::compress_prepend_size(payload);
        if c.len() < payload.len() {
            (tag | COMPRESSION_FLAG, std::borrow::Cow::Owned(c))
        } else {
            (tag, std::borrow::Cow::Borrowed(payload))
        }
    } else {
        (tag, std::borrow::Cow::Borrowed(payload))
    };
    let mut frame = Vec::with_capacity(1 + 4 + out_payload.len());
    frame.push(out_tag);
    frame.extend_from_slice(&(out_payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(&out_payload);
    frame
}

/// Per-client session: holds a sender to push outbound events.
///
/// P5: channel payload is `Arc<[u8]>` — the broadcast thread encodes + compresses
/// each frame ONCE and then hands out cheap `Arc::clone` references to every
/// target session. No per-session encode, no per-session payload copy.
struct ClientSession {
    player_name: String,
    event_tx: tokio::sync::mpsc::Sender<Arc<[u8]>>,
    viewport: Option<Viewport>,
    /// P6: per-session monotonic sequence counter. Incremented + stamped on
    /// every GameEvent the broadcast thread dispatches to this session. The
    /// client compares against its last-known sequence and requests a
    /// snapshot via TAG_GAME_STATE_REQUEST (query_type="seq-gap") when a gap
    /// is observed.
    ///
    /// Wrapped in Arc so the broadcast thread can stamp + encode the frame
    /// per-session without holding the sessions mutex during encode.
    seq: Arc<AtomicU64>,
    /// Phase 2 lockstep: set true once this session sent a JoinRequest (0x13).
    /// TickBatch (0x11) and StateHash (0x12) broadcasts only fan out to
    /// sessions with this flag — clients on the legacy GameEvent path
    /// (omb-mcp, omfx during Phase 2 transition) won't see lockstep traffic.
    lockstep_joined: bool,
}

/// Pure-function policy dispatch used by both the broadcast thread and unit
/// tests. Returns the list of session IDs that should receive the frame.
///
/// `sessions` is a borrow into the live session map; `aoi_lookup` is a callback
/// the broadcast thread wires to `AoiGrid::lookup_pos` (tests can stub it).
/// This lets us unit-test the dispatch rules without spinning up KCP / tokio.
#[cfg(test)]
fn select_targets_for_policy(
    policy: Option<&BroadcastPolicy>,
    topic: &str,
    entity_pos: Option<(f32, f32)>,
    sessions: &std::collections::BTreeMap<String, (String, Option<Viewport>)>,
    aoi_lookup: &dyn Fn(u64) -> Option<(f32, f32)>,
) -> Vec<String> {
    match policy {
        Some(BroadcastPolicy::All) => sessions.keys().cloned().collect(),
        Some(BroadcastPolicy::PlayerOnly(name)) => sessions.iter()
            .filter(|(_, (player_name, _))| player_name == name)
            .map(|(id, _)| id.clone())
            .collect(),
        Some(BroadcastPolicy::AoiPoint(x, y)) => sessions.iter()
            .filter(|(_, (_, vp))| match vp {
                Some(v) => v.contains(*x, *y),
                None => true,
            })
            .map(|(id, _)| id.clone())
            .collect(),
        Some(BroadcastPolicy::AoiEntity(eid)) => match aoi_lookup(*eid) {
            Some((x, y)) => sessions.iter()
                .filter(|(_, (_, vp))| match vp {
                    Some(v) => v.contains(x, y),
                    None => true,
                })
                .map(|(id, _)| id.clone())
                .collect(),
            None => sessions.keys().cloned().collect(),
        },
        None => {
            let is_broadcast = topic.contains("/all/");
            sessions.iter()
                .filter(|(_, (player_name, vp))| {
                    let topic_ok = is_broadcast
                        || topic.contains(&format!("/{}/", player_name))
                        || player_name.is_empty();
                    if !topic_ok { return false; }
                    match (entity_pos, vp) {
                        (Some((x, y)), Some(v)) => v.contains(x, y),
                        _ => true,
                    }
                })
                .map(|(id, _)| id.clone())
                .collect()
        }
    }
}

// ===== Batch-window dedupe =====
// Within a single 33ms batch window, multiple messages for the same
// (msg_type, action, entity_id) collapse to the latest-value-wins. This trims
// redundant HP / movement / stats updates before wire encoding.
//
// NOTE: `peek_kind_and_id` parses JSON a second time (the encode loop below
// also parses). This duplication is intentional for clarity — P2 cleanup
// target: proto oneof / strongly-typed msgs will eliminate both parses.

#[derive(Hash, Eq, PartialEq, Debug)]
struct DedupeKey {
    msg_type: String,
    action: String,
    entity_id: u64,
}

/// Returns true if (msg_type, action) is a latest-wins kind safe to dedupe.
///
/// Included: movement/facing/HP/slow/stats updates where only the latest value
/// matters within a 33ms window.
///
/// Excluded (handled by default pass-through): creation/destroy events
/// (`*.C` / `*.D` / `*.death`), buffs, game-state events, tower upgrades,
/// heartbeats — each of these is independently meaningful and must arrive.
///
/// Note on emitted kinds:
/// - `F` (facing) is always emitted with msg_type="entity" (for creep/hero/tower)
/// - `H` (HP) is emitted with dynamic msg_type based on the hit unit
///   ("hero" / "creep" / "unit" / "entity"); we cover all variants.
fn is_dedupable(msg_type: &str, action: &str) -> bool {
    matches!(
        (msg_type, action),
        ("creep", "M")
            | ("creep", "H") | ("hero", "H") | ("unit", "H") | ("entity", "H")
            | ("entity", "F")
            | ("creep", "S")
            // P3: hero.stats 拆成兩條 prost 事件，仍為 latest-wins（hot 每 0.3s 推一次；
            // static 只在 level up / ability learn 觸發，多個在 33ms 內可 dedupe）。
            | ("hero", "hot")
            | ("hero", "static")
    )
}

/// Collapse dedupable messages by (msg_type, action, entity_id), keeping latest.
/// Non-dedupable messages pass through in original order. Dedupable messages
/// keep their FIRST occurrence's slot, with its value overwritten by the LATEST
/// payload. Unknown / malformed JSON → pass through.
fn dedupe_batch(batch: Vec<OutboundMsg>) -> Vec<OutboundMsg> {
    let mut out: Vec<OutboundMsg> = Vec::with_capacity(batch.len());
    let mut dedupe_idx: hashbrown::HashMap<DedupeKey, usize> = hashbrown::HashMap::new();
    for msg in batch {
        let (t, a, id) = peek_kind_and_id(&msg.msg);
        match (id, is_dedupable(&t, &a)) {
            (Some(entity_id), true) => {
                let key = DedupeKey { msg_type: t, action: a, entity_id };
                match dedupe_idx.get(&key) {
                    Some(&idx) => {
                        // Replace in place so post-dedupe order stays
                        // deterministic (first-occurrence slot, latest payload).
                        out[idx] = msg;
                    }
                    None => {
                        dedupe_idx.insert(key, out.len());
                        out.push(msg);
                    }
                }
            }
            // Not dedupable, or id field missing/malformed → pass-through.
            // Note: id=0 is a *legal* specs::Entity index, so we only skip
            // dedupe when the id field itself is absent (parse returned None),
            // not on the value 0.
            _ => out.push(msg),
        }
    }
    out
}

fn outbound_urgency(msg: &OutboundMsg) -> Urgency {
    if msg.lockstep_frame.is_some() {
        return Urgency::Urgent;
    }
    let (t, a, _) = peek_kind_and_id(&msg.msg);
    urgency(&t, &a)
}

/// Extract (msg_type, action, Option<entity_id>) from an OutboundMsg JSON
/// payload. `entity_id = None` means either parse failure or `d.id` absent;
/// caller treats that as non-dedupable. A present `d.id = 0` is a legal
/// `specs::Entity` index and is returned as `Some(0)`.
fn peek_kind_and_id(payload: &str) -> (String, String, Option<u64>) {
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(payload) else {
        return (String::new(), String::new(), None);
    };
    let t = parsed.get("t").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let a = parsed.get("a").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let id = parsed.get("d").and_then(|d| d.get("id")).and_then(|v| v.as_u64());
    (t, a, id)
}

/// Start the KCP transport layer.
///
/// Phase 2 lockstep: callers pass shared `Arc<Mutex<InputBuffer>>` and
/// `Arc<Mutex<LockstepState>>` so the per-client read loop can:
///   - push 0x10 InputSubmit payloads into the buffer at the right tick;
///   - register players + reply with 0x14 GameStart on 0x13 JoinRequest;
///   - reply with 0x16 SnapshotResp on 0x15 SnapshotReq.
///
/// Phase 5.3 adds `lockstep_snapshot_store`: the dispatcher tick loop in
/// `state::core::tick()` mirrors a fresh `WorldSnapshot` into this Arc every
/// `SNAPSHOT_INTERVAL_TICKS` (= 30 s @ 30 Hz). The 0x15 SnapshotReq handler
/// clones the latest bytes out and returns them as 0x16 SnapshotResp to the
/// requesting observer client. Empty bytes (`tick=0`) are valid — the
/// observer falls back to playing from the current tick without bootstrap.
pub async fn start(
    server_addr: String,
    server_port: String,
    lockstep_input_buffer: Arc<std::sync::Mutex<crate::lockstep::InputBuffer>>,
    lockstep_state: Arc<std::sync::Mutex<crate::lockstep::LockstepState>>,
    lockstep_snapshot_store: Arc<std::sync::Mutex<crate::comp::SnapshotStore>>,
) -> Result<TransportHandle, Error> {
    // Phase 5.x backpressure fix: under TD_STRESS the host tick systems still
    // emit legacy per-entity events (creep.M / creep.H / entity.F / projectile.C
    // — Phase 5 design wants replica clients to compute these locally but the
    // producers haven't all been cut yet). Combined with TickBroadcaster's
    // 60Hz lockstep frames, peak rate is ~1000 msg/sec. The old `bounded(10000)`
    // saturated in ~10s and `out_tx.send` (blocking) deadlocked the broadcaster
    // task — clients then saw zero TickBatches and sim_runner blocked on its
    // input recv. 100k buffers ~100s of headroom. Real fix is to drop the
    // legacy event broadcasts entirely (Phase 5 scope) and/or split lockstep
    // and game-event channels so a slow drain on one doesn't stall the other.
    let (out_tx, out_rx): (Sender<OutboundMsg>, Receiver<OutboundMsg>) = bounded(100_000);
    let (in_tx, in_rx): (Sender<InboundMsg>, Receiver<InboundMsg>) = bounded(10000);
    let (query_tx, query_rx): (Sender<QueryRequest>, Receiver<QueryRequest>) = bounded(100);
    let (viewport_tx, viewport_rx): (Sender<ViewportMsg>, Receiver<ViewportMsg>) = bounded(1024);

    let sessions: Arc<Mutex<HashMap<String, ClientSession>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // Per-event bytes/msg counter. Shared with the broadcast thread so tests
    // and the game loop can snapshot/reset the observed wire volume.
    let counter: Arc<KcpBytesCounter> = Arc::new(KcpBytesCounter::new());

    // P5: shared AOI broadphase grid. Game loop rebuilds per tick, transport
    // thread reads for `BroadcastPolicy::AoiEntity` lookups. `std::sync::Mutex`
    // (not `tokio::sync::Mutex`) because both touch-points are synchronous
    // code holding the lock for microseconds — no `.await` while locked.
    let aoi: Arc<std::sync::Mutex<AoiGrid>> =
        Arc::new(std::sync::Mutex::new(AoiGrid::new()));

    // Background thread: read from out_rx and broadcast to all sessions
    let sessions_broadcast = sessions.clone();
    let counter_broadcast = counter.clone();
    let aoi_broadcast = aoi.clone();
    thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            // ===== P6: Two-tier batch window =====
            // Urgent events (death/spawn/projectile/explosion/buff/tower upgrade/
            // creep.stall/game.*) flush ASAP for sub-10ms UX latency.
            // Normal events (creep.M/H/S, entity.F, hero.hot, heartbeat) enter
            // a 10~33ms window so they benefit from dedupe.
            //
            // Algorithm:
            //   1. Block on first message.
            //   2. If first is Urgent → drain what's already ready (try_recv
            //      loop), flush immediately.
            //   3. Otherwise: batch for MIN_BATCH, then keep batching up to
            //      MAX_BATCH — BUT if any Urgent message arrives after
            //      MIN_BATCH, flush immediately.
            //
            // Rationale for MIN_BATCH = 10ms: gives dedupe a real chance to
            // collapse creep.M/.H bursts emitted within the same tick (server
            // runs at ~30fps so one tick's worth of events fires in <1ms, but
            // consecutive ticks' Normals can still land in the same window).
            use std::time::{Duration, Instant};
            const MIN_BATCH: Duration = Duration::from_millis(10);
            const MAX_BATCH: Duration = Duration::from_millis(33);

            'outer: loop {
                // 等第一筆訊息（阻塞）
                let first = match out_rx.recv() {
                    Ok(m) => m,
                    Err(_) => {
                        info!("Outbound channel closed, stopping KCP broadcaster");
                        break 'outer;
                    }
                };
                let first_is_urgent = outbound_urgency(&first) == Urgency::Urgent;
                let mut batch: Vec<crate::transport::OutboundMsg> = vec![first];
                let window_start = Instant::now();

                if first_is_urgent {
                    // Urgent head: drain whatever's already queued without
                    // blocking, flush immediately. Keeps the latency budget
                    // <1ms for this event while still batching anything that
                    // happened to arrive alongside it.
                    while let Ok(m) = out_rx.try_recv() {
                        batch.push(m);
                    }
                } else {
                    // Normal head: batch for MIN_BATCH unconditionally, then
                    // between MIN_BATCH..=MAX_BATCH flush early on any Urgent.
                    loop {
                        let now = Instant::now();
                        let elapsed = now.duration_since(window_start);
                        if elapsed >= MAX_BATCH {
                            break;
                        }
                        let timeout = MAX_BATCH - elapsed;
                        match out_rx.recv_timeout(timeout) {
                            Ok(m) => {
                                let is_lockstep = m.lockstep_frame.is_some();
                                let is_urg = outbound_urgency(&m) == Urgency::Urgent;
                                batch.push(m);
                                if is_lockstep {
                                    break;
                                }
                                // If Urgent arrived after MIN_BATCH, don't
                                // wait any longer — the batch has already
                                // earned its dedupe savings.
                                if is_urg && window_start.elapsed() >= MIN_BATCH {
                                    break;
                                }
                            }
                            Err(crossbeam_channel::RecvTimeoutError::Timeout) => break,
                            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break 'outer,
                        }
                    }
                }

                // Collapse redundant latest-wins updates (creep.M / *.H / entity.F / creep.S / hero.stats)
                // before encoding. See `is_dedupable` for the full policy.
                let batch = dedupe_batch(batch);

                // 處理整個批次
                for msg in batch {
                    // Phase 2 lockstep: when `lockstep_frame` is set, emit the
                    // corresponding tag directly, bypassing the GameEvent
                    // envelope. Targets:
                    //   TickBatch / StateHash → all sessions with
                    //                           lockstep_joined=true
                    //   GameStart / SnapshotResp → unicast to client_session_id
                    if let Some(frame) = msg.lockstep_frame.clone() {
                        match frame {
                            crate::lockstep::LockstepFrame::TickBatch(batch_msg) => {
                                let payload = batch_msg.encode_to_vec();
                                let frame_bytes = build_framed_bytes(TAG_TICK_BATCH, &payload);
                                let frame_arc: Arc<[u8]> = Arc::from(frame_bytes.into_boxed_slice());
                                let sessions = sessions_broadcast.lock().await;
                                let mut to_remove = Vec::new();
                                for (sid, session) in sessions.iter() {
                                    if !session.lockstep_joined { continue; }
                                    if session.event_tx.try_send(frame_arc.clone()).is_err() {
                                        to_remove.push(sid.clone());
                                    }
                                }
                                drop(sessions);
                                if !to_remove.is_empty() {
                                    let mut sessions = sessions_broadcast.lock().await;
                                    for id in to_remove {
                                        sessions.remove(&id);
                                        info!("Removed disconnected KCP session: {}", id);
                                    }
                                }
                            }
                            crate::lockstep::LockstepFrame::StateHash(sh) => {
                                let payload = sh.encode_to_vec();
                                let frame_bytes = build_framed_bytes(TAG_STATE_HASH, &payload);
                                let frame_arc: Arc<[u8]> = Arc::from(frame_bytes.into_boxed_slice());
                                let sessions = sessions_broadcast.lock().await;
                                let mut to_remove = Vec::new();
                                for (sid, session) in sessions.iter() {
                                    if !session.lockstep_joined { continue; }
                                    if session.event_tx.try_send(frame_arc.clone()).is_err() {
                                        to_remove.push(sid.clone());
                                    }
                                }
                                drop(sessions);
                                if !to_remove.is_empty() {
                                    let mut sessions = sessions_broadcast.lock().await;
                                    for id in to_remove {
                                        sessions.remove(&id);
                                        info!("Removed disconnected KCP session: {}", id);
                                    }
                                }
                            }
                            crate::lockstep::LockstepFrame::GameStart { client_session_id, msg: gs } => {
                                let payload = gs.encode_to_vec();
                                let frame_bytes = build_framed_bytes(TAG_GAME_START, &payload);
                                let frame_arc: Arc<[u8]> = Arc::from(frame_bytes.into_boxed_slice());
                                let sessions = sessions_broadcast.lock().await;
                                if let Some(session) = sessions.get(&client_session_id) {
                                    let _ = session.event_tx.try_send(frame_arc);
                                } else {
                                    warn!("GameStart unicast: session '{}' not found", client_session_id);
                                }
                            }
                            crate::lockstep::LockstepFrame::SnapshotResp { client_session_id, msg: sr } => {
                                let payload = sr.encode_to_vec();
                                let frame_bytes = build_framed_bytes(TAG_SNAPSHOT_RESP, &payload);
                                let frame_arc: Arc<[u8]> = Arc::from(frame_bytes.into_boxed_slice());
                                let sessions = sessions_broadcast.lock().await;
                                if let Some(session) = sessions.get(&client_session_id) {
                                    let _ = session.event_tx.try_send(frame_arc);
                                } else {
                                    warn!("SnapshotResp unicast: session '{}' not found", client_session_id);
                                }
                            }
                        }
                        continue; // skip legacy GameEvent path
                    }
                    {
                        // Parse the msg JSON to extract t, a, d fields
                        let (msg_type, action, data_bytes) = if let Ok(parsed) =
                            serde_json::from_str::<serde_json::Value>(&msg.msg)
                        {
                            let t = parsed.get("t").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            let a = parsed.get("a").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            let d = parsed.get("d").map(|v| v.to_string().into_bytes()).unwrap_or_default();
                            (t, a, d)
                        } else {
                            ("".to_string(), "".to_string(), msg.msg.as_bytes().to_vec())
                        };

                        let timestamp_ms = msg
                            .time
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis() as u64)
                            .unwrap_or(0);

                        // P9 envelope-strip: every event must carry a typed
                        // `payload` oneof. When `msg.typed` is None (legacy
                        // emit site), wrap the JSON in a `LegacyJson` variant
                        // so the wire still avoids envelope strings.
                        let payload = match msg.typed.as_ref() {
                            Some(TypedOutbound::Heartbeat(m))         => game_event::Payload::Heartbeat(m.clone()),
                            Some(TypedOutbound::HeroCreate(m))        => game_event::Payload::HeroCreate(m.clone()),
                            Some(TypedOutbound::UnitCreate(m))        => game_event::Payload::UnitCreate(m.clone()),
                            Some(TypedOutbound::BuffAdd(m))           => game_event::Payload::BuffAdd(m.clone()),
                            Some(TypedOutbound::BuffRemove(m))        => game_event::Payload::BuffRemove(m.clone()),
                            Some(TypedOutbound::GameLives(m))         => game_event::Payload::GameLives(m.clone()),
                            Some(TypedOutbound::GameEnd(m))           => game_event::Payload::GameEnd(m.clone()),
                            Some(TypedOutbound::LegacyJson(m))        => game_event::Payload::LegacyJson(m.clone()),
                            None => game_event::Payload::LegacyJson(LegacyJson {
                                msg_type: msg_type.clone(),
                                action: action.clone(),
                                data_json: data_bytes,
                            }),
                        };
                        let _ = timestamp_ms; // P9: timestamp now client-local

                        // P6: per-session sequence stamped below; template is shared.
                        let event_template = GameEvent {
                            sequence: 0,
                            payload: Some(payload),
                        };

                        let sessions = sessions_broadcast.lock().await;
                        let mut to_remove = Vec::new();

                        // Resolve BroadcastPolicy → target session IDs.
                        //
                        // Policy dispatch is explicit (P5). If `msg.policy` is
                        // None we fall back to the legacy topic-based heuristic
                        // for un-migrated emit sites. The legacy path also
                        // honours `entity_pos` viewport filtering for AOI —
                        // keeps creep/projectile events AOI-gated during the
                        // per-site migration rollout.
                        let targets: Vec<String> = match &msg.policy {
                            Some(BroadcastPolicy::All) => {
                                sessions.keys().cloned().collect()
                            }
                            Some(BroadcastPolicy::PlayerOnly(name)) => {
                                // Session map is keyed by session_id (kcp_<addr>);
                                // we need to find the session whose player_name
                                // matches. O(N) but N = players (≤ ~32).
                                sessions.iter()
                                    .filter(|(_, s)| &s.player_name == name)
                                    .map(|(id, _)| id.clone())
                                    .collect()
                            }
                            Some(BroadcastPolicy::AoiPoint(x, y)) => {
                                sessions.iter()
                                    .filter(|(_, s)| match &s.viewport {
                                        Some(vp) => vp.contains(*x, *y),
                                        None => true, // no viewport yet → pass through (heartbeat / join flow)
                                    })
                                    .map(|(id, _)| id.clone())
                                    .collect()
                            }
                            Some(BroadcastPolicy::AoiEntity(eid)) => {
                                // Resolve entity_id → pos via AoiGrid. If the
                                // grid doesn't know this entity (spawned this
                                // tick after rebuild, or already dead), fall
                                // back to broadcast to avoid silently dropping.
                                let pos_opt = aoi_broadcast.lock()
                                    .ok()
                                    .and_then(|g| g.lookup_pos(*eid));
                                match pos_opt {
                                    Some((x, y)) => sessions.iter()
                                        .filter(|(_, s)| match &s.viewport {
                                            Some(vp) => vp.contains(x, y),
                                            None => true,
                                        })
                                        .map(|(id, _)| id.clone())
                                        .collect(),
                                    None => sessions.keys().cloned().collect(),
                                }
                            }
                            None => {
                                // Legacy topic-based routing. "/all/" ⇒ broadcast;
                                // "/<player_name>/" ⇒ per-player. entity_pos
                                // provides viewport filter for both.
                                let is_broadcast = msg.topic.contains("/all/");
                                sessions.iter()
                                    .filter(|(_, s)| {
                                        let topic_ok = is_broadcast
                                            || msg.topic.contains(&format!("/{}/", s.player_name))
                                            || s.player_name.is_empty();
                                        if !topic_ok { return false; }
                                        match (msg.entity_pos, &s.viewport) {
                                            (Some((x, y)), Some(vp)) => vp.contains(x, y),
                                            _ => true,
                                        }
                                    })
                                    .map(|(id, _)| id.clone())
                                    .collect()
                            }
                        };

                        let is_per_player_topic = !msg.topic.contains("/all/") && msg.topic.starts_with("td/") && msg.topic.ends_with("/res");
                        let mut route_hits = 0u32;
                        // P6: encode+compress+frame per session because each
                        // session stamps its own `sequence`. The payload bytes
                        // differ ONLY in the sequence field (typically 1~2
                        // varint bytes) so CPU cost is dominated by LZ4 which
                        // we re-run per target. For the expected ≤32 players
                        // this is comfortably within the 33ms max window.
                        //
                        // A future optimisation could hoist the compressed
                        // bytes minus-the-sequence and splice — but that
                        // complicates prost framing and the measured cost is
                        // already acceptable.
                        for target in &targets {
                            if let Some(session) = sessions.get(target) {
                                // Stamp the per-session sequence (monotonic,
                                // no gaps — client uses these to detect loss
                                // even though AOI may drop events pre-stamp).
                                let seq_val = session.seq.fetch_add(1, Ordering::Relaxed);
                                let mut ev = event_template.clone();
                                ev.sequence = seq_val;
                                let payload = ev.encode_to_vec();

                                // Compress payload when ≥ threshold AND the
                                // compressed bytes are strictly smaller;
                                // otherwise fall back to raw.
                                // KEEP IN SYNC with write_framed above.
                                let (frame_tag, frame_payload): (u8, std::borrow::Cow<'_, [u8]>) =
                                    if payload.len() >= LZ4_THRESHOLD {
                                        let c = lz4_flex::block::compress_prepend_size(&payload);
                                        if c.len() < payload.len() {
                                            (TAG_GAME_EVENT | COMPRESSION_FLAG, std::borrow::Cow::Owned(c))
                                        } else {
                                            (TAG_GAME_EVENT, std::borrow::Cow::Borrowed(&payload))
                                        }
                                    } else {
                                        (TAG_GAME_EVENT, std::borrow::Cow::Borrowed(&payload))
                                    };

                                let mut frame = Vec::with_capacity(1 + 4 + frame_payload.len());
                                frame.push(frame_tag);
                                frame.extend_from_slice(&(frame_payload.len() as u32).to_be_bytes());
                                frame.extend_from_slice(&frame_payload);

                                // Record observed wire bytes per session so the
                                // counter reflects real wire volume (N sessions
                                // × 1 encoded frame each).
                                counter_broadcast.record(&msg_type, &action, frame.len());

                                let frame_arc: Arc<[u8]> = Arc::from(frame.into_boxed_slice());
                                if session.event_tx.try_send(frame_arc).is_err() {
                                    to_remove.push(target.clone());
                                } else {
                                    route_hits += 1;
                                }
                            }
                        }
                        if is_per_player_topic {
                            log::debug!("📡 routed per-player topic='{}' policy={:?} hits={}/{} (sessions={})",
                                msg.topic, msg.policy.as_ref().map(|_| "explicit").unwrap_or("legacy"),
                                route_hits, targets.len(), sessions.len());
                        }
                        drop(sessions);

                        if !to_remove.is_empty() {
                            let mut sessions = sessions_broadcast.lock().await;
                            for id in to_remove {
                                sessions.remove(&id);
                                info!("Removed disconnected KCP session: {}", id);
                            }
                        }
                    }
                }
            }
        });
    });

    // Resolve bind address
    let bind_ip = match server_addr.as_str() {
        "localhost" | "127.0.0.1" => "0.0.0.0".to_string(),
        other => other.to_string(),
    };
    let addr = format!("{}:{}", bind_ip, server_port);
    let addr: std::net::SocketAddr = addr
        .parse()
        .map_err(|e| failure::err_msg(format!("Invalid address '{}': {}", addr, e)))?;

    let mut config = KcpConfig::default();
    config.nodelay = KcpNoDelayConfig::fastest();

    info!("Starting KCP server on {}", addr);

    let sessions_accept = sessions.clone();
    let in_tx_accept = in_tx.clone();
    let query_tx_accept = query_tx.clone();
    let viewport_tx_accept = viewport_tx.clone();
    let out_tx_accept = out_tx.clone();
    let lockstep_input_buffer_accept = lockstep_input_buffer.clone();
    let lockstep_state_accept = lockstep_state.clone();
    let lockstep_snapshot_store_accept = lockstep_snapshot_store.clone();

    // Bind synchronously so startup fails fast if the port is taken by a stale instance.
    let mut listener = KcpListener::bind(config, addr)
        .await
        .map_err(|e| failure::err_msg(format!("Failed to bind KCP listener on {}: {}", addr, e)))?;

    tokio::spawn(async move {
        loop {
            let (stream, peer_addr) = match listener.accept().await {
                Ok(s) => s,
                Err(e) => {
                    error!("KCP accept error: {}", e);
                    continue;
                }
            };

            info!("KCP client connected from {}", peer_addr);

            let sessions = sessions_accept.clone();
            let in_tx = in_tx_accept.clone();
            let query_tx = query_tx_accept.clone();
            let viewport_tx = viewport_tx_accept.clone();
            let out_tx = out_tx_accept.clone();
            let lockstep_input_buffer = lockstep_input_buffer_accept.clone();
            let lockstep_state = lockstep_state_accept.clone();
            let lockstep_snapshot_store = lockstep_snapshot_store_accept.clone();
            let session_id = format!("kcp_{}", peer_addr);

            tokio::spawn(async move {
                if let Err(e) = handle_client(
                    stream, session_id, sessions, in_tx, query_tx, viewport_tx,
                    out_tx, lockstep_input_buffer, lockstep_state, lockstep_snapshot_store,
                ).await {
                    warn!("KCP client handler error: {}", e);
                }
            });
        }
    });

    Ok(TransportHandle {
        tx: out_tx,
        rx: in_rx,
        query_rx,
        viewport_rx,
        counter,
        aoi,
    })
}

async fn handle_client(
    stream: KcpStream,
    session_id: String,
    sessions: Arc<Mutex<HashMap<String, ClientSession>>>,
    in_tx: Sender<InboundMsg>,
    query_tx: Sender<QueryRequest>,
    viewport_tx: Sender<ViewportMsg>,
    out_tx: Sender<OutboundMsg>,
    lockstep_input_buffer: Arc<std::sync::Mutex<crate::lockstep::InputBuffer>>,
    lockstep_state: Arc<std::sync::Mutex<crate::lockstep::LockstepState>>,
    lockstep_snapshot_store: Arc<std::sync::Mutex<crate::comp::SnapshotStore>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (mut reader, mut writer) = tokio::io::split(stream);

    // Per-session outbound channel (lazy — only used after SubscribeRequest).
    // P5: payload is `Arc<[u8]>` — same bytes the broadcast thread encoded and
    // shared across all recipient sessions. Each session's writer task
    // dereferences the Arc and writes the slice to KCP — no copy.
    let mut event_rx: Option<tokio::sync::mpsc::Receiver<Arc<[u8]>>> = None;
    let mut subscribed = false;
    // Track the subscribed player_name so we can send a Remove on disconnect
    let mut player_name: Option<String> = None;

    // Main loop: read from client, optionally write outbound events
    loop {
        tokio::select! {
            result = read_framed(&mut reader) => {
                match result {
                    Ok(Some((tag, payload, _wire_bytes))) => {
                        match tag {
                            TAG_SUBSCRIBE_REQUEST => {
                                if let Ok(sub) = SubscribeRequest::decode(payload.as_slice()) {
                                    info!("🔌 KCP client subscribed as '{}' (session_id={})", sub.player_name, session_id);
                                    let (event_tx, rx) = tokio::sync::mpsc::channel::<Arc<[u8]>>(10000);
                                    event_rx = Some(rx);
                                    subscribed = true;
                                    player_name = Some(sub.player_name.clone());
                                    let mut sess = sessions.lock().await;
                                    sess.insert(
                                        session_id.clone(),
                                        ClientSession {
                                            player_name: sub.player_name,
                                            event_tx,
                                            viewport: None,
                                            // P6: per-session monotonic seq
                                            // starts at 0 on subscribe; the
                                            // first GameEvent delivered to
                                            // this session carries sequence=0.
                                            seq: Arc::new(AtomicU64::new(0)),
                                            // Phase 2: legacy SubscribeRequest
                                            // path — client is NOT on the
                                            // lockstep stream until it sends
                                            // a JoinRequest (0x13).
                                            lockstep_joined: false,
                                        },
                                    );
                                }
                            }
                            TAG_PLAYER_COMMAND => {
                                if let Ok(cmd) = PlayerCommand::decode(payload.as_slice()) {
                                    let data_json: serde_json::Value = if cmd.data_json.is_empty() {
                                        serde_json::Value::Null
                                    } else {
                                        serde_json::from_slice(&cmd.data_json)
                                            .unwrap_or(serde_json::Value::Null)
                                    };

                                    let inbound = InboundMsg {
                                        name: cmd.player_name,
                                        t: cmd.msg_type,
                                        a: cmd.action,
                                        d: data_json,
                                    };

                                    let _ = in_tx.send(inbound);

                                    // Send ack
                                    let ack = CommandAck {
                                        ok: true,
                                        message: "Command accepted".into(),
                                    };
                                    let ack_payload = ack.encode_to_vec();
                                    let _ = write_framed(&mut writer, TAG_COMMAND_ACK, &ack_payload).await;
                                }
                            }
                            TAG_GAME_STATE_REQUEST => {
                                if let Ok(req) = GameStateRequest::decode(payload.as_slice()) {
                                    // P6: handle "seq-gap" client-initiated
                                    // resync. player_name carries the
                                    // last-known seq as a decimal string (to
                                    // avoid a proto schema bump). For now we
                                    // just LOG + ACK — a full state snapshot
                                    // response is deferred to a follow-up.
                                    // Clients that see a gap will retry the
                                    // request periodically until the server
                                    // catches up organically.
                                    if req.query_type == "seq-gap" {
                                        warn!(
                                            "⚠️ seq-gap resync request from session={} last_seq={:?}",
                                            session_id, req.player_name
                                        );
                                        // Stub: ACK so client knows server
                                        // received the request. A future patch
                                        // should build a full view snapshot
                                        // (hero/creep/tower in AOI) and ship
                                        // it back as a batched replay.
                                        let resp = GameStateResponse {
                                            success: true,
                                            error: String::new(),
                                            data_json: b"{\"stub\":true,\"note\":\"seq-gap snapshot not yet implemented - server logged request\"}".to_vec(),
                                        };
                                        let resp_payload = resp.encode_to_vec();
                                        let _ = write_framed(&mut writer, TAG_GAME_STATE_RESPONSE, &resp_payload).await;
                                        continue;
                                    }
                                    let (resp_tx, resp_rx) = tokio::sync::oneshot::channel();
                                    let query = QueryRequest {
                                        query_type: req.query_type,
                                        player_name: req.player_name,
                                        response_tx: resp_tx,
                                    };

                                    let _ = query_tx.send(query);

                                    if let Ok(response) = resp_rx.await {
                                        let resp = GameStateResponse {
                                            success: response.success,
                                            error: response.error,
                                            data_json: response.data_json,
                                        };
                                        let resp_payload = resp.encode_to_vec();
                                        let _ = write_framed(&mut writer, TAG_GAME_STATE_RESPONSE, &resp_payload).await;
                                    }
                                }
                            }
                            TAG_VIEWPORT_UPDATE => {
                                if let Ok(vp) = ViewportUpdate::decode(payload.as_slice()) {
                                    if subscribed {
                                        let viewport = Viewport::new(
                                            vp.center_x, vp.center_y, vp.half_width, vp.half_height,
                                        );
                                        let mut sess = sessions.lock().await;
                                        if let Some(s) = sess.get_mut(&session_id) {
                                            info!("🎥 Viewport update from '{}': center=({}, {}), half=({}, {}), padded=({}, {})",
                                                s.player_name, vp.center_x, vp.center_y,
                                                vp.half_width, vp.half_height,
                                                viewport.padded_hw, viewport.padded_hh);
                                            s.viewport = Some(viewport);
                                            // Notify game loop so visibility diff can use it
                                            match viewport_tx.send(ViewportMsg::Set {
                                                player_name: s.player_name.clone(),
                                                viewport,
                                            }) {
                                                Ok(()) => info!("📤 Forwarded ViewportMsg::Set('{}') to game loop", s.player_name),
                                                Err(e) => warn!("Failed to forward ViewportMsg: {}", e),
                                            }
                                        } else {
                                            warn!("Viewport update but session '{}' not found", session_id);
                                        }
                                    } else {
                                        warn!("Viewport update before subscribe — ignored");
                                    }
                                } else {
                                    warn!("Failed to decode ViewportUpdate payload");
                                }
                            }
                            // ===== Phase 2 Lockstep tags =====
                            TAG_INPUT_SUBMIT => {
                                match InputSubmit::decode(payload.as_slice()) {
                                    Ok(req) => {
                                        let current_tick = lockstep_state.lock().unwrap().current_tick;
                                        let player_id = req.player_id;
                                        let target_tick = req.target_tick;
                                        let input_id = req.input_id;
                                        let input = req.input.unwrap_or_default();
                                        let accepted = lockstep_input_buffer
                                            .lock()
                                            .unwrap()
                                            .submit(current_tick, player_id, target_tick, input, input_id);
                                        if !accepted {
                                            warn!(
                                                "late InputSubmit from player {} target_tick={} current_tick={}",
                                                player_id, target_tick, current_tick
                                            );
                                        }
                                    }
                                    Err(e) => warn!("Failed to decode InputSubmit: {}", e),
                                }
                            }
                            TAG_JOIN_REQUEST => {
                                match JoinRequest::decode(payload.as_slice()) {
                                    Ok(req) => {
                                        let role = match req.role {
                                            x if x == JoinRole::RoleObserver as i32 => crate::lockstep::JoinRoleEnum::Observer,
                                            _ => crate::lockstep::JoinRoleEnum::Player,
                                        };
                                        let (player_id, master_seed, start_tick) = {
                                            let mut s = lockstep_state.lock().unwrap();
                                            let pid = s.register_player(req.player_name.clone(), role);
                                            (pid, s.master_seed, s.current_tick)
                                        };
                                        // Mark this session as joined to the
                                        // lockstep stream so future TickBatch /
                                        // StateHash broadcasts reach it.
                                        {
                                            let mut sess = sessions.lock().await;
                                            if let Some(s) = sess.get_mut(&session_id) {
                                                s.lockstep_joined = true;
                                                if s.player_name.is_empty() {
                                                    s.player_name = req.player_name.clone();
                                                }
                                            } else {
                                                // No prior SubscribeRequest —
                                                // create the session lazily so
                                                // lockstep-only clients (no
                                                // legacy GameEvent channel) can
                                                // still receive TickBatch.
                                                let (event_tx, rx) = tokio::sync::mpsc::channel::<Arc<[u8]>>(10000);
                                                event_rx = Some(rx);
                                                subscribed = true;
                                                player_name = Some(req.player_name.clone());
                                                sess.insert(
                                                    session_id.clone(),
                                                    ClientSession {
                                                        player_name: req.player_name.clone(),
                                                        event_tx,
                                                        viewport: None,
                                                        seq: Arc::new(AtomicU64::new(0)),
                                                        lockstep_joined: true,
                                                    },
                                                );
                                            }
                                        }
                                        info!(
                                            "🎮 KCP lockstep JoinRequest player='{}' role={:?} → assigned player_id={} (session={})",
                                            req.player_name, role, player_id, session_id
                                        );
                                        // Send GameStart unicast back via the
                                        // broadcast thread (so it goes through
                                        // the same per-session event_tx the
                                        // client is reading from).
                                        let game_start = GameStart {
                                            player_id,
                                            start_tick,
                                            master_seed,
                                            initial_state: Some(SimSnapshot {
                                                world_bytes: vec![],
                                                schema_version: 1,
                                            }),
                                        };
                                        let frame = crate::lockstep::LockstepFrame::GameStart {
                                            client_session_id: session_id.clone(),
                                            msg: game_start,
                                        };
                                        if let Err(e) = out_tx.send(OutboundMsg::lockstep_frame(frame)) {
                                            warn!("Failed to enqueue GameStart: {}", e);
                                        }
                                    }
                                    Err(e) => warn!("Failed to decode JoinRequest: {}", e),
                                }
                            }
                            TAG_SNAPSHOT_REQ => {
                                match SnapshotReq::decode(payload.as_slice()) {
                                    Ok(req) => {
                                        // Phase 5.3: serve the latest
                                        // bincode-serialized world snapshot
                                        // from the shared SnapshotStore. The
                                        // dispatcher tick loop refreshes this
                                        // every SNAPSHOT_INTERVAL_TICKS
                                        // (= 30 s @ 30 Hz). Empty bytes mean
                                        // no snapshot has been captured yet —
                                        // the observer falls back to playing
                                        // forward from `current_tick` without
                                        // bootstrap.
                                        let (snapshot_tick, snapshot_bytes) = {
                                            let store = lockstep_snapshot_store
                                                .lock()
                                                .expect("SnapshotStore mutex poisoned");
                                            (store.tick, store.bytes.clone())
                                        };
                                        let current_tick = lockstep_state.lock().unwrap().current_tick;
                                        info!(
                                            "📸 SnapshotReq from {} for from_tick={} → serving snapshot tick={} bytes={} (current_tick={})",
                                            session_id, req.from_tick, snapshot_tick, snapshot_bytes.len(), current_tick
                                        );
                                        let resp = SnapshotResp {
                                            tick: snapshot_tick,
                                            state: Some(SimSnapshot {
                                                world_bytes: snapshot_bytes,
                                                schema_version: crate::lockstep::SNAPSHOT_SCHEMA_VERSION,
                                            }),
                                        };
                                        let frame = crate::lockstep::LockstepFrame::SnapshotResp {
                                            client_session_id: session_id.clone(),
                                            msg: resp,
                                        };
                                        if let Err(e) = out_tx.send(OutboundMsg::lockstep_frame(frame)) {
                                            warn!("Failed to enqueue SnapshotResp: {}", e);
                                        }
                                    }
                                    Err(e) => warn!("Failed to decode SnapshotReq: {}", e),
                                }
                            }
                            TAG_PING_REQ => {
                                // Echo PingResponse with the same client_send_us
                                // so the client can derive RTT. Direct write —
                                // bypasses the broadcast channel for minimum
                                // turnaround time (out_tx adds queuing delay
                                // that contaminates the RTT measurement).
                                match PingRequest::decode(payload.as_slice()) {
                                    Ok(req) => {
                                        let resp = PingResponse {
                                            client_send_us: req.client_send_us,
                                        };
                                        let resp_payload = resp.encode_to_vec();
                                        let _ = write_framed(&mut writer, TAG_PING_RESP, &resp_payload).await;
                                    }
                                    Err(e) => warn!("Failed to decode PingRequest: {}", e),
                                }
                            }
                            _ => {
                                warn!("Unknown tag from client: 0x{:02x}", tag);
                            }
                        }
                    }
                    Ok(None) => {
                        info!("KCP client disconnected: {}", session_id);
                        break;
                    }
                    Err(e) => {
                        warn!("KCP read error for {}: {}", session_id, e);
                        break;
                    }
                }
            }
            Some(frame) = async {
                match event_rx.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                // P5: `frame` is `Arc<[u8]>` shared with every other session
                // that got this same event. Writing the underlying slice
                // copies into the KCP socket buffer but the game-side
                // allocation is refcounted — dropped when the last session
                // flushes.
                if writer.write_all(&frame).await.is_err() {
                    break;
                }
                let _ = writer.flush().await;
            }
        }
    }

    // Cleanup session
    {
        let mut sess = sessions.lock().await;
        sess.remove(&session_id);
    }
    // Inform game loop that this player's viewport is gone
    if let Some(name) = player_name {
        let _ = viewport_tx.send(ViewportMsg::Remove { player_name: name });
    }
    info!("KCP session cleaned up: {}", session_id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make(t: &str, a: &str, v: serde_json::Value) -> OutboundMsg {
        OutboundMsg::new_s("td/all/res", t, a, v)
    }

    #[test]
    fn dedupe_collapses_creep_h_same_entity() {
        // 3 creep.H updates for entity 42 arriving in one window should
        // collapse to a single message carrying the LATEST hp value.
        let batch = vec![
            make("creep", "H", json!({ "id": 42, "hp": 100.0, "max_hp": 200.0 })),
            make("creep", "H", json!({ "id": 42, "hp": 80.0, "max_hp": 200.0 })),
            make("creep", "H", json!({ "id": 42, "hp": 50.0, "max_hp": 200.0 })),
        ];
        let out = dedupe_batch(batch);
        assert_eq!(out.len(), 1);
        let parsed: serde_json::Value = serde_json::from_str(&out[0].msg).unwrap();
        assert_eq!(parsed["d"]["hp"].as_f64(), Some(50.0));
    }

    #[test]
    fn dedupe_preserves_different_entities() {
        let batch = vec![
            make("creep", "H", json!({ "id": 42, "hp": 100.0, "max_hp": 200.0 })),
            make("creep", "H", json!({ "id": 43, "hp":  90.0, "max_hp": 200.0 })),
        ];
        let out = dedupe_batch(batch);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn dedupe_preserves_different_actions() {
        // creep.H and creep.M share entity but are different actions → both keep.
        let batch = vec![
            make("creep", "H", json!({ "id": 42, "hp": 100.0, "max_hp": 200.0 })),
            make("creep", "M", json!({ "id": 42, "x": 1.0, "y": 2.0 })),
        ];
        let out = dedupe_batch(batch);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn non_dedupable_passes_through() {
        // Two creation events for the same entity must BOTH survive (creation
        // is semantic and the second one may carry the real data while the
        // first is only a placeholder — regardless, we must not collapse).
        let batch = vec![
            make("creep", "C", json!({ "id": 42, "kind": "orc" })),
            make("creep", "C", json!({ "id": 42, "kind": "orc" })),
        ];
        let out = dedupe_batch(batch);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn unknown_kind_passes_through() {
        // Malformed JSON → peek returns ("", "", 0), dedupe skips it (not panic).
        // Unknown (msg_type, action) pair → not in is_dedupable → pass-through.
        let mut raw_bad = OutboundMsg::new_s("td/all/res", "x", "y", json!({}));
        raw_bad.msg = "not json at all }}}".to_string();

        let batch = vec![
            raw_bad,
            make("game", "lives", json!({ "lives": 3 })), // unknown kind per policy
            make("buff", "buff_add", json!({ "id": 42, "buff": "slow" })),
        ];
        let out = dedupe_batch(batch);
        assert_eq!(out.len(), 3);
    }

    // ===== P5 BroadcastPolicy dispatch tests =====

    use std::collections::BTreeMap;

    fn mk_sessions(entries: &[(&str, &str, Option<Viewport>)])
        -> BTreeMap<String, (String, Option<Viewport>)>
    {
        entries.iter()
            .map(|(id, name, vp)| (id.to_string(), (name.to_string(), *vp)))
            .collect()
    }

    #[test]
    fn policy_all_reaches_every_session() {
        let sessions = mk_sessions(&[
            ("s1", "alice", Some(Viewport::new(0.0, 0.0, 100.0, 100.0))),
            ("s2", "bob",   Some(Viewport::new(1000.0, 1000.0, 100.0, 100.0))),
            ("s3", "carol", None),
        ]);
        let targets = select_targets_for_policy(
            Some(&BroadcastPolicy::All),
            "td/all/res", None, &sessions, &|_| None,
        );
        assert_eq!(targets.len(), 3);
    }

    #[test]
    fn policy_player_only_hits_one_session() {
        let sessions = mk_sessions(&[
            ("s1", "alice", None),
            ("s2", "bob",   None),
        ]);
        let targets = select_targets_for_policy(
            Some(&BroadcastPolicy::PlayerOnly("bob".into())),
            "td/bob/res", None, &sessions, &|_| None,
        );
        assert_eq!(targets, vec!["s2".to_string()]);
    }

    #[test]
    fn policy_aoi_point_filters_by_viewport() {
        let sessions = mk_sessions(&[
            ("s1", "alice", Some(Viewport::new(0.0, 0.0, 100.0, 100.0))),
            ("s2", "bob",   Some(Viewport::new(1000.0, 1000.0, 100.0, 100.0))),
            ("s3", "no_vp", None),
        ]);
        // Event at (10, 10) — alice sees it, bob doesn't, no_vp passes through
        // (policy treats missing viewport as "not yet filtering" so heartbeat
        // / initial state still reaches them).
        let targets = select_targets_for_policy(
            Some(&BroadcastPolicy::AoiPoint(10.0, 10.0)),
            "td/all/res", None, &sessions, &|_| None,
        );
        let mut sorted = targets.clone();
        sorted.sort();
        assert_eq!(sorted, vec!["s1".to_string(), "s3".to_string()]);
    }

    #[test]
    fn policy_aoi_entity_uses_grid_lookup() {
        let sessions = mk_sessions(&[
            ("s1", "alice", Some(Viewport::new(500.0, 500.0, 100.0, 100.0))),
            ("s2", "bob",   Some(Viewport::new(0.0, 0.0, 100.0, 100.0))),
        ]);
        // Entity 42 lives at (500, 500) — only alice's viewport contains it.
        let lookup = |eid: u64| if eid == 42 { Some((500.0, 500.0)) } else { None };
        let targets = select_targets_for_policy(
            Some(&BroadcastPolicy::AoiEntity(42)),
            "td/all/res", None, &sessions, &lookup,
        );
        assert_eq!(targets, vec!["s1".to_string()]);
    }

    #[test]
    fn policy_aoi_entity_unknown_falls_back_to_broadcast() {
        let sessions = mk_sessions(&[
            ("s1", "alice", Some(Viewport::new(500.0, 500.0, 100.0, 100.0))),
            ("s2", "bob",   Some(Viewport::new(0.0, 0.0, 100.0, 100.0))),
        ]);
        // Entity 999 unknown → broadcast to every session (safety fallback).
        let targets = select_targets_for_policy(
            Some(&BroadcastPolicy::AoiEntity(999)),
            "td/all/res", None, &sessions, &|_| None,
        );
        assert_eq!(targets.len(), 2);
    }

    #[test]
    fn policy_none_preserves_legacy_topic_routing() {
        let sessions = mk_sessions(&[
            ("s1", "alice", Some(Viewport::new(0.0, 0.0, 100.0, 100.0))),
            ("s2", "bob",   Some(Viewport::new(1000.0, 1000.0, 100.0, 100.0))),
        ]);
        // Legacy /all/ topic + entity_pos → viewport filter applied.
        // Event at (0, 0) — alice contains, bob doesn't.
        let targets = select_targets_for_policy(
            None, "td/all/res", Some((0.0, 0.0)), &sessions, &|_| None,
        );
        assert_eq!(targets, vec!["s1".to_string()]);

        // Per-player topic "td/bob/res" → only bob's session.
        let targets = select_targets_for_policy(
            None, "td/bob/res", None, &sessions, &|_| None,
        );
        assert_eq!(targets, vec!["s2".to_string()]);
    }

    #[test]
    fn policy_none_no_pos_reaches_all_matching_topic() {
        let sessions = mk_sessions(&[
            ("s1", "alice", Some(Viewport::new(0.0, 0.0, 100.0, 100.0))),
            ("s2", "bob",   Some(Viewport::new(1000.0, 1000.0, 100.0, 100.0))),
        ]);
        // /all/ topic + no entity_pos → every session passes.
        let targets = select_targets_for_policy(
            None, "td/all/res", None, &sessions, &|_| None,
        );
        assert_eq!(targets.len(), 2);
    }

    // ===== P6 two-tier batch window tests =====
    //
    // Rather than spin up the entire runtime, we extract the algorithm into a
    // small helper that takes a crossbeam Receiver and returns the collected
    // batch. That lets us feed synthetic (timed) inputs and assert flush
    // timing. KEEP IN SYNC with the real broadcast loop above.
    use crossbeam_channel::{bounded, RecvTimeoutError};
    use std::time::{Duration, Instant};

    fn collect_batch(
        rx: &crossbeam_channel::Receiver<OutboundMsg>,
        min_batch: Duration,
        max_batch: Duration,
    ) -> Option<Vec<OutboundMsg>> {
        let first = rx.recv().ok()?;
        let first_urg = outbound_urgency(&first);
        let mut batch = vec![first];
        let start = Instant::now();
        if first_urg == Urgency::Urgent {
            while let Ok(m) = rx.try_recv() {
                batch.push(m);
            }
            return Some(batch);
        }
        loop {
            let now = Instant::now();
            let elapsed = now.duration_since(start);
            if elapsed >= max_batch {
                break;
            }
            let timeout = max_batch - elapsed;
            match rx.recv_timeout(timeout) {
                Ok(m) => {
                    let is_lockstep = m.lockstep_frame.is_some();
                    let is_urg = outbound_urgency(&m) == Urgency::Urgent;
                    batch.push(m);
                    if is_lockstep || (is_urg && start.elapsed() >= min_batch) {
                        break;
                    }
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        Some(batch)
    }

    #[test]
    fn urgent_first_flushes_immediately() {
        // First msg is Urgent → algorithm drains ready messages and returns.
        // Must complete well under MIN_BATCH even though MIN_BATCH is 10ms.
        let (tx, rx) = bounded::<OutboundMsg>(32);
        tx.send(make("creep", "D", json!({ "id": 42 }))).unwrap();
        tx.send(make("creep", "H", json!({ "id": 42, "hp": 10.0 }))).unwrap();
        let t0 = Instant::now();
        let batch = collect_batch(&rx, Duration::from_millis(10), Duration::from_millis(33)).unwrap();
        let elapsed = t0.elapsed();
        assert!(elapsed < Duration::from_millis(5), "urgent flush took {:?}", elapsed);
        // Both messages made it in (the Normal got piggy-backed via try_recv).
        assert_eq!(batch.len(), 2);
    }

    #[test]
    fn lockstep_frame_flushes_immediately() {
        use crate::lockstep::{LockstepFrame, TickBatch};

        let (tx, rx) = bounded::<OutboundMsg>(32);
        tx.send(OutboundMsg::lockstep_frame(LockstepFrame::TickBatch(TickBatch {
            tick: 1,
            inputs: Vec::new(),
            server_events: Vec::new(),
        }))).unwrap();
        let t0 = Instant::now();
        let batch = collect_batch(&rx, Duration::from_millis(10), Duration::from_millis(33)).unwrap();
        let elapsed = t0.elapsed();
        assert!(elapsed < Duration::from_millis(5), "lockstep flush took {:?}", elapsed);
        assert_eq!(batch.len(), 1);
    }

    #[test]
    fn lockstep_frame_short_circuits_normal_batch() {
        use crate::lockstep::{LockstepFrame, TickBatch};

        let (tx, rx) = bounded::<OutboundMsg>(32);
        tx.send(make("creep", "H", json!({ "id": 42, "hp": 10.0 }))).unwrap();
        tx.send(OutboundMsg::lockstep_frame(LockstepFrame::TickBatch(TickBatch {
            tick: 1,
            inputs: Vec::new(),
            server_events: Vec::new(),
        }))).unwrap();
        let t0 = Instant::now();
        let batch = collect_batch(&rx, Duration::from_millis(10), Duration::from_millis(33)).unwrap();
        let elapsed = t0.elapsed();
        assert!(elapsed < Duration::from_millis(5), "lockstep did not short-circuit: {:?}", elapsed);
        assert_eq!(batch.len(), 2);
    }

    #[test]
    fn normal_first_waits_at_least_min_batch() {
        // When the head is Normal and no Urgent arrives, we hold up to
        // MAX_BATCH. With an empty channel after the first msg we hit the
        // timeout at exactly MAX_BATCH (modulo OS scheduler slop).
        let (tx, rx) = bounded::<OutboundMsg>(32);
        tx.send(make("creep", "H", json!({ "id": 42, "hp": 10.0 }))).unwrap();
        let t0 = Instant::now();
        let batch = collect_batch(&rx, Duration::from_millis(10), Duration::from_millis(33)).unwrap();
        let elapsed = t0.elapsed();
        // Expect ≥ MAX_BATCH (within a generous tolerance — Windows scheduler
        // quantum is ~15ms so the lower bound is what we really care about).
        assert!(elapsed >= Duration::from_millis(25), "normal flush fired too early: {:?}", elapsed);
        assert_eq!(batch.len(), 1);
    }

    #[test]
    fn urgent_after_min_batch_short_circuits() {
        // Normal head, then after MIN_BATCH an Urgent arrives → should flush
        // before MAX_BATCH. Drive a separate thread to deliver the urgent
        // message ~15ms in.
        let (tx, rx) = bounded::<OutboundMsg>(32);
        tx.send(make("creep", "H", json!({ "id": 42, "hp": 10.0 }))).unwrap();
        let tx2 = tx.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(15));
            tx2.send(make("creep", "D", json!({ "id": 42 }))).unwrap();
        });
        let t0 = Instant::now();
        let batch = collect_batch(&rx, Duration::from_millis(10), Duration::from_millis(33)).unwrap();
        let elapsed = t0.elapsed();
        // Must flush after the Urgent arrives (~15ms) but before MAX_BATCH (33ms).
        assert!(elapsed < Duration::from_millis(30), "didn't short-circuit on Urgent: {:?}", elapsed);
        assert!(elapsed >= Duration::from_millis(14), "flushed before MIN_BATCH: {:?}", elapsed);
        assert_eq!(batch.len(), 2);
    }

    #[test]
    fn urgent_before_min_batch_still_holds() {
        // If an Urgent arrives BEFORE MIN_BATCH, we keep batching until at
        // least MIN_BATCH — the Urgent is only a short-circuit signal once
        // we've already amortised the batch cost.
        let (tx, rx) = bounded::<OutboundMsg>(32);
        tx.send(make("creep", "H", json!({ "id": 42, "hp": 10.0 }))).unwrap();
        let tx2 = tx.clone();
        std::thread::spawn(move || {
            // Urgent arrives at ~3ms — well inside MIN_BATCH.
            std::thread::sleep(Duration::from_millis(3));
            tx2.send(make("creep", "D", json!({ "id": 42 }))).unwrap();
        });
        let t0 = Instant::now();
        let batch = collect_batch(&rx, Duration::from_millis(10), Duration::from_millis(33)).unwrap();
        let elapsed = t0.elapsed();
        // Should keep batching past MIN_BATCH even though Urgent was seen
        // (then hit MAX_BATCH since nothing else arrives).
        assert!(elapsed >= Duration::from_millis(25), "flushed on early Urgent: {:?}", elapsed);
        assert_eq!(batch.len(), 2);
    }

    #[test]
    fn dedupe_preserves_order_for_mixed_traffic() {
        // Non-dedupable keeps its slot; dedupable keeps its FIRST-occurrence slot.
        let batch = vec![
            make("creep", "H", json!({ "id": 42, "hp": 100.0 })), // slot 0
            make("game", "lives", json!({ "lives": 3 })),         // slot 1 (pass-through)
            make("creep", "H", json!({ "id": 42, "hp": 50.0 })),  // dedupes into slot 0
            make("creep", "M", json!({ "id": 42, "x": 1.0, "y": 2.0 })), // slot 2
        ];
        let out = dedupe_batch(batch);
        assert_eq!(out.len(), 3);
        let first: serde_json::Value = serde_json::from_str(&out[0].msg).unwrap();
        assert_eq!(first["t"].as_str(), Some("creep"));
        assert_eq!(first["a"].as_str(), Some("H"));
        assert_eq!(first["d"]["hp"].as_f64(), Some(50.0)); // latest value
        let second: serde_json::Value = serde_json::from_str(&out[1].msg).unwrap();
        assert_eq!(second["t"].as_str(), Some("game"));
        let third: serde_json::Value = serde_json::from_str(&out[2].msg).unwrap();
        assert_eq!(third["t"].as_str(), Some("creep"));
        assert_eq!(third["a"].as_str(), Some("M"));
    }

    // ===== Phase 2 lockstep round-trip tests =====

    #[test]
    fn lockstep_input_submit_decode_roundtrip() {
        // Encode an InputSubmit, then decode + assert fields. Smoke test for
        // the per-client read loop's `InputSubmit::decode(payload.as_slice())`.
        let original = InputSubmit {
            player_id: 42,
            target_tick: 1234,
            input: Some(PlayerInput {
                action: Some(player_input::Action::NoOp(NoOp {})),
            }),
            input_id: 99,
        };
        let bytes = original.encode_to_vec();
        let decoded = InputSubmit::decode(bytes.as_slice()).expect("decode");
        assert_eq!(decoded.player_id, 42);
        assert_eq!(decoded.target_tick, 1234);
        assert_eq!(decoded.input_id, 99);
        assert!(decoded.input.is_some());
    }

    #[test]
    fn lockstep_join_request_role_mapping() {
        // Both ROLE_PLAYER (1) and ROLE_OBSERVER (2) should round-trip; the
        // server's match arm (in handle_client) treats unknown ints as Player.
        let player = JoinRequest {
            player_name: "alice".into(),
            role: JoinRole::RolePlayer as i32,
        };
        let observer = JoinRequest {
            player_name: "bob".into(),
            role: JoinRole::RoleObserver as i32,
        };
        let p_bytes = player.encode_to_vec();
        let o_bytes = observer.encode_to_vec();
        let p_dec = JoinRequest::decode(p_bytes.as_slice()).unwrap();
        let o_dec = JoinRequest::decode(o_bytes.as_slice()).unwrap();
        assert_eq!(p_dec.role, JoinRole::RolePlayer as i32);
        assert_eq!(o_dec.role, JoinRole::RoleObserver as i32);
        assert_eq!(p_dec.player_name, "alice");
        assert_eq!(o_dec.player_name, "bob");
    }

    #[test]
    fn lockstep_build_framed_bytes_layout() {
        // build_framed_bytes must produce [tag][4B BE len][payload] matching
        // the existing write_framed wire format. Below threshold = no compression.
        let payload = b"hello".to_vec();
        let frame = build_framed_bytes(TAG_TICK_BATCH, &payload);
        assert_eq!(frame[0], TAG_TICK_BATCH);
        assert_eq!(frame[0] & COMPRESSION_FLAG, 0);
        let len = u32::from_be_bytes(frame[1..5].try_into().unwrap()) as usize;
        assert_eq!(len, payload.len());
        assert_eq!(&frame[5..], payload.as_slice());
    }

    #[test]
    fn lockstep_build_framed_bytes_compresses_large_redundant() {
        // 1KB redundant payload → COMPRESSION_FLAG set + total wire bytes drop.
        let payload = vec![0xABu8; 1000];
        let frame = build_framed_bytes(TAG_TICK_BATCH, &payload);
        assert_eq!(frame[0] & COMPRESSION_FLAG, COMPRESSION_FLAG);
        assert_eq!(frame[0] & 0x7F, TAG_TICK_BATCH);
        assert!(frame.len() < 500, "expected compressed frame < 500B, got {}", frame.len());
    }
}
