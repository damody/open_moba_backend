//! Selection-only TCP adapter. No CONFIG singleton, script DLL or gameplay World.
use omoba_core::runtime::native::moba_match::{
    bots::RoleBotMatchPlan,
    selection::{
        service::{SelectionRoom, MAX_SELECTION_LINE_BYTES},
        HeroSelectionSession,
    },
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    ffi::OsStr,
    fs::{self, OpenOptions},
    io::{self, BufRead, BufReader, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    path::Path,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

const INVITE_SCOPE: &str = "selection-room-link";
const MAX_REPLY_BYTES: usize = 1024 * 1024;
const MAX_CONNECTIONS: usize = 10;

// Never derive Debug or include token values in logs/process arguments.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Invitation {
    schema_version: u32,
    scope: String,
    address: SocketAddr,
    admitted_player_id: u32,
    catalog_data_hash: String,
    token: String,
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn authority(message: String) -> io::Error {
    io::Error::other(message)
}
fn valid_token(token: &str) -> bool {
    token.len() == 64
        && token
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn same_token(first: &str, second: &str) -> bool {
    if !valid_token(first) || !valid_token(second) {
        return false;
    }
    first
        .bytes()
        .zip(second.bytes())
        .fold(0u8, |diff, (a, b)| diff | (a ^ b))
        == 0
}

/// Bounded framing shared by handshake and gateway. No unbounded read_line.
fn frame(input: &mut impl BufRead, limit: usize) -> io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    loop {
        let available = input.fill_buf()?;
        if available.is_empty() {
            return if line.is_empty() {
                Ok(None)
            } else {
                Err(invalid("incomplete selection frame"))
            };
        }
        let count = available
            .iter()
            .position(|b| *b == b'\n')
            .map_or(available.len(), |i| i + 1);
        if line.len() + count > limit {
            return Err(invalid("selection frame exceeds limit"));
        }
        let complete = available[count - 1] == b'\n';
        line.extend_from_slice(&available[..count]);
        input.consume(count);
        if complete {
            return Ok(Some(line));
        }
    }
}

fn write_new(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let staging = path.with_extension(format!("{}.pending", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staging)?;
    serde_json::to_writer(&mut file, value)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    drop(file);
    // Publish complete data atomically, never replacing an existing artifact.
    fs::hard_link(&staging, path)?;
    fs::remove_file(staging)
}

pub fn is_invitation(text: &str) -> io::Result<bool> {
    let value: serde_json::Value = serde_json::from_str(text)?;
    Ok(value.get("scope").and_then(|value| value.as_str()) == Some(INVITE_SCOPE))
}

type ActiveSeats = Arc<Mutex<BTreeMap<u32, TcpStream>>>;
struct SeatLease {
    active: ActiveSeats,
    player: u32,
}
impl Drop for SeatLease {
    fn drop(&mut self) {
        if let Ok(mut seats) = self.active.lock() {
            seats.remove(&self.player);
        }
    }
}

fn connection(
    room: SelectionRoom,
    tokens: Arc<BTreeMap<u32, String>>,
    active: ActiveSeats,
    mut socket: TcpStream,
) -> io::Result<()> {
    // Listener polling must not leak nonblocking mode into a worker's framed I/O.
    socket.set_nonblocking(false)?;
    socket.set_nodelay(true)?;
    socket.set_read_timeout(Some(Duration::from_secs(5)))?;
    socket.set_write_timeout(Some(Duration::from_secs(10)))?;
    let mut input = BufReader::new(socket.try_clone()?);
    let line = frame(&mut input, MAX_SELECTION_LINE_BYTES)?
        .ok_or_else(|| invalid("selection handshake missing"))?;
    let invite: Invitation =
        serde_json::from_slice(&line).map_err(|_| invalid("invalid selection handshake"))?;
    let expected = tokens.get(&invite.admitted_player_id);
    if invite.schema_version != 1
        || invite.scope != INVITE_SCOPE
        || invite.catalog_data_hash != omoba_template_ids::CONTENT_CATALOG_DATA_HASH
        || !expected.is_some_and(|token| same_token(token, &invite.token))
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "selection admission rejected",
        ));
    }
    let mut service = room.bind(invite.admitted_player_id).map_err(authority)?;
    {
        let mut seats = active
            .lock()
            .map_err(|_| invalid("selection connection registry unavailable"))?;
        if seats.contains_key(&invite.admitted_player_id) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "selection seat already connected",
            ));
        }
        seats.insert(invite.admitted_player_id, socket.try_clone()?);
    }
    let _lease = SeatLease {
        active,
        player: invite.admitted_player_id,
    };
    // Human thinking has no read deadline. The room lock is never held during I/O.
    socket.set_read_timeout(None)?;
    service.serve(&mut input, &mut socket)
}

