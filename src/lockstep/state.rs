//! 鎖步會話狀態－玩家、目前刻度、主種子。
//!
//! 保存在以下之間共用的 `Arc<Mutex<LockstepState>>` 內：
//! - `TickBroadcaster` 任務（每個刻度推進 `current_tick`），
//! - kcp 傳輸（任務 2.3）JoinRequest 處理程序（註冊玩家），
//! - 遊戲循環（讀取確定性 SimRng 流的 master_seed）。

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
    /// 該玩家最近成功提交的target_tick
    /// 為了。用於偵測卡住的客戶端（階段 3+ 可能會暫停滴答循環
    /// 當玩家的滯後超過預算時）。
    pub last_input_tick: u32,
}

pub struct LockstepState {
    /// 權威伺服器滴答計數器 — 由 `TickBroadcaster` 改進
    /// 每 16.67 毫秒一次。
    pub current_tick: u32,
    /// 在「GameStart」中進行種子廣播 — 用戶端將其輸入 omoba_sim
    /// `SimRng::from_master_*` 建構子。必須匹配所有同行。
    pub master_seed: u64,
    pub players: BTreeMap<u32, PlayerSession>,
    pub secure_fog_required: bool,
    pub match_protocol: Option<omoba_core::transport::MatchProtocol>,
    authenticated_team_bindings: BTreeMap<u32, u32>,
    /// Rejoin secret per player: issued (and rotated) on every accepted join,
    /// sent only in that player's unicast TeamGameStart, never logged.
    resume_tokens: BTreeMap<u32, ResumeTokens>,
}

/// Two-phase rotation (E077): a newly issued token stays `pending` until the
/// client proves it received it by presenting it on a later join. Until then
/// the last proven token (`confirmed`) keeps working, so a join that the
/// server accepted but whose TeamGameStart never reached the client (the
/// attempt timed out on a dead link) cannot lock the player out.
#[derive(Debug, Clone, Default)]
struct ResumeTokens {
    confirmed: Option<[u8; 32]>,
    pending: Option<[u8; 32]>,
}

/// Length and content compared without an early exit.
fn token_matches(expected: &[u8; 32], presented: &[u8]) -> bool {
    let mut difference = (presented.len() ^ expected.len()) as u8;
    for (index, byte) in expected.iter().enumerate() {
        difference |= byte ^ presented.get(index).copied().unwrap_or(0);
    }
    difference == 0
}

/// Outcome of the rejoin identity check for one JoinRequest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinIdentity {
    /// The player never joined this match; no token is required.
    FirstJoin,
    /// The player joined before and presented its latest resume token.
    Rejoin,
}

impl LockstepState {
    pub fn new(master_seed: u64) -> Self {
        Self {
            current_tick: 0,
            master_seed,
            players: BTreeMap::new(),
            secure_fog_required: false,
            match_protocol: None,
            authenticated_team_bindings: BTreeMap::new(),
            resume_tokens: BTreeMap::new(),
        }
    }

    /// Identity check before registration. A player that has joined before can
    /// come back only with a token the server issued to it and that is still
    /// live: the last one the client proved it holds, or the newest one issued
    /// since. A missing or other token is refused, so nobody can take over a
    /// held hero. Until the first issued token is proven, an empty token still
    /// counts as a first join (that client never learned any token). Leaves all
    /// state unchanged.
    pub fn verify_join_identity(&self, player_id: u32, presented: &[u8]) -> Result<JoinIdentity, String> {
        let Some(tokens) = self.resume_tokens.get(&player_id) else {
            return if presented.is_empty() { Ok(JoinIdentity::FirstJoin) }
                else { Err(format!("player_id {player_id} presented a resume token before any join")) };
        };
        if tokens.confirmed.is_none() && presented.is_empty() {
            return Ok(JoinIdentity::FirstJoin);
        }
        // Both candidates are always compared, without an early exit.
        let confirmed = tokens.confirmed.as_ref().is_some_and(|t| token_matches(t, presented));
        let pending = tokens.pending.as_ref().is_some_and(|t| token_matches(t, presented));
        if !(confirmed | pending) {
            return Err(format!("player_id {player_id} rejoin refused: missing or stale resume token"));
        }
        Ok(JoinIdentity::Rejoin)
    }

