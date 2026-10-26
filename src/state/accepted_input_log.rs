//! Replayable authority input record (debug tooling, E086).
//!
//! Every tick's accepted inputs exactly as the gameplay phases consume them
//! (humans and server role bots alike, after fog filtering, shop-undo pinning
//! and item-use sealing), one JSONL line per tick that has inputs:
//!   {"kind":"header","version":1,"step_fps":60,"seed":..}
//!   {"tick":<ECS Tick before the step>,"local_tick":..,"elapsed_raw":<match elapsed before the step>,
//!    "inputs":[[player,"<hex prost PlayerInput>"],..]}
//! A soak replays the file against the match's role plan with both team replicas
//! (`SOAK_ACCEPTED_INPUTS`), so any live observer mismatch reproduces offline.
//! Enabled by `OMOBA_ACCEPTED_INPUTS=<path>`; off when unset or empty.
use std::io::Write;

pub(crate) struct AcceptedInputLog {
    out: std::io::BufWriter<std::fs::File>,
}

impl AcceptedInputLog {
    pub(crate) fn from_env(step_fps: u32, seed: u64) -> Option<Self> {
        let path = std::env::var_os("OMOBA_ACCEPTED_INPUTS").filter(|path| !path.is_empty())?;
        let file = match std::fs::File::create(&path) {
            Ok(file) => file,
            Err(error) => {
                log::warn!("accepted input log disabled: cannot create {:?}: {error}", path);
                return None;
            }
        };
        let mut log = Self { out: std::io::BufWriter::new(file) };
        log.write_line(&serde_json::json!({"kind": "header", "version": 1, "step_fps": step_fps, "seed": seed}));
        log::info!("accepted input log: {:?}", path);
        Some(log)
    }

    /// `elapsed_raw`: MobaMatch elapsed time before this step, the replay key (a
    /// soak world and the server start their ECS tick at different moments).
    pub(crate) fn record(&mut self, tick: u64, local_tick: u64, elapsed_raw: Option<i64>, inputs: &[(u32, crate::lockstep::PlayerInput)]) {
        if inputs.is_empty() { return; }
        let rows: Vec<_> = inputs.iter()
            .map(|(player, input)| serde_json::json!([player, hex::encode(prost::Message::encode_to_vec(input))]))
            .collect();
        self.write_line(&serde_json::json!({"tick": tick, "local_tick": local_tick, "elapsed_raw": elapsed_raw, "inputs": rows}));
    }

    /// One line, flushed: a killed server still leaves every completed tick.
    fn write_line(&mut self, value: &serde_json::Value) {
        if writeln!(self.out, "{value}").and_then(|_| self.out.flush()).is_err() {
            log::warn!("accepted input log write failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_then_one_line_per_tick_with_decodable_inputs() {
        let dir = std::env::temp_dir().join(format!("omoba-accepted-inputs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("accepted-inputs.jsonl");
        let mut log = AcceptedInputLog { out: std::io::BufWriter::new(std::fs::File::create(&file).unwrap()) };
        log.write_line(&serde_json::json!({"kind": "header", "version": 1, "step_fps": 60, "seed": 7}));
        let input = crate::lockstep::PlayerInput { action: Some(crate::lockstep::PlayerInputEnum::MoveTo(
            crate::lockstep::MoveTo { target: Some(omoba_core::game_proto::Vec2I { x: 1024, y: -2048 }), queued: true })) };
        log.record(70, 12, Some(17 * 1024), &[]);
        log.record(71, 13, Some(18 * 1024), &[(6, input.clone())]);
        drop(log);
        let text = std::fs::read_to_string(&file).unwrap();
        let lines: Vec<serde_json::Value> = text.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
        assert_eq!(lines.len(), 2, "an empty tick writes nothing");
        assert_eq!(lines[0]["kind"], "header");
        assert_eq!((lines[1]["tick"].as_u64(), lines[1]["elapsed_raw"].as_i64()), (Some(71), Some(18 * 1024)));
        let row = &lines[1]["inputs"][0];
        assert_eq!(row[0], 6);
        let bytes = hex::decode(row[1].as_str().unwrap()).unwrap();
        assert_eq!(<crate::lockstep::PlayerInput as prost::Message>::decode(bytes.as_slice()).unwrap(), input);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