pub fn host(plan_file: &OsStr, address: SocketAddr, output: &OsStr) -> io::Result<()> {
    if address.ip().is_unspecified() || address.ip().is_multicast() {
        return Err(invalid("selection host requires a concrete bind address"));
    }
    let plan: RoleBotMatchPlan = serde_json::from_str(&fs::read_to_string(plan_file)?)?;
    let humans: Vec<_> = plan
        .players
        .iter()
        .filter(|p| !p.bot)
        .map(|p| p.player_id)
        .collect();
    if humans.is_empty() {
        return Err(invalid("selection host requires human seats"));
    }
    let room = SelectionRoom::new(HeroSelectionSession::new(plan, 0, 60).map_err(authority)?);
    let listener = TcpListener::bind(address)?;
    let address = listener.local_addr()?;
    listener.set_nonblocking(true)?;
    let output = Path::new(output);
    fs::create_dir(output)?; // Refuse all pre-existing output, including an empty directory.
    let mut tokens = BTreeMap::new();
    for player in humans {
        let token = hex::encode(rand::random::<[u8; 32]>());
        let invitation = Invitation {
            schema_version: 1,
            scope: INVITE_SCOPE.into(),
            address,
            admitted_player_id: player,
            catalog_data_hash: omoba_template_ids::CONTENT_CATALOG_DATA_HASH.into(),
            token: token.clone(),
        };
        write_new(&output.join(format!("player-{player}.json")), &invitation)?;
        tokens.insert(player, token);
    }
    write_new(
        &output.join("ready.json"),
        &serde_json::json!({"schema_version":1,
        "scope":"shared-selection-host-ready","address":address,"tick_rate_hz":60,
        "catalog_data_hash":omoba_template_ids::CONTENT_CATALOG_DATA_HASH}),
    )?;
    let tokens = Arc::new(tokens);
    let active: ActiveSeats = Arc::new(Mutex::new(BTreeMap::new()));
    let mut workers: Vec<thread::JoinHandle<io::Result<()>>> = Vec::new();
    let mut finalized = false;
    let mut error_log = None;
    let mut error_count = 0u32;
    loop {
        let mut index = 0;
        while index < workers.len() {
            if workers[index].is_finished() {
                let worker = workers.swap_remove(index);
                match worker.join() {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        eprintln!("selection connection ended: {:?}", error.kind());
                        record_error(
                            output,
                            &mut error_log,
                            &mut error_count,
                            &format!("{:?}", error.kind()),
                        )?;
                    }
                    Err(_) => {
                        eprintln!("selection connection worker panicked");
                        record_error(output, &mut error_log, &mut error_count, "worker panic")?;
                    }
                }
            } else {
                index += 1;
            }
        }
        if !finalized {
            if let Some(plan) = room.take_finalized_plan().map_err(authority)? {
                write_new(
                    &output.join("finalized-plan.json"),
                    &serde_json::json!({"schema_version":1,
                    "scope":"shared-selection-host-finalized","tick_rate_hz":60,
                    "selection":room.snapshot().map_err(authority)?,"plan":plan}),
                )?;
                finalized = true;
            }
        }
        if finalized && workers.is_empty() {
            return Ok(());
        }
        match listener.accept() {
            Ok((socket, _)) if workers.len() < MAX_CONNECTIONS => {
                let (room, tokens, active) = (room.clone(), tokens.clone(), active.clone());
                workers.push(thread::spawn(move || {
                    connection(room, tokens, active, socket)
                }));
            }
            Ok((socket, _)) => {
                let _ = socket.shutdown(Shutdown::Both);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) => return Err(error),
        }
        thread::sleep(Duration::from_millis(10));
    }
}

