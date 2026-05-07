// 該檔案是由 prost-build @產生的。
/// 對應PlayerData / InboundMsg
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct PlayerCommand {
    #[prost(string, tag = "1")]
    pub player_name: ::prost::alloc::string::String,
    /// “t”字段
    #[prost(string, tag = "2")]
    pub msg_type: ::prost::alloc::string::String,
    /// “a”字段
    #[prost(string, tag = "3")]
    pub action: ::prost::alloc::string::String,
    /// JSON 位元組形式的「d」字段
    #[prost(bytes = "vec", tag = "4")]
    pub data_json: ::prost::alloc::vec::Vec<u8>,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct CommandAck {
    #[prost(bool, tag = "1")]
    pub ok: bool,
    #[prost(string, tag = "2")]
    pub message: ::prost::alloc::string::String,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SubscribeRequest {
    #[prost(string, tag = "1")]
    pub player_name: ::prost::alloc::string::String,
}
/// 對應MqttMsg / OutboundMsg
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GameEvent {
    #[prost(string, tag = "1")]
    pub topic: ::prost::alloc::string::String,
    /// “t”字段
    #[prost(string, tag = "2")]
    pub msg_type: ::prost::alloc::string::String,
    /// “a”字段
    #[prost(string, tag = "3")]
    pub action: ::prost::alloc::string::String,
    /// JSON 位元組形式的「d」字段
    #[prost(bytes = "vec", tag = "4")]
    pub data_json: ::prost::alloc::vec::Vec<u8>,
    #[prost(uint64, tag = "5")]
    pub timestamp_ms: u64,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct TestCommandRequest {
    #[prost(string, tag = "1")]
    pub command_json: ::prost::alloc::string::String,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct TestCommandResponse {
    #[prost(string, tag = "1")]
    pub response_json: ::prost::alloc::string::String,
}
/// 產生的伺服器實作。
pub mod game_service_server {
    #![allow(
        unused_variables,
        dead_code,
        missing_docs,
        clippy::wildcard_imports,
        clippy::let_unit_value,
    )]
    use tonic::codegen::*;
    /// 產生的特徵包含應該實作以與 GameServiceServer 一起使用的 gRPC 方法。
    #[async_trait]
    pub trait GameService: std::marker::Send + std::marker::Sync + 'static {
        /// 玩家向遊戲伺服器發送命令
        async fn send_command(
            &self,
            request: tonic::Request<super::PlayerCommand>,
        ) -> std::result::Result<tonic::Response<super::CommandAck>, tonic::Status>;
        /// SubscribeEvents 方法的伺服器流回應類型。
        type SubscribeEventsStream: tonic::codegen::tokio_stream::Stream<
                Item = std::result::Result<super::GameEvent, tonic::Status>,
            >
            + std::marker::Send
            + 'static;
        /// 訂閱遊戲事件（伺服器串流）
        async fn subscribe_events(
            &self,
            request: tonic::Request<super::SubscribeRequest>,
        ) -> std::result::Result<
            tonic::Response<Self::SubscribeEventsStream>,
            tonic::Status,
        >;
        /// 自動化測試的測試接口
        async fn test_command(
            &self,
            request: tonic::Request<super::TestCommandRequest>,
        ) -> std::result::Result<
            tonic::Response<super::TestCommandResponse>,
            tonic::Status,
        >;
    }
    #[derive(Debug)]
    pub struct GameServiceServer<T> {
        inner: Arc<T>,
        accept_compression_encodings: EnabledCompressionEncodings,
        send_compression_encodings: EnabledCompressionEncodings,
        max_decoding_message_size: Option<usize>,
        max_encoding_message_size: Option<usize>,
    }
    impl<T> GameServiceServer<T> {
        pub fn new(inner: T) -> Self {
            Self::from_arc(Arc::new(inner))
        }
        pub fn from_arc(inner: Arc<T>) -> Self {
            Self {
                inner,
                accept_compression_encodings: Default::default(),
                send_compression_encodings: Default::default(),
                max_decoding_message_size: None,
                max_encoding_message_size: None,
            }
        }
        pub fn with_interceptor<F>(
            inner: T,
            interceptor: F,
        ) -> InterceptedService<Self, F>
        where
            F: tonic::service::Interceptor,
        {
            InterceptedService::new(Self::new(inner), interceptor)
        }
        /// 啟用使用給定編碼的解壓縮請求。
        #[must_use]
        pub fn accept_compressed(mut self, encoding: CompressionEncoding) -> Self {
            self.accept_compression_encodings.enable(encoding);
            self
        }
        /// 如果客戶端支持，則使用給定的編碼壓縮回應。
        #[must_use]
        pub fn send_compressed(mut self, encoding: CompressionEncoding) -> Self {
            self.send_compression_encodings.enable(encoding);
            self
        }
        /// 限制解碼訊息的最大大小。
        ///
        /// 預設值：`4MB`
        #[must_use]
        pub fn max_decoding_message_size(mut self, limit: usize) -> Self {
            self.max_decoding_message_size = Some(limit);
            self
        }
        /// 限制編碼訊息的最大大小。
        ///
        /// 預設值：`usize::MAX`
        #[must_use]
        pub fn max_encoding_message_size(mut self, limit: usize) -> Self {
            self.max_encoding_message_size = Some(limit);
            self
        }
    }
    impl<T, B> tonic::codegen::Service<http::Request<B>> for GameServiceServer<T>
    where
        T: GameService,
        B: Body + std::marker::Send + 'static,
        B::Error: Into<StdError> + std::marker::Send + 'static,
    {
        type Response = http::Response<tonic::body::BoxBody>;
        type Error = std::convert::Infallible;
        type Future = BoxFuture<Self::Response, Self::Error>;
        fn poll_ready(
            &mut self,
            _cx: &mut Context<'_>,
        ) -> Poll<std::result::Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
        fn call(&mut self, req: http::Request<B>) -> Self::Future {
            match req.uri().path() {
                "/game.GameService/SendCommand" => {
                    #[allow(non_camel_case_types)]
                    struct SendCommandSvc<T: GameService>(pub Arc<T>);
                    impl<
                        T: GameService,
                    > tonic::server::UnaryService<super::PlayerCommand>
                    for SendCommandSvc<T> {
                        type Response = super::CommandAck;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::PlayerCommand>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as GameService>::send_command(&inner, request).await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = SendCommandSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/game.GameService/SubscribeEvents" => {
                    #[allow(non_camel_case_types)]
                    struct SubscribeEventsSvc<T: GameService>(pub Arc<T>);
                    impl<
                        T: GameService,
                    > tonic::server::ServerStreamingService<super::SubscribeRequest>
                    for SubscribeEventsSvc<T> {
                        type Response = super::GameEvent;
                        type ResponseStream = T::SubscribeEventsStream;
                        type Future = BoxFuture<
                            tonic::Response<Self::ResponseStream>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::SubscribeRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as GameService>::subscribe_events(&inner, request).await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = SubscribeEventsSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.server_streaming(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/game.GameService/TestCommand" => {
                    #[allow(non_camel_case_types)]
                    struct TestCommandSvc<T: GameService>(pub Arc<T>);
                    impl<
                        T: GameService,
                    > tonic::server::UnaryService<super::TestCommandRequest>
                    for TestCommandSvc<T> {
                        type Response = super::TestCommandResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::TestCommandRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as GameService>::test_command(&inner, request).await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = TestCommandSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                _ => {
                    Box::pin(async move {
                        let mut response = http::Response::new(empty_body());
                        let headers = response.headers_mut();
                        headers
                            .insert(
                                tonic::Status::GRPC_STATUS,
                                (tonic::Code::Unimplemented as i32).into(),
                            );
                        headers
                            .insert(
                                http::header::CONTENT_TYPE,
                                tonic::metadata::GRPC_CONTENT_TYPE,
                            );
                        Ok(response)
                    })
                }
            }
        }
    }
    impl<T> Clone for GameServiceServer<T> {
        fn clone(&self) -> Self {
            let inner = self.inner.clone();
            Self {
                inner,
                accept_compression_encodings: self.accept_compression_encodings,
                send_compression_encodings: self.send_compression_encodings,
                max_decoding_message_size: self.max_decoding_message_size,
                max_encoding_message_size: self.max_encoding_message_size,
            }
        }
    }
    /// 產生的 gRPC 服務名稱
    pub const SERVICE_NAME: &str = "game.GameService";
    impl<T> tonic::server::NamedService for GameServiceServer<T> {
        const NAME: &'static str = SERVICE_NAME;
    }
}
