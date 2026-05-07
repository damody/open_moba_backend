use crossbeam_channel::{bounded, Sender, Receiver};
use failure::Error;
use log::*;
use std::thread;
use tonic::{transport::Server, Request, Response, Status};
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use async_stream::stream;

use super::types::{InboundMsg, OutboundMsg, TransportHandle, QueryRequest, QueryResponse, ViewportMsg};

// 包含生成的原始程式碼
pub mod game_proto {
    tonic::include_proto!("game");
}

use game_proto::game_service_server::{GameService, GameServiceServer};
use game_proto::*;

/// gRPC服務實現
pub struct GameServiceImpl {
    /// 將入站訊息（來自玩家）傳送到遊戲邏輯的通道
    in_tx: Sender<InboundMsg>,
    /// 出站遊戲事件轉播頻道
    event_tx: broadcast::Sender<OutboundMsg>,
    /// 將查詢請求傳送到遊戲循環的通道
    query_tx: Sender<QueryRequest>,
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
                        // 解析主題以檢查此事件是針對該玩家還是廣播
                        let is_broadcast = msg.topic.contains("/all/");
                        let is_for_player = msg.topic.contains(&format!("/{}/", player_name));
                        if is_broadcast || is_for_player || player_name.is_empty() {
                            // P9：gRPC 路徑不攜帶類型化的 prost 變體
                            // （broadcast::Sender<OutboundMsg> 只能看到 JSON 訊息
                            // 透過 OutboundMsg::new_s* 建置 - `typed` 欄位是
                            // 門控在 feature="kcp" 後面）。所以我們包裝 JSON
                            // 進入 LegacyJson 變體。
                            let (msg_type, action, data_bytes) = if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&msg.msg) {
                                let t = parsed.get("t").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                let a = parsed.get("a").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                let d = parsed.get("d").map(|v| v.to_string().into_bytes()).unwrap_or_default();
                                (t, a, d)
                            } else {
                                ("".to_string(), "".to_string(), msg.msg.as_bytes().to_vec())
                            };

                            yield Ok(GameEvent {
                                sequence: 0,
                                payload: Some(game_event::Payload::LegacyJson(LegacyJson {
                                    msg_type,
                                    action,
                                    data_json: data_bytes,
                                })),
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
        // 現在，迴響一下。稍後可以連接到 test_commands 處理程序。
        Ok(Response::new(TestCommandResponse {
            response_json: format!("{{\"echo\": {}}}", cmd_json),
        }))
    }

    async fn query_game_state(
        &self,
        request: Request<GameStateRequest>,
    ) -> Result<Response<GameStateResponse>, Status> {
        let req = request.into_inner();
        let (resp_tx, resp_rx) = tokio::sync::oneshot::channel();

        let query = QueryRequest {
            query_type: req.query_type,
            player_name: req.player_name,
            response_tx: resp_tx,
        };

        self.query_tx
            .send(query)
            .map_err(|e| Status::internal(format!("Failed to send query: {}", e)))?;

        let response = resp_rx
            .await
            .map_err(|e| Status::internal(format!("Failed to receive query response: {}", e)))?;

        Ok(Response::new(GameStateResponse {
            success: response.success,
            error: response.error,
            data_json: response.data_json,
        }))
    }
}

/// 啟動 gRPC 傳輸層。
///
/// 傳回一個“TransportHandle”，其“tx”提供出站訊息
/// 其“rx”產生入站玩家命令。
pub async fn start(
    server_addr: String,
    server_port: String,
) -> Result<TransportHandle, Error> {
    let (out_tx, out_rx): (Sender<OutboundMsg>, Receiver<OutboundMsg>) = bounded(10000);
    let (in_tx, in_rx): (Sender<InboundMsg>, Receiver<InboundMsg>) = bounded(10000);

    // 用於將出站訊息扇出到所有訂閱用戶端的廣播通道
    let (event_tx, _) = broadcast::channel::<OutboundMsg>(10000);
    let event_tx_clone = event_tx.clone();

    // 後台執行緒：從out_rx讀取並廣播給所有訂閱者
    thread::spawn(move || {
        loop {
            match out_rx.recv() {
                Ok(msg) => {
                    // 向所有連接的 gRPC 串流訂閱者廣播
                    let _ = event_tx_clone.send(msg);
                }
                Err(_) => {
                    info!("Outbound channel closed, stopping gRPC broadcaster");
                    break;
                }
            }
        }
    });

    let (query_tx, query_rx): (Sender<QueryRequest>, Receiver<QueryRequest>) = bounded(100);

    let service = GameServiceImpl {
        in_tx,
        event_tx,
        query_tx,
    };

    // 將主機名稱解析為可綁定的 SocketAddr（例如“localhost”→“0.0.0.0”）
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

    // gRPC 不實現視口更新；提供始終為空的通道
    // 因此 State API 與 KCP 傳輸保持一致。
    let (_viewport_tx, viewport_rx): (Sender<ViewportMsg>, Receiver<ViewportMsg>) = bounded(1);
    drop(_viewport_tx);

    Ok(TransportHandle {
        tx: out_tx,
        rx: in_rx,
        query_rx,
        viewport_rx,
    })
}
