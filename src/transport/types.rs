#[cfg(feature = "kcp")]
use super::metrics::KcpBytesCounter;
use crossbeam_channel::{Receiver, Sender};
use serde::{Deserialize, Serialize};
use serde_json::json;
#[cfg(feature = "kcp")]
use std::sync::Arc;
use std::time::SystemTime;

/// P2 二進位協定遷移：類型化的 prost 有效負載與
/// 舊版 JSON `msg` 字串。當「OutboundMsg.typed」為「Some(_)」時，KCP
/// 廣播線程直接建構`GameEvent.typed_pa​​yload`並離開
/// `data_json` 為空 — 因此線路僅攜帶 prost 變體。
///
/// JSON `msg` 欄位保留用於重複資料刪除/路由器自省
/// 廣播線程。它不會在鍵入的路徑中走線。
///
/// 僅在“kcp”下可用，因為 prost 類型位於
/// `kcp_transport::game_proto`。
#[cfg(feature = "kcp")]
#[derive(Clone, Debug)]
pub enum TypedOutbound {
    Heartbeat(super::kcp_transport::game_proto::HeartbeatTick),
    BuffAdd(super::kcp_transport::game_proto::BuffAdd),
    BuffRemove(super::kcp_transport::game_proto::BuffRemove),
    HeroCreate(super::kcp_transport::game_proto::HeroCreate),
    UnitCreate(super::kcp_transport::game_proto::UnitCreate),
    GameLives(super::kcp_transport::game_proto::GameLives),
    GameEnd(super::kcp_transport::game_proto::GameEnd),
    LegacyJson(super::kcp_transport::game_proto::LegacyJson),
}

/// P5 广播策略 — 声明谁应该接收此事件。這
/// 傳輸的廣播線程使用它來選擇目標會話之前
/// 透過 `Arc<[u8]>` 克隆編碼幀。當「policy」為「None」時，遺留
/// 基於主題的路由適用（未遷移的發射站點的向後相容）。
#[cfg(any(feature = "grpc", feature = "kcp"))]
#[derive(Clone, Debug)]
pub enum BroadcastPolicy {
    /// 覆蓋每一位已連線的玩家。用於遊戲範圍的狀態
    /// （回合/生命/結束/tower_templates/map_data）。
    All,
    /// 過濾到視口包含 (x, y) 的玩家。
    /// 大桶：蠕動事件、彈道事件、entity.F、塔事件。
    AoiPoint(f32, f32),
    /// 與 AoiPoint 相同，但座標來自查找“entity_id”
    /// 透過 AoiGrid 註冊表取得目前 Pos。當來電者有 id 但沒有 pos 時使用
    /// 價格便宜（例如，hero.stats hot tick）。
    AoiEntity(u64),
    /// 單一目標 - 特定於玩家的事件，例如 Hero.inventory、
    /// 蠕動可見度差異（目前 `td/{player}/res` 主題）。
    PlayerOnly(String),
}

/// 從遊戲邏輯到傳輸層的出站訊息。
/// 替換遊戲邏輯代碼中的“MqttMsg”。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OutboundMsg {
    pub topic: String,
    pub msg: String,
    pub time: SystemTime,
    /// 遊戲座標中的實體位置，用於視口過濾。
    /// None = 繞過過濾的全域事件（心跳、死亡等）。
    #[serde(skip)]
    pub entity_pos: Option<(f32, f32)>,
    /// P2 二進位遷移：當 Some 時，傳輸會發出 GameEvent
    /// `typed_pa​​yload` 設定和 `data_json` 留空。 `msg` 仍然帶有
    /// 用於重複資料刪除/路由器自省的 JSON 副本（僅保留在記憶體中）。
    #[cfg(feature = "kcp")]
    #[serde(skip)]
    pub typed: Option<TypedOutbound>,
    /// P5顯式廣播策略。 「無」 = 傳統的主題為基礎的路由。
    /// 當 Some 時，廣播線程會忽略基於「主題」的啟發式方法，並且
    /// 根據策略變體確定目標會話。
    #[cfg(any(feature = "grpc", feature = "kcp"))]
    #[serde(skip)]
    pub policy: Option<BroadcastPolicy>,
    /// 第 2 階段鎖步線框。當“Some”時，kcp 傳輸
    /// 廣播線程發出對應的鎖步標記（0x11 / 0x12 /
    /// 0x14 / 0x16）直接繞過 GameEvent 信封。當「無」時，
    /// 使用舊的“類型化”/JSON 路徑。
    ///
    /// 其他「OutboundMsg」欄位（「topic」、「msg」、「typed」、「policy」）
    /// 設定“lockstep_frame”時將被忽略；鎖步框架承載
    /// 它自己的路由（廣播所有 Tick/Hash，每個客戶端廣播
    /// 遊戲開始/快照Resp）。
    #[cfg(feature = "kcp")]
    #[serde(skip)]
    pub lockstep_frame: Option<crate::lockstep::LockstepFrame>,
}

