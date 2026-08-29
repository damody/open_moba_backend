#![cfg(feature = "erps-game-server")]
use erps_client::{Client, ConnectOptions, Event, QueueMode};
use omobab::erps::{
    ErpsGameServerClient, ErpsServerConfig, GameInstanceAdapter, InstanceReport, LaunchAssignment,
    LaunchReady, MatchCompletion,
};
use parking_lot::Mutex;
use std::{sync::Arc, time::Duration};
use tokio_stream::StreamExt;
#[derive(Default)]
struct Adapter {
    instances: Mutex<Vec<InstanceReport>>,
    completed: Mutex<Vec<MatchCompletion>>,
}
impl GameInstanceAdapter for Adapter {
    fn launch(&self, v: LaunchAssignment) -> Result<LaunchReady, String> {
        self.instances.lock().push(InstanceReport {
            match_id: v.match_id,
            state: "running".into(),
            reserved_cost: v.reserved_cost,
            endpoint: "127.0.0.1:7100".into(),
            connection_token: "omb-ready-token".into(),
        });
        Ok(LaunchReady {
            endpoint: "127.0.0.1:7100".into(),
            connection_token: "omb-ready-token".into(),
        })
    }
    fn snapshot(&self) -> Vec<InstanceReport> {
        self.instances.lock().clone()
    }
    fn poll_completed_result(&self) -> Option<MatchCompletion> {
        let mut completed = self.completed.lock();
        (!completed.is_empty()).then(|| completed.remove(0))
    }
}
async fn proposal(stream: &mut erps_client::EventStream) -> String {
    loop {
        if let Event::Proposal { proposal_id, .. } = stream.next().await.unwrap().unwrap() {
            return proposal_id;
        }
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn adapter_registers_launches_and_reports_ready() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let (mut stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    let mut server_config = erps::ErpsConfig::default();
    server_config.allow_development_plaintext = true;
    tokio::spawn(async move {
        erps::grpc::serve(addr, server_config, async {
            let _ = stop_rx.await;
        })
        .await
        .unwrap()
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    let endpoint = format!("http://{addr}");
    let adapter = Arc::new(Adapter::default());
    let task = tokio::spawn(
        ErpsGameServerClient::new(
            ErpsServerConfig {
                endpoint: endpoint.clone(),
                tls_domain: None,
                tls_ca_pem: None,
                auth_token: "server".into(),
                server_id: uuid::Uuid::new_v4().to_string(),
                generation: 1,
                game_endpoint: "127.0.0.1:7100".into(),
                region: "tw".into(),
                server_class: String::new(),
                capacity_total: 100,
                max_instances: 100,
                mode_costs: vec![(erps_proto::v1::QueueMode::OneVOne as i32, 1)],
                heartbeat: Duration::from_millis(20),
            },
            adapter.clone(),
        )
        .run(),
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut a = Client::connect(ConnectOptions::plaintext_loopback(&endpoint, "a"))
        .await
        .unwrap();
    let mut b = Client::connect(ConnectOptions::plaintext_loopback(&endpoint, "b"))
        .await
        .unwrap();
    let pa = a.create_party("甲1").await.unwrap();
    let pb = b.create_party("乙2").await.unwrap();
    let mut ae = a.events().await.unwrap();
    let mut be = b.events().await.unwrap();
    a.enqueue(
        pa.entity_id,
        pa.revision,
        QueueMode::OneVsOne,
        ["tw".into()],
    )
    .await
    .unwrap();
    b.enqueue(
        pb.entity_id,
        pb.revision,
        QueueMode::OneVsOne,
        ["tw".into()],
    )
    .await
    .unwrap();
    let p1 = proposal(&mut ae).await;
    let p2 = proposal(&mut be).await;
    a.accept_match(p1).await.unwrap();
    b.accept_match(p2).await.unwrap();
    let (match_id, teams) = loop {
        if let Event::Matched {
            match_id,
            teams,
            endpoint,
            connection_token,
            ..
        } = tokio::time::timeout(Duration::from_secs(3), ae.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
        {
            assert_eq!(endpoint, "127.0.0.1:7100");
            assert_eq!(connection_token, "omb-ready-token");
            break (match_id, teams);
        }
    };
    assert_eq!(adapter.snapshot().len(), 1);
    adapter.instances.lock()[0].state = "finished".into();
    adapter.completed.lock().push(MatchCompletion {
        match_id,
        placements: teams
            .into_iter()
            .enumerate()
            .flat_map(|(team, players)| {
                players
                    .into_iter()
                    .map(move |player| (player, team as u32 + 1))
            })
            .collect(),
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if a.get_state().await.unwrap().match_id.is_none() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    task.abort();
    let _ = stop_tx.send(());
}
