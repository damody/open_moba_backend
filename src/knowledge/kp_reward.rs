//! 對局結束 KP 發放邏輯。
//!
//! 在偵測到 `("td/all/res", "game", "end")` RuntimeEvent 時呼叫
//! `award_kp()`，自動增加玩家 profile 的 `total_kp` 並寫回磁碟。

use std::path::Path;

use super::player_profile::{save_profile, PlayerProfile};

/// `game.toml` `[general_knowledge]` 的 KP 獎勵設定。
#[derive(Debug, Clone, Copy)]
pub struct KpRewardConfig {
    pub base_kp_reward: u32,
    pub win_kp_bonus: u32,
}

impl Default for KpRewardConfig {
    fn default() -> Self {
        Self {
            base_kp_reward: 3,
            win_kp_bonus: 2,
        }
    }
}

/// 對局結束時發放 KP。
///
/// - `is_victory`：是否勝利（勝利額外獎勵 `win_kp_bonus`）。
/// - CHIMPS 模式下 `is_victory` 語意相同，KP 照常發放（CHIMPS 只禁對局內加成）。
pub fn award_kp(omb_dir: &Path, profile: &mut PlayerProfile, config: KpRewardConfig, is_victory: bool) {
    let earned = config.base_kp_reward + if is_victory { config.win_kp_bonus } else { 0 };
    profile.total_kp = profile.total_kp.saturating_add(earned);
    save_profile(omb_dir, profile);
    log::info!(
        "[general_knowledge] 對局結束，{}，發放 KP +{}（total={}）",
        if is_victory { "勝利" } else { "失敗" },
        earned,
        profile.total_kp,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::player_profile::PlayerProfile;

    #[test]
    fn victory_gives_base_plus_bonus() {
        let dir = std::env::temp_dir().join("gk_kp_victory");
        let _ = std::fs::create_dir_all(&dir);
        let config = KpRewardConfig { base_kp_reward: 3, win_kp_bonus: 2 };
        let mut p = PlayerProfile::default();
        award_kp(&dir, &mut p, config, true);
        assert_eq!(p.total_kp, 5);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn defeat_gives_base_only() {
        let dir = std::env::temp_dir().join("gk_kp_defeat");
        let _ = std::fs::create_dir_all(&dir);
        let config = KpRewardConfig { base_kp_reward: 3, win_kp_bonus: 2 };
        let mut p = PlayerProfile::default();
        award_kp(&dir, &mut p, config, false);
        assert_eq!(p.total_kp, 3);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