impl OutboundMsg {
    pub fn new(topic: &String, t: &String, a: &String, v: serde_json::Value) -> OutboundMsg {
        #[derive(Serialize, Deserialize)]
        struct ResData {
            t: String,
            a: String,
            d: serde_json::Value,
        }
        let res = ResData {
            t: t.clone(),
            a: a.clone(),
            d: v,
        };
        OutboundMsg {
            topic: topic.to_owned(),
            msg: json!(res).to_string(),
            time: SystemTime::now(),
            entity_pos: None,
            #[cfg(feature = "kcp")]
            typed: None,
            #[cfg(any(feature = "grpc", feature = "kcp"))]
            policy: None,
            #[cfg(feature = "kcp")]
            lockstep_frame: None,
        }
    }

    pub fn new_s<'a>(topic: &'a str, t: &'a str, a: &'a str, v: serde_json::Value) -> OutboundMsg {
        #[derive(Serialize, Deserialize)]
        struct ResData {
            t: String,
            a: String,
            d: serde_json::Value,
        }
        let res = ResData {
            t: t.to_owned(),
            a: a.to_owned(),
            d: v,
        };
        OutboundMsg {
            topic: topic.to_owned(),
            msg: json!(res).to_string(),
            time: SystemTime::now(),
            entity_pos: None,
            #[cfg(feature = "kcp")]
            typed: None,
            #[cfg(any(feature = "grpc", feature = "kcp"))]
            policy: None,
            #[cfg(feature = "kcp")]
            lockstep_frame: None,
        }
    }

    /// 建立一個具有實體位置的 OutboundMsg 以進行視口過濾。
    pub fn new_s_at(
        topic: &str,
        t: &str,
        a: &str,
        v: serde_json::Value,
        x: f32,
        y: f32,
    ) -> OutboundMsg {
        #[derive(Serialize, Deserialize)]
        struct ResData {
            t: String,
            a: String,
            d: serde_json::Value,
        }
        let res = ResData {
            t: t.to_owned(),
            a: a.to_owned(),
            d: v,
        };
        OutboundMsg {
            topic: topic.to_owned(),
            msg: json!(res).to_string(),
            time: SystemTime::now(),
            entity_pos: Some((x, y)),
            #[cfg(feature = "kcp")]
            typed: None,
            // P5：entity_pos 網站預設為 AoiPoint，因此傳統來電者選擇加入
            // 無需按站點遷移即可進行 AOI 過濾。他們還可以
            // 透過 `.with_policy(...)` 覆蓋 All / PlayerOnly / AoiEntity。
            #[cfg(any(feature = "grpc", feature = "kcp"))]
            policy: Some(BroadcastPolicy::AoiPoint(x, y)),
            #[cfg(feature = "kcp")]
            lockstep_frame: None,
        }
    }

    /// P2 二進位遷移建構函式。 `typed` 是預先建立的 prost 訊息；
    /// `json_fallback` 是用於建立 `msg` 字串的舊版 `d` 字段
    /// 用於重複資料刪除/路由器自省（JSON 形式不會出現在
    /// 當「typed」為 Some 時連線 — 僅發出 prost 變體）。
    #[cfg(feature = "kcp")]
    pub fn new_typed(
        topic: &str,
        t: &str,
        a: &str,
        typed: TypedOutbound,
        json_fallback: serde_json::Value,
    ) -> OutboundMsg {
        OutboundMsg {
            topic: topic.to_owned(),
            msg: json!({ "t": t, "a": a, "d": json_fallback }).to_string(),
            time: SystemTime::now(),
            entity_pos: None,
            typed: Some(typed),
            policy: None,
            lockstep_frame: None,
        }
    }

    /// 與“new_typed”相同，但帶有用於視窗過濾的實體位置。
    #[cfg(feature = "kcp")]
    pub fn new_typed_at(
        topic: &str,
        t: &str,
        a: &str,
        typed: TypedOutbound,
        json_fallback: serde_json::Value,
        x: f32,
        y: f32,
    ) -> OutboundMsg {
        OutboundMsg {
            topic: topic.to_owned(),
            msg: json!({ "t": t, "a": a, "d": json_fallback }).to_string(),
            time: SystemTime::now(),
            entity_pos: Some((x, y)),
            typed: Some(typed),
            // P5：預設為 AoiPoint，因此廣播線程過濾
            // 無需按站點遷移的視口。需要的來電者 全部 /
            // AoiEntity / PlayerOnly 透過 `.with_policy(...)` 覆蓋。
            policy: Some(BroadcastPolicy::AoiPoint(x, y)),
            lockstep_frame: None,
        }
    }

    // ===== P5 政策助手 =====

    /// 將“BroadcastPolicy”附加到此訊息（建構器樣式）。
    #[cfg(any(feature = "grpc", feature = "kcp"))]
    pub fn with_policy(mut self, policy: BroadcastPolicy) -> Self {
        self.policy = Some(policy);
        self
    }

    /// 快捷方式：使用“BroadcastPolicy::All”建立一個“new_typed”訊息。
    /// 用於遊戲範圍的事件（回合/生命/結束/tower_templates/map_data）。
    #[cfg(feature = "kcp")]
    pub fn new_typed_all(
        topic: &str,
        t: &str,
        a: &str,
        typed: TypedOutbound,
        json_fallback: serde_json::Value,
    ) -> OutboundMsg {
        OutboundMsg {
            topic: topic.to_owned(),
            msg: json!({ "t": t, "a": a, "d": json_fallback }).to_string(),
            time: SystemTime::now(),
            entity_pos: None,
            typed: Some(typed),
            policy: Some(BroadcastPolicy::All),
            lockstep_frame: None,
        }
    }

    /// 快捷方式：使用“BroadcastPolicy::AoiEntity(id)”建立一個“new_typed”訊息。
    /// 當發射站點擁有“entity_id”但不擁有其目前位置時使用
    /// 便宜的手頭（例如，hero.hot tick、實體死亡、沒有 pos 的 HP 更新）。
    #[cfg(feature = "kcp")]
    pub fn new_typed_aoi_entity(
        topic: &str,
        t: &str,
        a: &str,
        typed: TypedOutbound,
        json_fallback: serde_json::Value,
        entity_id: u64,
    ) -> OutboundMsg {
        OutboundMsg {
            topic: topic.to_owned(),
            msg: json!({ "t": t, "a": a, "d": json_fallback }).to_string(),
            time: SystemTime::now(),
            entity_pos: None,
            typed: Some(typed),
            policy: Some(BroadcastPolicy::AoiEntity(entity_id)),
            lockstep_frame: None,
        }
    }

    /// 階段 2 鎖步：建造一個攜帶鎖步線框的 OutboundMsg。
    /// kcp 傳輸的廣播線程讀取“lockstep_frame”並發出
    /// 所有會話的適當標籤（0x11 / 0x12 / 0x14 / 0x16）
    /// (TickBatch / StateHash) 或一個會話 (GameStart / SnapshotResp)。
    /// 此路徑上的所有其他「OutboundMsg」欄位都將被忽略。
    #[cfg(feature = "kcp")]
    pub fn lockstep_frame(frame: crate::lockstep::LockstepFrame) -> OutboundMsg {
        OutboundMsg {
            topic: String::new(),
            msg: String::new(),
            time: SystemTime::now(),
            entity_pos: None,
            typed: None,
            policy: Some(BroadcastPolicy::All),
            lockstep_frame: Some(frame),
        }
    }

    /// 快捷方式：僅 JSON（非 kcp）“全部”策略。對於非 kcp 構建，我們仍然
    /// 想要標記遊戲範圍的事件，以便 grpc 廣播線程看到它們。
    #[cfg(any(feature = "grpc", feature = "kcp"))]
    pub fn new_s_all(topic: &str, t: &str, a: &str, v: serde_json::Value) -> OutboundMsg {
        let mut m = OutboundMsg::new_s(topic, t, a, v);
        m.policy = Some(BroadcastPolicy::All);
        m
    }
}