    /// Called once the join that presented `presented` is accepted: a pending
    /// token presented here is now proven received and becomes the confirmed
    /// one (older tokens die). Returns a fresh pending secret for the next
    /// rejoin; it replaces any earlier unproven one.
    pub fn issue_resume_token(&mut self, player_id: u32, presented: &[u8]) -> Vec<u8> {
        let tokens = self.resume_tokens.entry(player_id).or_default();
        if let Some(pending) = tokens.pending {
            if token_matches(&pending, presented) {
                tokens.confirmed = Some(pending);
            }
        }
        let token: [u8; 32] = rand::random();
        tokens.pending = Some(token);
        token.to_vec()
    }

    /// Populated only by the server authentication boundary, never from a
    /// player wire field.
    pub fn authorize_player_team(&mut self, player_id: u32, team_id: u32) -> Result<(), String> {
        if player_id == 0 || team_id == 0 {
            return Err("invalid authenticated player/team binding".into());
        }
        self.authenticated_team_bindings.insert(player_id, team_id);
        Ok(())
    }

    pub fn register_secure_player(
        &mut self,
        player_id: u32,
        name: String,
        role: JoinRoleEnum,
        negotiation: omoba_core::transport::MatchCapabilityNegotiation,
        view_epoch: u64,
    ) -> Result<(u32, omoba_core::runtime::SecureSessionBinding), String> {
        let team_id = *self
            .authenticated_team_bindings
            .get(&player_id)
            .ok_or_else(|| format!("player_id {player_id} has no authenticated team binding"))?;
        let binding = omoba_core::runtime::SecureSessionBinding::negotiate(
            &negotiation,
            self.match_protocol,
            team_id,
            view_epoch,
            true,
        )
        .map_err(|error| format!("secure negotiation rejected: {error:?}"))?;
        let id = self.register_player(player_id, name, role)?;
        self.match_protocol = Some(binding.protocol);
        Ok((id, binding))
    }

    pub fn register_player(
        &mut self,
        player_id: u32,
        name: String,
        role: JoinRoleEnum,
    ) -> Result<u32, String> {
        if role == JoinRoleEnum::Player && player_id == 0 {
            return Err("player join missing non-zero client-declared player_id".to_string());
        }
        if role == JoinRoleEnum::Player && self.players.contains_key(&player_id) {
            return Err(format!(
                "player_id {} already has an active session",
                player_id
            ));
        }
        let id = player_id;
        self.players.insert(
            id,
            PlayerSession {
                player_id: id,
                player_name: name,
                role,
                last_input_tick: 0,
            },
        );
        Ok(id)
    }

