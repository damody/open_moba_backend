use crossbeam_channel::{bounded, Sender, Receiver};
use failure::Error;
use log::*;
use std::thread;
use tonic::{transport::Server, Request, Response, Status};
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use async_stream::stream;

use super::types::{InboundMsg, OutboundMsg, TransportHandle};

// Include the generated proto code
pub mod game_proto {
    tonic::include_proto!("game");
}

use game_proto::game_service_server::{GameService, GameServiceServer};
use game_proto::*;

/// gRPC service implementation
pub struct GameServiceImpl {
    /// Channel to send inbound messages (from player) to game logic
    in_tx: Sender<InboundMsg>,
    /// Broadcast channel for outbound game events
    event_tx: broadcast::Sender<OutboundMsg>,
}

#[tonic::async_trait]
impl GameService for GameServiceImpl {
    type SubscribeEventsStream = std::pin::Pin<
        Box<dyn tokio_stream::Stream<Item = Result<GameEvent, Status>> + Send>,
    >;

    async fn send_command(
        &self,
        request: Request<PlayerCommand>,
    ) -> Result<Response<CommandAck>, Status> {
        let cmd = request.into_inner();

        let data_json: serde_json::Value = if cmd.data_json.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&cmd.data_json)
                .map_err(|e| Status::invalid_argument(format!("Invalid JSON in data_json: {}", e)))?
        };

        let inbound = InboundMsg {
            name: cmd.player_name,
            t: cmd.msg_type,
            a: cmd.action,
            d: data_json,
        };

        self.in_tx
            .send(inbound)
            .map_err(|e| Status::internal(format!("Failed to forward command: {}", e)))?;

        Ok(Response::new(CommandAck {
            ok: true,
            message: "Command accepted".into(),
        }))
    }

    async fn subscribe_events(
        &self,
        request: Request<SubscribeRequest>,
    ) -> Result<Response<Self::SubscribeEventsStream>, Status> {
        let player_name = request.into_inner().player_name;
        let mut rx = self.event_tx.subscribe();

        let output = stream! {
            loop {
                match rx.recv().await {
                    Ok(msg) => {
                        // Parse topic to check if this event is for this player or broadcast
                        let is_broadcast = msg.topic.contains("/all/");
                        let is_for_player = msg.topic.contains(&format!("/{}/", player_name));
                        if is_broadcast || is_for_player || player_name.is_empty() {
                            // Parse the msg JSON to extract t, a, d fields
                            let (msg_type, action, data_bytes) = if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&msg.msg) {
                                let t = parsed.get("t").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                let a = parsed.get("a").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                let d = parsed.get("d").map(|v| v.to_string().into_bytes()).unwrap_or_default();
                                (t, a, d)
                            } else {
                                ("".to_string(), "".to_string(), msg.msg.as_bytes().to_vec())
                            };

                            let timestamp_ms = msg.time
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_millis() as u64)
                                .unwrap_or(0);

                            yield Ok(GameEvent {
                                topic: msg.topic,
                                msg_type,
                                action,
                                data_json: data_bytes,
                                timestamp_ms,
                            });
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("Client {} lagged behind by {} events", player_name, n);
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }
        };

        Ok(Response::new(Box::pin(output)))
    }

    async fn test_command(
        &self,
        request: Request<TestCommandRequest>,
    ) -> Result<Response<TestCommandResponse>, Status> {
        let cmd_json = request.into_inner().command_json;
        // For now, echo back. Can be connected to test_commands handler later.
        Ok(Response::new(TestCommandResponse {
            response_json: format!("{{\"echo\": {}}}", cmd_json),
        }))
    }
}

/// Start the gRPC transport layer.
///
/// Returns a `TransportHandle` whose `tx` feeds outbound messages
/// and whose `rx` yields inbound player commands.
pub async fn start(
    server_addr: String,
    server_port: String,
) -> Result<TransportHandle, Error> {
    let (out_tx, out_rx): (Sender<OutboundMsg>, Receiver<OutboundMsg>) = bounded(10000);
    let (in_tx, in_rx): (Sender<InboundMsg>, Receiver<InboundMsg>) = bounded(10000);

    // Broadcast channel for fanning out outbound messages to all subscribed clients
    let (event_tx, _) = broadcast::channel::<OutboundMsg>(10000);
    let event_tx_clone = event_tx.clone();

    // Background thread: read from out_rx and broadcast to all subscribers
    thread::spawn(move || {
        loop {
            match out_rx.recv() {
                Ok(msg) => {
                    // Broadcast to all connected gRPC stream subscribers
                    let _ = event_tx_clone.send(msg);
                }
                Err(_) => {
                    info!("Outbound channel closed, stopping gRPC broadcaster");
                    break;
                }
            }
        }
    });

    let service = GameServiceImpl {
        in_tx,
        event_tx,
    };

    // Resolve hostname to a bindable SocketAddr (e.g. "localhost" → "0.0.0.0")
    let bind_ip = match server_addr.as_str() {
        "localhost" | "127.0.0.1" => "0.0.0.0".to_string(),
        other => other.to_string(),
    };
    let addr = format!("{}:{}", bind_ip, server_port);
    let addr: std::net::SocketAddr = addr.parse().map_err(|e| failure::err_msg(format!("Invalid address '{}': {}", addr, e)))?;

    info!("Starting gRPC server on {}", addr);

    tokio::spawn(async move {
        if let Err(e) = Server::builder()
            .add_service(GameServiceServer::new(service))
            .serve(addr)
            .await
        {
            error!("gRPC server error: {}", e);
        }
    });

    Ok(TransportHandle {
        tx: out_tx,
        rx: in_rx,
    })
}