impl Default for OutboundMsg {
    fn default() -> OutboundMsg {
        OutboundMsg {
            topic: "".to_owned(),
            msg: "".to_owned(),
            time: SystemTime::now(),
            entity_pos: None,
            #[cfg(feature = "kcp")]
            typed: None,
            #[cfg(any(feature = "grpc", feature = "kcp"))]
            policy: None,
            #[cfg(feature = "kcp")]
            lockstep_frame: None,
        }
    }
}

/// P6 兩層批次視窗：依延遲預算對出站事件進行分類。
///
/// 「緊急」事件立即刷新（低於 10 毫秒的使用者體驗預算）。 「正常」事件
/// 從 10~33ms 視窗內的批次 + 重複資料刪除中受益。
#[cfg(any(feature = "grpc", feature = "kcp"))]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Urgency {
    /// 盡快沖洗。死亡/技能施放/爆炸/生成/投射物/增益/遊戲狀態。
    Urgent,
    /// 批次時間長達 33 毫秒。 HP/面對/移動滴答聲，心跳，其他一切。
    Normal,
}

/// P6：將「(msg_type, action)」分類為「緊急」類別。鏡像形狀
/// `is_dedupable` - 集中在一個地方，因此兩個策略都會發展
/// 一起。
///
/// 每堂課的理由：
/// - `*.D` / `.death` — 死亡回饋：英雄需要立即看到。
/// - `*.C` / `.create` — 產生：位置/視覺彈出需要立即進行。
/// - `projectile.*` — 所有投射物事件（創造、銷毀）都是視覺的並且
/// 簡短的;延遲它們會破壞命中/未命中的感知。
/// - `game.explosion/.end/.round` — 遊戲狀態轉換，不可批次處理。
/// - `creep.stall` — 一次性碰撞回饋。
/// - `tower.create/.upgrade` — 很少見，但對使用者體驗至關重要。
/// - `buff.*` — 新增/刪除：玩家必須看到 buff 清晰地出現/消失。
///
/// 其他所有內容（creep.M/.H/.S、entity.F、hero.hot、heartbeat.tick）都會下降
/// 到“正常”並享受重複資料刪除視窗。
#[cfg(any(feature = "grpc", feature = "kcp"))]
pub fn urgency(msg_type: &str, action: &str) -> Urgency {
    // 行動級：無論何種類型，任何破壞/創造都是緊急的。
    match action {
        "D" | "death" | "C" | "create" => return Urgency::Urgent,
        _ => {}
    }
    match (msg_type, action) {
        ("game", "explosion" | "end" | "round") => Urgency::Urgent,
        ("projectile", _) => Urgency::Urgent,
        ("tower", "upgrade") => Urgency::Urgent,
        ("creep", "stall") => Urgency::Urgent,
        ("buff", _) => Urgency::Urgent,
        _ => Urgency::Normal,
    }
}

