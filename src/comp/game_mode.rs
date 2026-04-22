use serde::{Deserialize, Serialize};

/// 遊戲模式。從 map.json 頂層的 `GameMode` 欄位讀取；
/// 未指定則為 `Moba`，以保留既有 MVP_1 行為。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameMode {
    Moba,
    TowerDefense,
}

impl Default for GameMode {
    fn default() -> Self {
        GameMode::Moba
    }
}

impl GameMode {
    pub fn from_opt_str(s: Option<&str>) -> Self {
        match s.map(|x| x.trim()) {
            Some("TowerDefense") | Some("tower_defense") | Some("td") | Some("TD") => {
                GameMode::TowerDefense
            }
            _ => GameMode::Moba,
        }
    }

    pub fn is_td(&self) -> bool {
        matches!(self, GameMode::TowerDefense)
    }
}

/// 玩家生命（TD 模式下才有意義）。小兵走到 path 終點時扣 1，歸零敗北。
/// 預設 0 當作「非 TD 模式」旗標；TD 模式在 `state/initialization.rs` 顯式
/// 設為 `PlayerLives::td_default()` = 100。前端藉 `lives > 0` 判斷是否為 TD 模式。
#[derive(Clone, Copy, Debug)]
pub struct PlayerLives(pub i32);

impl PlayerLives {
    pub const TD_INITIAL: i32 = 100;
    pub fn td_default() -> Self {
        PlayerLives(Self::TD_INITIAL)
    }
}

impl Default for PlayerLives {
    fn default() -> Self {
        PlayerLives(0)
    }
}
