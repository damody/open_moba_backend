//! Lockstep session state — players, current tick, master seed.
//!
//! Held inside an `Arc<Mutex<LockstepState>>` shared between:
//!  - the `TickBroadcaster` task (advances `current_tick` each tick),
//!  - the kcp transport (Task 2.3) JoinRequest handler (registers players),
//!  - the game loop (reads master_seed for deterministic SimRng streams).

use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinRoleEnum {
    Player,
    Observer,
}

#[derive(Debug, Clone)]
pub struct PlayerSession {
    pub player_id: u32,
    pub player_name: String,
    pub role: JoinRoleEnum,
    /// The most recent target_tick this player has successfully submitted
    /// for. Used to detect stuck clients (Phase 3+ may pause the tick loop
    /// when a player's lag exceeds a budget).
    pub last_input_tick: u32,
}

pub struct LockstepState {
    /// Authoritative server tick counter — advanced by `TickBroadcaster`
    /// once per 16.67ms.
    pub current_tick: u32,
    /// Seed broadcast in `GameStart` — clients feed it into omoba_sim's
    /// `SimRng::from_master_*` constructors. Must match across all peers.
    pub master_seed: u64,
    pub players: BTreeMap<u32, PlayerSession>,
    pub next_player_id: u32,
}

impl LockstepState {
    pub fn new(master_seed: u64) -> Self {
        Self {
            current_tick: 0,
            master_seed,
            players: BTreeMap::new(),
            next_player_id: 1,
        }
    }

    pub fn register_player(&mut self, name: String, role: JoinRoleEnum) -> u32 {
        let id = self.next_player_id;
        self.next_player_id += 1;
        self.players.insert(
            id,
            PlayerSession {
                player_id: id,
                player_name: name,
                role,
                last_input_tick: 0,
            },
        );
        id
    }

    pub fn unregister_player(&mut self, player_id: u32) {
        self.players.remove(&player_id);
    }
}