/// 從傳輸層到遊戲邏輯的入站訊息。
/// 替換遊戲邏輯程式碼中的「PlayerData」。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct InboundMsg {
    pub name: String,
    pub t: String,
    pub a: String,
    pub d: serde_json::Value,
}

/// 從 MCP 伺服器到遊戲循環的查詢請求。
#[cfg(any(feature = "grpc", feature = "kcp"))]
pub struct QueryRequest {
    pub query_type: String,
    pub player_name: String,
    pub response_tx: tokio::sync::oneshot::Sender<QueryResponse>,
}

/// 從遊戲循環返回 gRPC/KCP 處理程序的查詢回應。
#[cfg(any(feature = "grpc", feature = "kcp"))]
pub struct QueryResponse {
    pub success: bool,
    pub error: String,
    pub data_json: Vec<u8>,
}

/// 用於空間過濾和可見性差異的客戶端視口矩形（填充）。
#[cfg(any(feature = "grpc", feature = "kcp"))]
#[derive(Copy, Clone, Debug)]
pub struct Viewport {
    pub cx: f32,
    pub cy: f32,
    pub padded_hw: f32,
    pub padded_hh: f32,
}

#[cfg(any(feature = "grpc", feature = "kcp"))]
impl Viewport {
    pub fn new(cx: f32, cy: f32, hw: f32, hh: f32) -> Self {
        Self {
            cx,
            cy,
            padded_hw: hw * 1.3,
            padded_hh: hh * 1.3,
        }
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        (x - self.cx).abs() <= self.padded_hw && (y - self.cy).abs() <= self.padded_hh
    }
}

/// 從傳輸發送到遊戲循環的視口生命週期訊息。
#[cfg(any(feature = "grpc", feature = "kcp"))]
#[derive(Debug, Clone)]
pub enum ViewportMsg {
    Set {
        player_name: String,
        viewport: Viewport,
    },
    Remove {
        player_name: String,
    },
}

