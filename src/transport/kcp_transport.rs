use crossbeam_channel::{bounded, Sender, Receiver};
use failure::Error;
use log::*;
use std::collections::HashMap;
use std::sync::Arc;
use std::thread;
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::sync::Mutex;

use tokio_kcp::{KcpConfig, KcpListener, KcpNoDelayConfig, KcpStream};

use super::types::{InboundMsg, OutboundMsg, TransportHandle, QueryRequest, QueryResponse, TypedOutbound, Viewport, ViewportMsg};
use super::metrics::KcpBytesCounter;

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

/// Read a framed message, returns (tag, payload bytes).
/// If COMPRESSION_FLAG is set on the wire tag, the payload is decompressed and
/// the returned tag has the flag stripped (callers see only 0x01~0x07).
/// KEEP IN SYNC with omoba-core::kcp::framing::read_framed.
async fn read_framed<R: AsyncReadExt + Unpin>(
    reader: &mut R,
) -> std::io::Result<Option<(u8, Vec<u8>)>> {
    let tag_raw = match reader.read_u8().await {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    };
    let len = reader.read_u32().await? as usize;
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    if tag_raw & COMPRESSION_FLAG != 0 {
        let base_tag = tag_raw & 0x7F;
        let decompressed = lz4_flex::block::decompress_size_prepended(&buf)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Ok(Some((base_tag, decompressed)))
    } else {
        Ok(Some((tag_raw, buf)))
    }
}

