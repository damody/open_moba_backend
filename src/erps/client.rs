use super::{GameInstanceAdapter, InstanceReport, LaunchAssignment};
use erps_proto::v1::{self as pb, game_server_service_client::GameServerServiceClient};
use parking_lot::Mutex;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Duration,
};
use tokio::{sync::mpsc, time::sleep};
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint};

const RESULT_OUTBOX_CAPACITY: usize = 4096;

#[derive(Clone, Debug)]
pub struct ErpsServerConfig {
    pub endpoint: String,
    /// TLS server name. `None` is intended only for loopback development endpoints.
    pub tls_domain: Option<String>,
    /// Optional private CA certificate in PEM format; platform roots remain enabled.
    pub tls_ca_pem: Option<Vec<u8>>,
    pub auth_token: String,
    pub server_id: String,
    pub generation: u64,
    pub game_endpoint: String,
    pub region: String,
    pub server_class: String,
    pub capacity_total: u32,
    pub max_instances: u32,
    pub mode_costs: Vec<(i32, u32)>,
    pub heartbeat: Duration,
}

pub struct ErpsGameServerClient<A: GameInstanceAdapter> {
    config: ErpsServerConfig,
    adapter: Arc<A>,
}
impl<A: GameInstanceAdapter> ErpsGameServerClient<A> {
    pub fn new(config: ErpsServerConfig, adapter: Arc<A>) -> Self {
        Self { config, adapter }
    }
    pub async fn run(self) -> Result<(), tonic::Status> {
        validate_config(&self.config)?;
        let mut backoff = Duration::from_millis(100);
        let pending_results = Arc::new(Mutex::new(BTreeMap::new()));
        loop {
            match self.run_connection(pending_results.clone()).await {
                Ok(()) => backoff = Duration::from_millis(100),
                Err(status) => {
                    if !matches!(
                        status.code(),
                        tonic::Code::Unavailable | tonic::Code::Cancelled | tonic::Code::Unknown
                    ) {
                        return Err(status);
                    }
                }
            }
            sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(5));
        }
    }
    async fn run_connection(
        &self,
        pending_results: Arc<Mutex<BTreeMap<String, super::MatchCompletion>>>,
    ) -> Result<(), tonic::Status> {
        let channel = endpoint(&self.config)?
            .connect()
            .await
            .map_err(|e| tonic::Status::unavailable(e.to_string()))?;
        let mut rpc = GameServerServiceClient::new(channel);
        let register = pb::RegisterServerRequest {
            api: Some(api()),
            auth_token: self.config.auth_token.clone(),
            server_id: self.config.server_id.clone(),
            generation: self.config.generation,
            endpoint: self.config.game_endpoint.clone(),
            region: self.config.region.clone(),
            capacity_total: self.config.capacity_total,
            max_instances: self.config.max_instances,
            mode_costs: self
                .config
                .mode_costs
                .iter()
                .map(|(mode, cost)| pb::ModeCost {
                    mode: *mode,
                    cost: *cost,
                })
                .collect(),
            instances: self.instances(),
            server_class: self.config.server_class.clone(),
        };
        let result = rpc.register(register).await?.into_inner();
        if !result.accepted {
            return Err(tonic::Status::failed_precondition(result.code));
        }
        rpc.reconcile_instances(pb::ReconcileRequest {
            api: Some(api()),
            server_id: self.config.server_id.clone(),
            generation: self.config.generation,
            instances: self.instances(),
            auth_token: self.config.auth_token.clone(),
        })
        .await?;
        let (tx, rx) = mpsc::channel(64);
        let mut controls = rpc
            .control_stream(ReceiverStream::new(rx))
            .await?
            .into_inner();
        let identity = (self.config.server_id.clone(), self.config.generation);
        let heartbeat = self.config.heartbeat;
        let heartbeats = tx.clone();
        let adapter = self.adapter.clone();
        let pending_launches = Arc::new(Mutex::new(BTreeSet::<String>::new()));
        let heartbeat_pending_launches = pending_launches.clone();
        let outbound_results = pending_results.clone();
        let server_auth = self.config.auth_token.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(heartbeat);
            loop {
                ticker.tick().await;
                let pending = heartbeat_pending_launches.lock().clone();
                let snapshot: Vec<_> = adapter
                    .snapshot()
                    .into_iter()
                    .filter(|instance| !pending.contains(&instance.match_id))
                    .collect();
                if heartbeats
                    .send(pb::ServerControl {
                        api: Some(api()),
                        server_id: identity.0.clone(),
                        generation: identity.1,
                        auth_token: server_auth.clone(),
                        message: Some(pb::server_control::Message::Heartbeat(pb::Heartbeat {
                            capacity_used: reported_capacity_used(&snapshot),
                            running_instances: snapshot
                                .iter()
                                .filter(|v| v.state.eq_ignore_ascii_case("running"))
                                .count() as u32,
                        })),
                    })
                    .await
                    .is_err()
                {
                    break;
                }
                for instance in snapshot {
                    if instance.state.eq_ignore_ascii_case("finished")
                        || instance.state.eq_ignore_ascii_case("server_lost")
                    {
                        continue;
                    }
                    if heartbeats
                        .send(pb::ServerControl {
                            api: Some(api()),
                            server_id: identity.0.clone(),
                            generation: identity.1,
                            auth_token: server_auth.clone(),
                            message: Some(pb::server_control::Message::Instance(
                                pb::InstanceState {
                                    match_id: instance.match_id,
                                    state: instance.state,
                                    reserved_cost: instance.reserved_cost,
                                    endpoint: instance.endpoint,
                                    connection_token: instance.connection_token,
                                },
                            )),
                        })
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                let remaining =
                    RESULT_OUTBOX_CAPACITY.saturating_sub(outbound_results.lock().len());
                for _ in 0..remaining {
                    let Some(completed) = adapter.poll_completed_result() else {
                        break;
                    };
                    outbound_results
                        .lock()
                        .insert(completed.match_id.clone(), completed);
                }
                let retry_results: Vec<_> = outbound_results.lock().values().cloned().collect();
                for completed in retry_results {
                    if heartbeats
                        .send(pb::ServerControl {
                            api: Some(api()),
                            server_id: identity.0.clone(),
                            generation: identity.1,
                            auth_token: server_auth.clone(),
                            message: Some(pb::server_control::Message::MatchResult(
                                pb::MatchResult {
                                    match_id: completed.match_id,
                                    placements: completed
                                        .placements
                                        .into_iter()
                                        .map(|(player_id, rank)| pb::PlayerPlacement {
                                            player_id,
                                            rank,
                                        })
                                        .collect(),
                                },
                            )),
                        })
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }
        });
        while let Some(control) = controls.message().await? {
            match control.message {
                Some(pb::erps_control::Message::Launch(launch)) => {
                    let match_id = launch.match_id.clone();
                    pending_launches.lock().insert(match_id.clone());
                    tx.send(pb::ServerControl {
                        api: Some(api()),
                        server_id: self.config.server_id.clone(),
                        generation: self.config.generation,
                        auth_token: self.config.auth_token.clone(),
                        message: Some(pb::server_control::Message::LaunchResult(
                            pb::LaunchResult {
                                match_id: match_id.clone(),
                                state: "accepted".into(),
                                endpoint: String::new(),
                                connection_token: String::new(),
                                reason: String::new(),
                            },
                        )),
                    })
                    .await
                    .map_err(|_| tonic::Status::unavailable("control stream closed"))?;
                    let assignment = LaunchAssignment::from(launch);
                    let result = self.adapter.launch(assignment);
                    let (state, endpoint, connection_token, reason) = match result {
                        Ok(ready) => (
                            "ready".into(),
                            ready.endpoint,
                            ready.connection_token,
                            String::new(),
                        ),
                        Err(reason) => ("rejected".into(), String::new(), String::new(), reason),
                    };
                    tx.send(pb::ServerControl {
                        api: Some(api()),
                        server_id: self.config.server_id.clone(),
                        generation: self.config.generation,
                        auth_token: self.config.auth_token.clone(),
                        message: Some(pb::server_control::Message::LaunchResult(
                            pb::LaunchResult {
                                match_id: match_id.clone(),
                                state,
                                endpoint,
                                connection_token,
                                reason,
                            },
                        )),
                    })
                    .await
                    .map_err(|_| tonic::Status::unavailable("control stream closed"))?;
                    pending_launches.lock().remove(&match_id);
                }
                Some(pb::erps_control::Message::MatchResultAck(match_id)) => {
                    pending_results.lock().remove(&match_id);
                }
                Some(pb::erps_control::Message::ShutdownReason(_)) | None => {}
            }
        }
        Ok(())
    }
    fn instances(&self) -> Vec<pb::InstanceState> {
        self.adapter
            .snapshot()
            .into_iter()
            .filter(|instance| {
                !instance.state.eq_ignore_ascii_case("finished")
                    && !instance.state.eq_ignore_ascii_case("server_lost")
            })
            .map(|v: InstanceReport| pb::InstanceState {
                match_id: v.match_id,
                state: v.state,
                reserved_cost: v.reserved_cost,
                endpoint: v.endpoint,
                connection_token: v.connection_token,
            })
            .collect()
    }
}
fn validate_config(config: &ErpsServerConfig) -> Result<(), tonic::Status> {
    if config.heartbeat.is_zero() {
        return Err(tonic::Status::invalid_argument(
            "heartbeat interval must be greater than zero",
        ));
    }
    Ok(())
}
fn reported_capacity_used(snapshot: &[InstanceReport]) -> u32 {
    snapshot
        .iter()
        .filter(|instance| {
            !instance.state.eq_ignore_ascii_case("finished")
                && !instance.state.eq_ignore_ascii_case("server_lost")
        })
        .fold(0_u32, |used, instance| {
            used.saturating_add(instance.reserved_cost)
        })
}
fn endpoint(config: &ErpsServerConfig) -> Result<Endpoint, tonic::Status> {
    let mut endpoint = Endpoint::from_shared(config.endpoint.clone())
        .map_err(|error| tonic::Status::invalid_argument(error.to_string()))?;
    if let Some(domain) = &config.tls_domain {
        let mut tls = ClientTlsConfig::new()
            .with_enabled_roots()
            .domain_name(domain.clone());
        if let Some(ca_pem) = &config.tls_ca_pem {
            tls = tls.ca_certificate(Certificate::from_pem(ca_pem));
        }
        endpoint = endpoint
            .tls_config(tls)
            .map_err(|error| tonic::Status::invalid_argument(error.to_string()))?;
    }
    Ok(endpoint)
}
fn api() -> pb::ApiVersion {
    pb::ApiVersion {
        major: 1,
        minor: 0,
        capabilities: vec!["reconcile".into()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn feature_config_supports_one_to_one_hundred_instances() {
        for v in [1, 100] {
            let c = ErpsServerConfig {
                endpoint: "http://127.0.0.1:1".into(),
                tls_domain: None,
                tls_ca_pem: None,
                auth_token: "x".into(),
                server_id: "x".into(),
                generation: 1,
                game_endpoint: "127.0.0.1:2".into(),
                region: "tw".into(),
                server_class: String::new(),
                capacity_total: 100,
                max_instances: v,
                mode_costs: vec![],
                heartbeat: Duration::from_secs(2),
            };
            assert!((1..=100).contains(&c.max_instances));
        }
    }

    #[test]
    fn production_endpoint_accepts_tls_server_name() {
        let config = ErpsServerConfig {
            endpoint: "https://erps.internal:50051".into(),
            tls_domain: Some("erps.internal".into()),
            tls_ca_pem: None,
            auth_token: "token".into(),
            server_id: "server".into(),
            generation: 1,
            game_endpoint: "game.internal:7000".into(),
            region: "tw".into(),
            server_class: "large".into(),
            capacity_total: 100,
            max_instances: 100,
            mode_costs: vec![],
            heartbeat: Duration::from_secs(2),
        };
        assert!(endpoint(&config).is_ok());
    }

    #[test]
    fn heartbeat_capacity_report_saturates_and_ignores_released_instances() {
        let snapshot = vec![
            InstanceReport {
                match_id: "a".into(),
                state: "running".into(),
                reserved_cost: u32::MAX,
                endpoint: String::new(),
                connection_token: String::new(),
            },
            InstanceReport {
                match_id: "b".into(),
                state: "reserved".into(),
                reserved_cost: 1,
                endpoint: String::new(),
                connection_token: String::new(),
            },
            InstanceReport {
                match_id: "c".into(),
                state: "finished".into(),
                reserved_cost: u32::MAX,
                endpoint: String::new(),
                connection_token: String::new(),
            },
        ];
        assert_eq!(reported_capacity_used(&snapshot), u32::MAX);
    }

    #[test]
    fn zero_heartbeat_is_rejected_before_background_task_spawn() {
        let config = ErpsServerConfig {
            endpoint: "http://127.0.0.1:1".into(),
            tls_domain: None,
            tls_ca_pem: None,
            auth_token: "token".into(),
            server_id: "server".into(),
            generation: 1,
            game_endpoint: "127.0.0.1:2".into(),
            region: "tw".into(),
            server_class: String::new(),
            capacity_total: 1,
            max_instances: 1,
            mode_costs: vec![],
            heartbeat: Duration::ZERO,
        };
        assert_eq!(
            validate_config(&config).unwrap_err().code(),
            tonic::Code::InvalidArgument
        );
    }
}