/// 傳輸層初始化傳回的句柄。
pub struct TransportHandle {
    pub tx: Sender<OutboundMsg>,
    /// KCP-only priority path for lockstep frames. Gameplay events continue using
    /// `tx`; TickBatch/GameStart/SnapshotResp should use this sender so high
    /// volume legacy GameEvent traffic cannot starve lockstep input playback.
    #[cfg(feature = "kcp")]
    pub lockstep_tx: Sender<OutboundMsg>,
    pub rx: Receiver<InboundMsg>,
    #[cfg(any(feature = "grpc", feature = "kcp"))]
    pub query_rx: Receiver<QueryRequest>,
    #[cfg(any(feature = "grpc", feature = "kcp"))]
    pub viewport_rx: Receiver<ViewportMsg>,
    /// 在 KCP 線上觀察到的每個事件位元組/訊息計數器。
    /// 與廣播線程共享，以便遊戲循環/測試可以調用
    /// 同時執行 `.snapshot()` 或 `.reset()`。
    #[cfg(feature = "kcp")]
    pub counter: Arc<KcpBytesCounter>,
    /// P5：共享 AOI 寬相網格。遊戲循環呼叫 `.lock().rebuild(..)`
    /// 來自心跳使用的相同預先收集的每個刻度；廣播線程
    /// 呼叫 `.lock().lookup_pos(id)` 來解析 `BroadcastPolicy::AoiEntity`。
    /// 互斥鎖爭用很少——兩者都保持鎖定微秒。
    #[cfg(feature = "kcp")]
    pub aoi: Arc<std::sync::Mutex<crate::aoi::AoiGrid>>,
}

#[cfg(test)]
#[cfg(any(feature = "grpc", feature = "kcp"))]
mod urgency_tests {
    use super::{urgency, Urgency};

    #[test]
    fn death_is_urgent_any_kind() {
        assert_eq!(urgency("creep", "D"), Urgency::Urgent);
        assert_eq!(urgency("hero", "D"), Urgency::Urgent);
        assert_eq!(urgency("tower", "D"), Urgency::Urgent);
        assert_eq!(urgency("entity", "death"), Urgency::Urgent);
    }

    #[test]
    fn create_is_urgent_any_kind() {
        assert_eq!(urgency("creep", "C"), Urgency::Urgent);
        assert_eq!(urgency("tower", "create"), Urgency::Urgent);
    }

    #[test]
    fn projectile_all_urgent() {
        // 每個彈體事件——創造、銷毀以及任何未來的變體
        // — 無論採取什麼行動，都是緊急的（視覺上低於 10 毫秒的預算）。
        assert_eq!(urgency("projectile", "C"), Urgency::Urgent);
        assert_eq!(urgency("projectile", "D"), Urgency::Urgent);
        assert_eq!(urgency("projectile", "hit"), Urgency::Urgent);
    }

    #[test]
    fn game_state_transitions_urgent() {
        assert_eq!(urgency("game", "explosion"), Urgency::Urgent);
        assert_eq!(urgency("game", "end"), Urgency::Urgent);
        assert_eq!(urgency("game", "round"), Urgency::Urgent);
    }

    #[test]
    fn buff_events_urgent() {
        assert_eq!(urgency("buff", "add"), Urgency::Urgent);
        assert_eq!(urgency("buff", "remove"), Urgency::Urgent);
        assert_eq!(urgency("buff", "buff_add"), Urgency::Urgent);
    }

    #[test]
    fn tower_upgrade_urgent() {
        assert_eq!(urgency("tower", "upgrade"), Urgency::Urgent);
    }

    #[test]
    fn creep_stall_urgent() {
        assert_eq!(urgency("creep", "stall"), Urgency::Urgent);
    }

    #[test]
    fn hot_path_normal() {
        // 頻寬密集的串流必須是正常的，這樣它們才能受益
        // 10~33ms 重複資料刪除視窗。如果其中任何一個回歸緊急狀態，我們
        // 失去 P1 支付的重複資料刪除節省費用。
        assert_eq!(urgency("creep", "M"), Urgency::Normal);
        assert_eq!(urgency("creep", "H"), Urgency::Normal);
        assert_eq!(urgency("creep", "S"), Urgency::Normal);
        assert_eq!(urgency("entity", "F"), Urgency::Normal);
        assert_eq!(urgency("hero", "hot"), Urgency::Normal);
        assert_eq!(urgency("heartbeat", "tick"), Urgency::Normal);
    }

    #[test]
    fn unknown_defaults_to_normal() {
        // 前向相容：我們尚未分類的新（msg_type，action）
        // 降到正常。比緊急更安全（這會擊敗
        // 我們並不打算立即刷新事件的批次）。
        assert_eq!(urgency("weird", "thing"), Urgency::Normal);
        assert_eq!(urgency("", ""), Urgency::Normal);
    }
}