pub fn proxy(text: &str, player: u32) -> io::Result<()> {
    let invite: Invitation =
        serde_json::from_str(text).map_err(|_| invalid("invalid selection invitation"))?;
    if invite.schema_version != 1
        || invite.scope != INVITE_SCOPE
        || invite.admitted_player_id != player
        || !valid_token(&invite.token)
        || invite.address.ip().is_unspecified()
        || invite.address.port() == 0
        || invite.catalog_data_hash != omoba_template_ids::CONTENT_CATALOG_DATA_HASH
    {
        return Err(invalid("selection invitation contract mismatch"));
    }
    let mut socket = TcpStream::connect_timeout(&invite.address, Duration::from_secs(5))?;
    socket.set_nonblocking(false)?;
    socket.set_nodelay(true)?;
    socket.set_read_timeout(Some(Duration::from_secs(10)))?;
    socket.set_write_timeout(Some(Duration::from_secs(10)))?;
    serde_json::to_writer(&mut socket, &invite)?;
    socket.write_all(b"\n")?;
    socket.flush()?;
    let mut network = BufReader::new(socket.try_clone()?);
    let mut output = io::stdout().lock();
    let mut input = io::stdin().lock();
    loop {
        let reply = frame(&mut network, MAX_REPLY_BYTES)?
            .ok_or_else(|| invalid("selection host closed before reply"))?;
        // Validate basic routing before exposing any data to the renderer.
        let mut value: serde_json::Value = serde_json::from_slice(&reply)?;
        if value["protocol_version"] != 1
            || value["admitted_player_id"] != player
            || value["selection"]["catalog_data_hash"] != invite.catalog_data_hash
        {
            return Err(invalid("selection host reply identity mismatch"));
        }
        value
            .as_object_mut()
            .ok_or_else(|| invalid("selection reply must be an object"))?
            .insert("shared_room".into(), serde_json::Value::Bool(true));
        let rendered = serde_json::to_vec(&value)?;
        if rendered.len() + 1 > MAX_REPLY_BYTES {
            return Err(invalid("selection reply exceeds limit"));
        }
        output.write_all(&rendered)?;
        output.write_all(b"\n")?;
        output.flush()?;
        let Some(command) = frame(&mut input, MAX_SELECTION_LINE_BYTES)? else {
            socket.shutdown(Shutdown::Both)?;
            return Ok(());
        };
        socket.write_all(&command)?;
        socket.flush()?;
    }
}

fn record_error(
    output: &Path,
    file: &mut Option<fs::File>,
    count: &mut u32,
    kind: &str,
) -> io::Result<()> {
    if *count >= 64 {
        return Ok(());
    }
    if file.is_none() {
        let mut opened = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output.join("errors.md"))?;
        opened.write_all(b"# Selection connection diagnostics\n\nBounded to 64 records; no invitation tokens or command payloads.\n\n")?;
        *file = Some(opened);
    }
    let opened = file
        .as_mut()
        .ok_or_else(|| invalid("selection error log unavailable"))?;
    writeln!(
        opened,
        "- {kind}: reject this connection; preserve room state and unrelated connections."
    )?;
    opened.flush()?;
    *count += 1;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_network_frames_are_bounded_and_complete() {
        assert!(frame(&mut io::Cursor::new(vec![b'x'; 17]), 16).is_err());
        assert!(frame(&mut io::Cursor::new(b"partial"), 16).is_err());
        let mut lines = io::Cursor::new(b"first\nsecond\n");
        assert_eq!(frame(&mut lines, 16).unwrap().unwrap(), b"first\n");
        assert_eq!(frame(&mut lines, 16).unwrap().unwrap(), b"second\n");
        assert!(frame(&mut lines, 16).unwrap().is_none());
        assert!(same_token(&"a".repeat(64), &"a".repeat(64)));
        assert!(!same_token(&"a".repeat(64), &"b".repeat(64)));
        assert!(!same_token(&"a".repeat(63), &"a".repeat(64)));
    }

    #[test]
    fn selection_network_publication_is_complete_and_never_overwrites() {
        let root = std::env::temp_dir().join(format!(
            "omoba-selection-publication-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir(&root).unwrap();
        let path = root.join("ready.json");
        let original = serde_json::json!({"scope":"test","payload":"x".repeat(65536)});
        write_new(&path, &original).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&fs::read(&path).unwrap()).unwrap(),
            original
        );
        assert!(write_new(&path, &serde_json::json!({"replacement":true})).is_err());
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&fs::read(&path).unwrap()).unwrap(),
            original
        );
        let mut log = None;
        let mut count = 0;
        for _ in 0..100 {
            record_error(&root, &mut log, &mut count, "PermissionDenied").unwrap();
        }
        assert_eq!(count, 64);
        assert_eq!(
            fs::read_to_string(root.join("errors.md"))
                .unwrap()
                .matches("- PermissionDenied:")
                .count(),
            64
        );
    }
}