    pub fn unregister_player(&mut self, player_id: u32) {
        self.players.remove(&player_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_player_accepts_client_declared_ids() {
        let mut state = LockstepState::new(0x1234);
        assert_eq!(
            state
                .register_player(1, "player1".into(), JoinRoleEnum::Player)
                .unwrap(),
            1
        );
        assert_eq!(
            state
                .register_player(2, "player2".into(), JoinRoleEnum::Player)
                .unwrap(),
            2
        );
        assert_eq!(state.players.len(), 2);
    }

    #[test]
    fn register_player_rejects_missing_or_duplicate_ids() {
        let mut state = LockstepState::new(0x1234);
        assert!(state
            .register_player(0, "missing".into(), JoinRoleEnum::Player)
            .is_err());
        state
            .register_player(1, "player1".into(), JoinRoleEnum::Player)
            .unwrap();
        assert!(state
            .register_player(1, "dupe".into(), JoinRoleEnum::Player)
            .is_err());
        assert_eq!(state.players.len(), 1);
    }

    #[test]
    fn secure_registration_accepts_multiple_players_on_the_same_team() {
        let mut state = LockstepState::new(0x1234);
        state.authorize_player_team(1, 1).unwrap();
        state.authorize_player_team(3, 1).unwrap();
        state.authorize_player_team(2, 2).unwrap();
        let negotiation = omoba_core::transport::MatchCapabilityNegotiation {
            requested_protocol: omoba_core::transport::SELECTIVE_LOCKSTEP_PROTOCOL_VERSION,
            supported_protocols: vec![omoba_core::transport::SELECTIVE_LOCKSTEP_PROTOCOL_VERSION],
            secure_fog_required: true,
        };

        let (_, first) = state
            .register_secure_player(
                1,
                "team-1-a".into(),
                JoinRoleEnum::Player,
                negotiation.clone(),
                1,
            )
            .unwrap();
        let (_, second) = state
            .register_secure_player(
                3,
                "team-1-b".into(),
                JoinRoleEnum::Player,
                negotiation.clone(),
                1,
            )
            .unwrap();
        let (_, opponent) = state
            .register_secure_player(2, "team-2".into(), JoinRoleEnum::Player, negotiation, 1)
            .unwrap();

        assert_eq!(first.authenticated_team_id, 1);
        assert_eq!(second.authenticated_team_id, 1);
        assert_eq!(opponent.authenticated_team_id, 2);
        assert_eq!(state.players.len(), 3);
    }

    #[test]
    fn rejoin_requires_the_latest_resume_token_of_that_player() {
        let mut state = LockstepState::new(0x1234);
        assert_eq!(state.verify_join_identity(7, &[]), Ok(JoinIdentity::FirstJoin));
        assert!(state.verify_join_identity(7, &[1; 32]).is_err(), "no token exists before a first join");
        state.register_player(7, "p7".into(), JoinRoleEnum::Player).unwrap();
        let first = state.issue_resume_token(7, &[]);
        assert_eq!(first.len(), 32);
        // Disconnect: the session ends and the player is unregistered.
        state.unregister_player(7);
        // The first token is proven by its first use; once a token is proven,
        // an empty one no longer counts as a first join.
        assert_eq!(state.verify_join_identity(7, &first), Ok(JoinIdentity::Rejoin));
        state.register_player(7, "p7".into(), JoinRoleEnum::Player).unwrap();
        let second = state.issue_resume_token(7, &first);
        assert_ne!(first, second, "every join rotates the token");
        state.unregister_player(7);
        for wrong in [Vec::new(), vec![0; 32], first[..31].to_vec(), [first.clone(), vec![0]].concat()] {
            assert!(state.verify_join_identity(7, &wrong).is_err(), "refused {wrong:?}");
        }
        assert!(state.verify_join_identity(8, &first).is_err(), "a token belongs to its own player");
        assert_eq!(state.verify_join_identity(7, &second), Ok(JoinIdentity::Rejoin));
        state.register_player(7, "p7".into(), JoinRoleEnum::Player).unwrap();
        let third = state.issue_resume_token(7, &second);
        state.unregister_player(7);
        assert!(state.verify_join_identity(7, &first).is_err(), "a token older than the proven one is stale");
        assert_eq!(state.verify_join_identity(7, &third), Ok(JoinIdentity::Rejoin));
    }

    /// E077: the server accepted a rejoin and issued a new token, but the
    /// TeamGameStart carrying it never reached the client (the attempt timed
    /// out on the dead link). The client retries with the token it still holds
    /// and must get in; the token it never saw is replaced.
    #[test]
    fn rejoin_whose_new_token_was_lost_can_retry_with_the_previous_token() {
        let mut state = LockstepState::new(0x1234);
        state.register_player(7, "p7".into(), JoinRoleEnum::Player).unwrap();
        // First join's TeamGameStart was lost too: an empty token still works.
        let lost_first = state.issue_resume_token(7, &[]);
        state.unregister_player(7);
        assert_eq!(state.verify_join_identity(7, &[]), Ok(JoinIdentity::FirstJoin));
        state.register_player(7, "p7".into(), JoinRoleEnum::Player).unwrap();
        let held = state.issue_resume_token(7, &[]);
        state.unregister_player(7);
        assert!(state.verify_join_identity(7, &lost_first).is_err(), "a replaced unproven token is dead");
        // Rejoin with `held` is accepted; its reply (with `lost`) never arrives.
        assert_eq!(state.verify_join_identity(7, &held), Ok(JoinIdentity::Rejoin));
        state.register_player(7, "p7".into(), JoinRoleEnum::Player).unwrap();
        let lost = state.issue_resume_token(7, &held);
        state.unregister_player(7);
        // Retry with the held token.
        assert_eq!(state.verify_join_identity(7, &held), Ok(JoinIdentity::Rejoin));
        state.register_player(7, "p7".into(), JoinRoleEnum::Player).unwrap();
        let received = state.issue_resume_token(7, &held);
        state.unregister_player(7);
        assert!(state.verify_join_identity(7, &lost).is_err(), "the never-delivered token was replaced");
        assert!(state.verify_join_identity(7, &[]).is_err(), "a proven player needs a token");
        assert_eq!(state.verify_join_identity(7, &received), Ok(JoinIdentity::Rejoin));
    }
}