/// Per-client session: holds a sender to push outbound events
struct ClientSession {
    player_name: String,
    event_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
    viewport: Option<Viewport>,
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
            | ("hero", "stats")
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
pub async fn start(
    server_addr: String,
    server_port: String,
) -> Result<TransportHandle, Error> {
    let (out_tx, out_rx): (Sender<OutboundMsg>, Receiver<OutboundMsg>) = bounded(10000);
    let (in_tx, in_rx): (Sender<InboundMsg>, Receiver<InboundMsg>) = bounded(10000);
    let (query_tx, query_rx): (Sender<QueryRequest>, Receiver<QueryRequest>) = bounded(100);
    let (viewport_tx, viewport_rx): (Sender<ViewportMsg>, Receiver<ViewportMsg>) = bounded(1024);

    let sessions: Arc<Mutex<HashMap<String, ClientSession>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // Per-event bytes/msg counter. Shared with the broadcast thread so tests
    // and the game loop can snapshot/reset the observed wire volume.
    let counter: Arc<KcpBytesCounter> = Arc::new(KcpBytesCounter::new());

    // Background thread: read from out_rx and broadcast to all sessions
    let sessions_broadcast = sessions.clone();
    let counter_broadcast = counter.clone();
    thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            // ===== 100ms Batch Send =====
            // 把 out_rx 的訊息彙整成 100ms window 的批次，一起寫入 KCP，降低 per-message overhead。
            // Client 端協定不變：仍是一個 framed GameEvent 一幀，這邊只是把多幀一次寫入。
            use std::time::{Duration, Instant};
            const BATCH_WINDOW: Duration = Duration::from_millis(33);
            'outer: loop {
                // 等第一筆訊息（阻塞）
                let first = match out_rx.recv() {
                    Ok(m) => m,
                    Err(_) => {
                        info!("Outbound channel closed, stopping KCP broadcaster");
                        break 'outer;
                    }
                };
                let mut batch: Vec<crate::transport::OutboundMsg> = vec![first];
                let window_start = Instant::now();
                // 在 100ms 內盡量多收
                loop {
                    let elapsed = window_start.elapsed();
                    if elapsed >= BATCH_WINDOW {
                        break;
                    }
                    match out_rx.recv_timeout(BATCH_WINDOW - elapsed) {
                        Ok(m) => batch.push(m),
                        Err(crossbeam_channel::RecvTimeoutError::Timeout) => break,
                        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break 'outer,
                    }
                }

                // Collapse redundant latest-wins updates (creep.M / *.H / entity.F / creep.S / hero.stats)
                // before encoding. See `is_dedupable` for the full policy.
                let batch = dedupe_batch(batch);

                // 處理整個批次
                for msg in batch {
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

                        // P2 binary-protocol path: when `msg.typed` is Some, build
                        // the prost oneof variant and drop the JSON `data_json` so
                        // only the typed payload traverses the wire.
                        let typed_payload = msg.typed.as_ref().map(|t| match t {
                            TypedOutbound::Heartbeat(hb) => {
                                game_event::TypedPayload::Heartbeat(hb.clone())
                            }
                        });
                        let data_json = if typed_payload.is_some() { Vec::new() } else { data_bytes };

                        let event = GameEvent {
                            topic: msg.topic.clone(),
                            msg_type,
                            action,
                            data_json,
                            timestamp_ms,
                            typed_payload,
                        };

                        let payload = event.encode_to_vec();

                        // Compress payload when ≥ threshold AND the compressed
                        // bytes are strictly smaller; otherwise fall back to raw.
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

                        // Build framed bytes: tag + len + payload
                        let mut frame = Vec::with_capacity(1 + 4 + frame_payload.len());
                        frame.push(frame_tag);
                        frame.extend_from_slice(&(frame_payload.len() as u32).to_be_bytes());
                        frame.extend_from_slice(&frame_payload);

                        // Record observed wire bytes (post-compression, includes
                        // tag + length prefix).  Bucketed by (msg_type, action) so
                        // downstream analysis can slice by game event category
                        // without the hot path paying a `format!()` per message.
                        counter_broadcast.record(
                            &event.msg_type,
                            &event.action,
                            frame.len(),
                        );

                        let sessions = sessions_broadcast.lock().await;
                        let mut to_remove = Vec::new();
                        let is_per_player_topic = !msg.topic.contains("/all/") && msg.topic.starts_with("td/") && msg.topic.ends_with("/res");
                        let mut route_hits = 0u32;
                        for (id, session) in sessions.iter() {
                            // Filter by topic: broadcast or for this player
                            let is_broadcast = msg.topic.contains("/all/");
                            let is_for_player = msg.topic.contains(&format!("/{}/", session.player_name));
                            if is_broadcast || is_for_player || session.player_name.is_empty() {
                                // Viewport filtering: only check entities that have a position
                                let in_viewport = match (msg.entity_pos, &session.viewport) {
                                    (Some((x, y)), Some(vp)) => vp.contains(x, y),
                                    _ => true, // no position or no viewport → pass through
                                };
                                if in_viewport {
                                    if session.event_tx.try_send(frame.clone()).is_err() {
                                        to_remove.push(id.clone());
                                    } else {
                                        route_hits += 1;
                                    }
                                } else if is_per_player_topic {
                                    log::debug!("⚠ per-player event at {:?} blocked by vp filter for '{}'",
                                        msg.entity_pos, session.player_name);
                                }
                            }
                        }
                        if is_per_player_topic {
                            log::debug!("📡 routed per-player topic='{}' hits={} (sessions={})",
                                msg.topic, route_hits, sessions.len());
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
            let session_id = format!("kcp_{}", peer_addr);

            tokio::spawn(async move {
                if let Err(e) = handle_client(stream, session_id, sessions, in_tx, query_tx, viewport_tx).await {
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
    })
}

async fn handle_client(
    stream: KcpStream,
    session_id: String,
    sessions: Arc<Mutex<HashMap<String, ClientSession>>>,
    in_tx: Sender<InboundMsg>,
    query_tx: Sender<QueryRequest>,
    viewport_tx: Sender<ViewportMsg>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (mut reader, mut writer) = tokio::io::split(stream);

    // Per-session outbound channel (lazy — only used after SubscribeRequest)
    let mut event_rx: Option<tokio::sync::mpsc::Receiver<Vec<u8>>> = None;
    let mut subscribed = false;
    // Track the subscribed player_name so we can send a Remove on disconnect
    let mut player_name: Option<String> = None;

    // Main loop: read from client, optionally write outbound events
    loop {
        tokio::select! {
            result = read_framed(&mut reader) => {
                match result {
                    Ok(Some((tag, payload))) => {
                        match tag {
                            TAG_SUBSCRIBE_REQUEST => {
                                if let Ok(sub) = SubscribeRequest::decode(payload.as_slice()) {
                                    info!("🔌 KCP client subscribed as '{}' (session_id={})", sub.player_name, session_id);
                                    let (event_tx, rx) = tokio::sync::mpsc::channel::<Vec<u8>>(10000);
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
                // frame is already [tag + len + payload] pre-encoded
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
}
