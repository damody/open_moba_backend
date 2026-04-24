use crossbeam_channel::{bounded, Sender, Receiver};
use failure::Error;
use log::*;
use std::collections::HashMap;
use std::sync::Arc;
use std::thread;
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::sync::Mutex;

use tokio_kcp::{KcpConfig, KcpListener, KcpNoDelayConfig, KcpStream};

use super::types::{InboundMsg, OutboundMsg, TransportHandle, QueryRequest, QueryResponse, Viewport, ViewportMsg};
use super::metrics::KcpBytesCounter;

// Include the generated proto code
pub mod game_proto {
    include!(concat!(env!("OUT_DIR"), "/game.rs"));
}

use game_proto::*;
use prost::Message;

// Framing tag constants (same protocol as omoba-core)
const TAG_PLAYER_COMMAND: u8 = 0x01;
const TAG_GAME_EVENT: u8 = 0x02;
const TAG_COMMAND_ACK: u8 = 0x03;
const TAG_SUBSCRIBE_REQUEST: u8 = 0x04;
const TAG_GAME_STATE_REQUEST: u8 = 0x05;
const TAG_GAME_STATE_RESPONSE: u8 = 0x06;
const TAG_VIEWPORT_UPDATE: u8 = 0x07;

/// Write a framed message: [1 byte tag][4 bytes len (big-endian)][N bytes payload]
async fn write_framed<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    tag: u8,
    payload: &[u8],
) -> std::io::Result<()> {
    let len = payload.len() as u32;
    writer.write_u8(tag).await?;
    writer.write_u32(len).await?;
    writer.write_all(payload).await?;
    writer.flush().await?;
    Ok(())
}

/// Read a framed message, returns (tag, payload bytes).
async fn read_framed<R: AsyncReadExt + Unpin>(
    reader: &mut R,
) -> std::io::Result<Option<(u8, Vec<u8>)>> {
    let tag = match reader.read_u8().await {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    };
    let len = reader.read_u32().await? as usize;
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    Ok(Some((tag, buf)))
}

/// Per-client session: holds a sender to push outbound events
struct ClientSession {
    player_name: String,
    event_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
    viewport: Option<Viewport>,
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

                        let event = GameEvent {
                            topic: msg.topic.clone(),
                            msg_type,
                            action,
                            data_json: data_bytes,
                            timestamp_ms,
                        };

                        let payload = event.encode_to_vec();

                        // Build framed bytes: tag + len + payload
                        let mut frame = Vec::with_capacity(1 + 4 + payload.len());
                        frame.push(TAG_GAME_EVENT);
                        frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
                        frame.extend_from_slice(&payload);

                        // Record observed wire bytes (includes tag + length prefix).
                        // Event kind = "<msg_type>.<action>" so downstream analysis can
                        // bucket by game event category.
                        counter_broadcast.record(
                            &format!("{}.{}", event.msg_type, event.action),
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
