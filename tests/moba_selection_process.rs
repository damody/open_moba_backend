#![cfg(feature = "kcp")]
use omoba_core::runtime::native::moba_match::bots::{BotRole, RoleBotMatchPlan, RoleBotPlayerPlan};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn selection_binary_live_stdio_flush_and_explicit_finalization() {
    let plan = RoleBotMatchPlan {
        schema_version: 1,
        map_id: "three_lane_training".into(),
        think_hz: 5,
        mana_enabled: false,
        ability_policies: vec![],
        ability_learning: vec![],
        sustain: None,
        item_builds: vec![],
        players: vec![
            RoleBotPlayerPlan {
                player_id: 7,
                team_id: 1,
                hero: "training_luminary".into(),
                role: BotRole::Top,
                lane: "top".into(),
                bot: false,
            },
            RoleBotPlayerPlan {
                player_id: 8,
                team_id: 2,
                hero: "training_luminary".into(),
                role: BotRole::Top,
                lane: "top".into(),
                bot: true,
            },
        ],
    };
    let root = std::env::temp_dir().join(format!(
        "omoba-selection-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let fixture = root.join("candidate.json");
    std::fs::write(&fixture, serde_json::to_vec(&plan).unwrap()).unwrap();
    let mut child = OwnedChild(
        Command::new(env!("CARGO_BIN_EXE_moba-config"))
            .arg("--selection-session")
            .arg(&fixture)
            .arg("7")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let output = child.0.stdout.take().unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(output).lines() {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    let receive = || -> Value {
        let line = receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("selection reply did not flush")
            .unwrap();
        serde_json::from_str(&line).expect("stdout must contain JSON only")
    };
    let initial = receive();
    assert_eq!(initial["selection"]["finalized"], false);
    assert_eq!(initial["admitted_player_id"], 7);
    assert_eq!(initial["revision_token"], "0");
    let hash = initial["selection"]["catalog_data_hash"].as_str().unwrap();
    let mut input = child.0.stdin.take().unwrap();
    for (request_id, revision, action) in [
        (1, 0, json!({"kind":"select","hero":"training_ranger"})),
        (2, 1, json!({"kind":"lock"})),
        (3, 2, json!({"kind":"finalize"})),
    ] {
        writeln!(
            input,
            "{}",
            json!({"protocol_version":1,"catalog_data_hash":hash,
            "request_id":request_id,"expected_revision":revision,"action":action})
        )
        .unwrap();
        input.flush().unwrap();
        let reply = receive();
        assert!(reply["error"].is_null(), "{reply}");
        assert_eq!(reply["request_id_token"], request_id.to_string());
        if request_id == 3 {
            assert_eq!(reply["selection"]["finalized"], true);
            assert_eq!(reply["plan"]["players"][0]["hero"], "training_ranger");
        } else {
            assert!(reply["plan"].is_null());
        }
    }
    drop(input); // EOF gracefully exits, without any implicit command.
    assert!(
        matches!(
            receiver.recv_timeout(Duration::from_secs(10)),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
        ),
        "selection did not exit on EOF"
    );
    reader.join().unwrap();
    assert!(child.0.wait().unwrap().success());
    std::fs::remove_file(fixture).unwrap();
    std::fs::remove_dir(root).unwrap();
}
