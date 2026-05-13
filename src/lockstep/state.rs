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
}

impl LockstepState {
    pub fn new(master_seed: u64) -> Self {
        Self {
            current_tick: 0,
            master_seed,
            players: BTreeMap::new(),
        }
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
}
