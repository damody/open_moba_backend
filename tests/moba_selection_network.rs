#![cfg(feature = "kcp")]
use omoba_core::runtime::native::moba_match::bots::{BotRole, RoleBotMatchPlan, RoleBotPlayerPlan};
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
struct Peer {
    child: OwnedChild,
    input: Option<ChildStdin>,
    replies: mpsc::Receiver<std::io::Result<String>>,
}
impl Peer {
    fn connect(invite: &Path, player: u32) -> Self {
        let mut child = OwnedChild(
            Command::new(env!("CARGO_BIN_EXE_moba-config"))
                .args([
                    "--selection-session",
                    invite.to_str().unwrap(),
                    &player.to_string(),
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        let (sender, replies) = mpsc::channel();
        let output = child.0.stdout.take().unwrap();
        thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        let input = child.0.stdin.take();
        Self {
            child,
            input,
            replies,
        }
    }
    fn receive(&self) -> Value {
        serde_json::from_str(
            &self
                .replies
                .recv_timeout(Duration::from_secs(10))
                .expect("gateway reply timeout")
                .unwrap(),
        )
        .unwrap()
    }
    fn request(&mut self, id: u64, revision: u64, action: Value) -> Value {
        writeln!(
            self.input.as_mut().unwrap(),
            "{}",
            json!({"protocol_version":1,
            "catalog_data_hash":omoba_template_ids::CONTENT_CATALOG_DATA_HASH,
            "request_id":id,"expected_revision":revision,"action":action})
        )
        .unwrap();
        self.input.as_mut().unwrap().flush().unwrap();
        let reply = self.receive();
        assert_eq!(reply["request_id_token"], id.to_string());
        assert_eq!(reply["shared_room"], true);
        reply
    }
    fn finish(&mut self) {
        drop(self.input.take());
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = self.child.0.try_wait().unwrap() {
                assert!(status.success());
                return;
            }
            assert!(
                Instant::now() < deadline,
                "gateway did not exit on stdin EOF"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
}

fn wait_file(path: &Path, child: &mut OwnedChild) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.is_file() {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "host exited before artifact"
        );
        assert!(Instant::now() < deadline, "host artifact timeout");
        thread::sleep(Duration::from_millis(10));
    }
    // Production publishes only complete files; do not hide a partial artifact with retries.
    serde_json::from_slice::<Value>(&fs::read(path).unwrap())
        .expect("host artifact must publish atomically");
}

#[test]
fn selection_host_two_live_gateways_admission_shared_roster_and_handoff() {
    let root =
        std::env::temp_dir().join(format!("omoba-selection-network-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    let plan = RoleBotMatchPlan {
        schema_version: 1,
        map_id: "three_lane_training".into(),
        think_hz: 5,
        mana_enabled: false,
        ability_policies: vec![],
        ability_learning: vec![],
        sustain: None,
        item_builds: vec![],
        players: [(7, 1), (8, 2)]
            .into_iter()
            .map(|(player_id, team_id)| RoleBotPlayerPlan {
                player_id,
                team_id,
                hero: "training_luminary".into(),
                role: BotRole::Top,
                lane: "top".into(),
                bot: false,
            })
            .collect(),
    };
    let candidate = root.join("candidate.json");
    fs::write(&candidate, serde_json::to_vec(&plan).unwrap()).unwrap();
    let output = root.join("host");
    let mut host = OwnedChild(
        Command::new(env!("CARGO_BIN_EXE_moba-config"))
            .arg("--selection-host")
            .arg(&candidate)
            .arg(&output)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    wait_file(&output.join("ready.json"), &mut host);
    let mut first = Peer::connect(&output.join("player-7.json"), 7);
    let mut second = Peer::connect(&output.join("player-8.json"), 8);
    for (peer, id) in [(&first, 7), (&second, 8)] {
        let initial = peer.receive();
        assert_eq!(initial["admitted_player_id"], id);
        assert_eq!(initial["revision_token"], "0");
        assert_eq!(initial["shared_room"], true);
    }
    let mut bad: Value =
        serde_json::from_slice(&fs::read(output.join("player-7.json")).unwrap()).unwrap();
    bad["token"] = json!("0".repeat(64));
    let refused = root.join("refused.json");
    fs::write(&refused, serde_json::to_vec(&bad).unwrap()).unwrap();
    let rejected = Peer::connect(&refused, 7);
    assert!(
        rejected
            .replies
            .recv_timeout(Duration::from_secs(10))
            .is_err(),
        "invalid capability leaked initial catalog"
    );
    let duplicate = Peer::connect(&output.join("player-7.json"), 7);
    let duplicate_reply = duplicate.replies.recv_timeout(Duration::from_secs(10));
    assert!(
        duplicate_reply.is_err(),
        "duplicate connection received frame; existing peer status={:?}, reply_ok={}",
        first.child.0.try_wait().unwrap(),
        duplicate_reply.as_ref().is_ok_and(|line| line.is_ok())
    );
    let mismatched = Peer::connect(&output.join("player-7.json"), 8);
    assert!(
        mismatched
            .replies
            .recv_timeout(Duration::from_secs(10))
            .is_err(),
        "invitation changed bound identity"
    );
    let reply = first.request(1, 0, json!({"kind":"select","hero":"training_ranger"}));
    assert!(reply["error"].is_null());
    let stale = second.request(1, 0, json!({"kind":"read"}));
    assert!(stale["error"].is_string());
    assert_eq!(
        stale["selection"]["seats"][0]["player"]["hero"],
        "training_ranger"
    );
    assert!(first.request(2, 1, json!({"kind":"lock"}))["error"].is_null());
    assert!(
        second.request(2, 2, json!({"kind":"lock"}))["selection"]["ready"]
            .as_bool()
            .unwrap()
    );
    let final_reply = first.request(3, 3, json!({"kind":"finalize"}));
    assert!(final_reply["selection"]["finalized"].as_bool().unwrap());
    let follower = second.request(3, 3, json!({"kind":"read"}));
    assert!(follower["error"].is_string()); // Stale request still carries final authoritative state.
    assert_eq!(follower["selection"]["finalized"], true);
    assert!(follower["plan"].is_null());
    first.finish();
    second.finish();
    wait_file(&output.join("finalized-plan.json"), &mut host);
    let finalized: Value =
        serde_json::from_slice(&fs::read(output.join("finalized-plan.json")).unwrap()).unwrap();
    assert_eq!(finalized["plan"], final_reply["plan"]);
    assert_eq!(finalized["selection"], final_reply["selection"]);
    let errors = fs::read_to_string(output.join("errors.md")).unwrap();
    assert!(errors.contains("PermissionDenied"));
    assert!(!errors.contains(bad["token"].as_str().unwrap()));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = host.0.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(Instant::now() < deadline, "completed host did not retire");
        thread::sleep(Duration::from_millis(10));
    }
    // Retain only this test's temp artifacts for failure diagnosis; never delete user paths.
}
