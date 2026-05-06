//! P4：每個蠕變廣播蠕變的選通狀態。 M 速度外推。
//!
//! 在 P4 之前，伺服器每次路點/方向/都會發出“creep.M”
//! 碰撞推進了蜱蟲 — 1000 次蠕變 × 30 TPS，這是一個潛力
//! 最壞情況為 30K 訊息/秒。客戶端已經在事件之間進行了 lerps，因此每個滴答
//! 發出的訊號完全是浪費頻寬。
//!
//! P4 將發射控制為「僅狀態變化」：
//! 1. 目標航路點變更（新檢查點、重新計算路徑、碰到障礙物）。
//! 2. 速度變化 > 5%（緩慢施加/移除冰）。
//! 3. 實體首先進入可見世界（首字母M由其處理）
//! CreepStatus::PreWalk 過渡，與之前相同的地點）。
//!
//! 比較狀態作為規範「Component」存在於此處；每個creep_tick
//! 迭代讀取最後廣播的快照並決定是否發出。

use specs::{Component, storage::VecStorage};

/// 最近包含在「creep.M」廣播中的欄位的快照
/// 對於這個實體。將每個刻度與目前目標/速度進行比較
/// 來決定是否需要一個新的M事件。
///
/// `last_broadcast_*` 是 `Option<_>` 所以第一次發出是無條件的
/// （該實體沒有先前的快照）。第一次發射後組件是
/// 填入 `Some(_)` 並隨後檢查與儲存的比較
/// 價值觀。
#[derive(Clone, Debug, Default)]
pub struct CreepMoveBroadcast {
    /// 最後廣播的目標航路點（世界單位）。 “無”=從不廣播。
    pub last_target: Option<vek::Vec2<f32>>,
    /// 最後廣播速度（每秒世界單位）。
    pub last_velocity: Option<f32>,
    /// 伺服器在發出最後一次廣播時勾選。
    pub last_start_tick: Option<u64>,
}

impl Component for CreepMoveBroadcast {
    type Storage = VecStorage<Self>;
}

impl CreepMoveBroadcast {
    /// 決定是否應在給定的情況下發出新的 Creep.M 事件
    /// 目前刻度的目標+速度。
    ///
    /// 規則：
    /// - 沒有事先廣播→總是發出。
    /// - 目標偏差 > 0.25 世界單位（= 1 量化步長）。
    /// - 速度偏差 > 5%（相對）或 > 1.0 絕對值 — 覆蓋
    /// 緩慢施冰（例如 200 → 140 = 30%）並緩慢過期快速恢復。
    pub fn should_emit(&self, target: vek::Vec2<f32>, velocity: f32) -> bool {
        // 首次發出：無先前狀態。
        let Some(prev_target) = self.last_target else { return true };
        let Some(prev_vel) = self.last_velocity else { return true };

        // 目標已更改：比較平方距離以跳過 sqrt。
        let dx = target.x - prev_target.x;
        let dy = target.y - prev_target.y;
        let dist_sq = dx * dx + dy * dy;
        // 0.25世界單位=Position16量化精度；低於那個
        // 無論如何，電匯值是相同的，因此無需發送任何內容。
        if dist_sq > 0.25 * 0.25 {
            return true;
        }

        // 速度變化：> 5% 相對或 > 1.0 絕對。
        // 相對處理慢/不慢（例如冰200→140）；絕對句柄
        // 低速蠕動，其中 5% 是子像素雜訊。
        let vel_diff = (velocity - prev_vel).abs();
        if vel_diff > 1.0 {
            return true;
        }
        if prev_vel.abs() > f32::EPSILON && vel_diff / prev_vel.abs() > 0.05 {
            return true;
        }

        false
    }

    /// 記錄給定字段剛剛發生的發射。
    pub fn record(&mut self, target: vek::Vec2<f32>, velocity: f32, start_tick: u64) {
        self.last_target = Some(target);
        self.last_velocity = Some(velocity);
        self.last_start_tick = Some(start_tick);
    }
}
