use crossbeam_channel::{bounded, Receiver, Sender};
use failure::Error;
use log::*;
use std::collections::HashMap;
use std::sync::Arc;
use std::thread;
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::sync::Mutex;

use tokio_kcp::{KcpConfig, KcpListener, KcpNoDelayConfig, KcpStream};

use super::metrics::KcpBytesCounter;
use super::types::{
    urgency, BroadcastPolicy, InboundMsg, OutboundMsg, QueryRequest, QueryResponse,
    TransportHandle, TypedOutbound, Urgency, Viewport, ViewportMsg,
};
use crate::aoi::AoiGrid;
use std::sync::atomic::{AtomicU64, Ordering};

// 包含生成的原始程式碼
pub mod game_proto {
    include!(concat!(env!("OUT_DIR"), "/game.rs"));
}

use game_proto::*;
use prost::Message;

// 框架標籤常數（與 omoba-core 相同的協定）。
// 與 omoba-core::kcp::framing 保持同步 — 幀格式必須逐位元組匹配。
const TAG_PLAYER_COMMAND: u8 = 0x01;
const TAG_GAME_EVENT: u8 = 0x02;
const TAG_COMMAND_ACK: u8 = 0x03;
const TAG_SUBSCRIBE_REQUEST: u8 = 0x04;
const TAG_GAME_STATE_REQUEST: u8 = 0x05;
const TAG_GAME_STATE_RESPONSE: u8 = 0x06;
const TAG_VIEWPORT_UPDATE: u8 = 0x07;

// 第 2 階段鎖步標籤。
const TAG_INPUT_SUBMIT: u8 = 0x10;
const TAG_TICK_BATCH: u8 = 0x11;
const TAG_STATE_HASH: u8 = 0x12;
const TAG_JOIN_REQUEST: u8 = 0x13;
const TAG_GAME_START: u8 = 0x14;
const TAG_SNAPSHOT_REQ: u8 = 0x15;
const TAG_SNAPSHOT_RESP: u8 = 0x16;
const TAG_PING_REQ: u8 = 0x17;
const TAG_PING_RESP: u8 = 0x18;

/// 標籤的高位元 — 當幀有效負載經過 LZ4 壓縮時設定。
/// 基本標籤 0x01~0x07 從不使用該位，因此它始終可以作為標誌自由使用。
const COMPRESSION_FLAG: u8 = 0x80;

/// 在嘗試 LZ4 壓縮之前，最小有效負載大小。
const LZ4_THRESHOLD: usize = 128;

/// 寫入幀訊息：[1 位元組標籤][4 位元組 len (big-endian)][N 位元組有效負載]
/// 當有效負載≥LZ4_THRESHOLD並且LZ4縮小它時，有效負載被替換為
/// 大小前置的 LZ4 區塊和 COMPRESSION_FLAG 與標籤進行「或」運算。
/// 與 omoba-core::kcp::framing::write_framed 保持同步。
async fn write_framed<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    tag: u8,
    payload: &[u8],
) -> std::io::Result<()> {
    debug_assert!(
        tag & COMPRESSION_FLAG == 0,
        "base tag must not use high bit; got 0x{:02x}",
        tag
    );
    let (out_tag, out_payload): (u8, &[u8]);
    let compressed_holder;
    if payload.len() >= LZ4_THRESHOLD {
        let c = lz4_flex::block::compress_prepend_size(payload);
        if c.len() < payload.len() {
            out_tag = tag | COMPRESSION_FLAG;
            compressed_holder = c;
            out_payload = &compressed_holder;
        } else {
            out_tag = tag;
            out_payload = payload;
        }
    } else {
        out_tag = tag;
        out_payload = payload;
    }
    let len = out_payload.len() as u32;
    writer.write_u8(out_tag).await?;
    writer.write_u32(len).await?;
    writer.write_all(out_payload).await?;
    writer.flush().await?;
    Ok(())
}

/// 讀取幀訊息，返回（tag，decompressed_pa​​yload，wire_bytes）。
/// 如果在wire標籤上設定了COMPRESSION_FLAG，則有效負載將被解壓縮並
/// 傳回的標籤已移除標誌（呼叫者只能看到 0x01~0x07）。
/// `wire_bytes` = 1（標籤）+ 4（長度）+ N（原始線上位元組）。
/// 與 omoba-core::kcp::framing::read_framed 保持同步。
async fn read_framed<R: AsyncReadExt + Unpin>(
    reader: &mut R,
) -> std::io::Result<Option<(u8, Vec<u8>, usize)>> {
    let tag_raw = match reader.read_u8().await {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    };
    let len = reader.read_u32().await? as usize;
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    let wire_bytes = 1 + 4 + len;
    if tag_raw & COMPRESSION_FLAG != 0 {
        let base_tag = tag_raw & 0x7F;
        let decompressed = lz4_flex::block::decompress_size_prepended(&buf)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Ok(Some((base_tag, decompressed, wire_bytes)))
    } else {
        Ok(Some((tag_raw, buf, wire_bytes)))
    }
}

/// 建立一個與 write_framed 的線上佈局相符的線框位元組緩衝區
/// (`[1B tag][4B len BE][N bytes Payload]`) 具有可選的 LZ4 壓縮
/// 當有效負載≥LZ4_THRESHOLD且壓縮更小時。使用者
/// 廣播線程組裝鎖步幀一次並弧共享它們
/// 跨所有收件者會話。
fn build_framed_bytes(tag: u8, payload: &[u8]) -> Vec<u8> {
    debug_assert!(
        tag & COMPRESSION_FLAG == 0,
        "base tag must not use high bit; got 0x{:02x}",
        tag
    );
    let (out_tag, out_payload): (u8, std::borrow::Cow<'_, [u8]>) = if payload.len() >= LZ4_THRESHOLD
    {
        let c = lz4_flex::block::compress_prepend_size(payload);
        if c.len() < payload.len() {
            (tag | COMPRESSION_FLAG, std::borrow::Cow::Owned(c))
        } else {
            (tag, std::borrow::Cow::Borrowed(payload))
        }
    } else {
        (tag, std::borrow::Cow::Borrowed(payload))
    };
    let mut frame = Vec::with_capacity(1 + 4 + out_payload.len());
    frame.push(out_tag);
    frame.extend_from_slice(&(out_payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(&out_payload);
    frame
}

/// 每個客戶端會話：持有一個發送者來推送出站事件。
///
/// P5：通道有效負載為「Arc<[u8]>」—廣播線程編碼+壓縮
/// 每個幀一次，然後向每個幀分發廉價的“Arc::clone”引用
/// 目標會話。沒有每個會話編碼，沒有每個會話有效負載副本。
struct ClientSession {
    player_name: String,
    event_tx: tokio::sync::mpsc::Sender<Arc<[u8]>>,
    viewport: Option<Viewport>,
    /// P6：每會話單調序列計數器。遞增+印記
    /// 廣播執行緒分派到此會話的每個 GameEvent。這
    /// 客戶端與其最後已知的序列進行比較並請求
    /// 當間隙時透過 TAG_GAME_STATE_REQUEST (query_type="seq-gap") 產生快照
    /// 被觀察到。
    ///
    /// 包裹在 Arc 中，以便廣播線程可以對幀進行標記 + 編碼
    /// 每個會話，無需在編碼期間保持會話互斥體。
    seq: Arc<AtomicU64>,
    /// 步驟 2 鎖定步驟：一旦此會話傳送 JoinRequest (0x13)，則設為 true。
    /// TickBatch (0x11) 和 StateHash (0x12) 廣播僅扇出到
    /// 具有此標誌的會話 - 舊 GameEvent 路徑上的用戶端
    /// （第 2 階段過渡期間的 omb-mcp、omfx）不會看到鎖步流量。
    lockstep_joined: bool,
}

/// 廣播線程和單元使用的純函數策略調度
/// 測試。傳回應接收訊框的會話 ID 清單。
///
/// `sessions` 是即時會話映射的借用； `aoi_lookup` 是一個回呼
/// 廣播線程連接到“AoiGrid::lookup_pos”（測試可以對其進行存根）。
/// 這讓我們可以在不啟動 KCP / tokio 的情況下對調度規則進行單元測試。
#[cfg(test)]
fn select_targets_for_policy(
    policy: Option<&BroadcastPolicy>,
    topic: &str,
    entity_pos: Option<(f32, f32)>,
    sessions: &std::collections::BTreeMap<String, (String, Option<Viewport>)>,
    aoi_lookup: &dyn Fn(u64) -> Option<(f32, f32)>,
) -> Vec<String> {
    match policy {
        Some(BroadcastPolicy::All) => sessions.keys().cloned().collect(),
        Some(BroadcastPolicy::PlayerOnly(name)) => sessions
            .iter()
            .filter(|(_, (player_name, _))| player_name == name)
            .map(|(id, _)| id.clone())
            .collect(),
        Some(BroadcastPolicy::AoiPoint(x, y)) => sessions
            .iter()
            .filter(|(_, (_, vp))| match vp {
                Some(v) => v.contains(*x, *y),
                None => true,
            })
            .map(|(id, _)| id.clone())
            .collect(),
        Some(BroadcastPolicy::AoiEntity(eid)) => match aoi_lookup(*eid) {
            Some((x, y)) => sessions
                .iter()
                .filter(|(_, (_, vp))| match vp {
                    Some(v) => v.contains(x, y),
                    None => true,
                })
                .map(|(id, _)| id.clone())
                .collect(),
            None => sessions.keys().cloned().collect(),
        },
        None => {
            let is_broadcast = topic.contains("/all/");
            sessions
                .iter()
                .filter(|(_, (player_name, vp))| {
                    let topic_ok = is_broadcast
                        || topic.contains(&format!("/{}/", player_name))
                        || player_name.is_empty();
                    if !topic_ok {
                        return false;
                    }
                    match (entity_pos, vp) {
                        (Some((x, y)), Some(v)) => v.contains(x, y),
                        _ => true,
                    }
                })
                .map(|(id, _)| id.clone())
                .collect()
        }
    }
}

// ===== 批次視窗重複資料刪除 =====
// 在單一 33ms 批次視窗內，同一訊息有多個訊息
// （msg_type、action、entity_id）折疊到最新值獲勝。這修剪
// 在線上編碼之前進行冗餘 HP / 移動 / 統計資料更新。
//
// 注意：「peek_kind_and_id」第二次解析 JSON（下面的編碼循環
// 也解析）。這種重複是為了清晰起見而故意的－P2清理
// 目標：原始 oneof / 強型別訊息將消除這兩種解析。

#[derive(Hash, Eq, PartialEq, Debug)]
struct DedupeKey {
    msg_type: String,
    action: String,
    entity_id: u64,
}

/// 如果 (msg_type, action) 是可安全進行重複資料刪除的最新獲勝類型，則傳回 true。
///
/// 包括：移動/朝向/HP/緩慢/統計更新，其中僅最新值
/// 33ms 視窗內的事件。
///
/// 排除（預設傳遞處理）：建立/銷毀事件
/// (`*.C` / `*.D` / `*.death`)、增益、遊戲狀態事件、塔升級、
/// 心跳－每一個都是獨立有意義的並且必須到達。
///
/// 關於發出類型的注意事項：
/// - `F`（面向）總是與 msg_type="entity" 一起發出（用於蠕動/英雄/塔）
/// - 依照命中單位以動態 msg_type 發出「H」（HP）
/// (“英雄”/“小兵”/“單位”/“實體”);我們涵蓋所有變體。
fn is_dedupable(msg_type: &str, action: &str) -> bool {
    matches!(
        (msg_type, action),
        ("creep", "M")
            | ("creep", "H") | ("hero", "H") | ("unit", "H") | ("entity", "H")
            | ("entity", "F")
            | ("creep", "S")
            // P3: hero.stats 拆成兩條 prost 事件，仍為 latest-wins（hot 每 0.3s 推一次；
            // static 只在 level up / ability learn 觸發，多個在 33ms 內可 dedupe）。
            | ("hero", "hot")
            | ("hero", "static")
    )
}

/// 按下（msg_type、action、entity_id）折疊可重複資料刪除訊息，保持最新。
/// 不可重複資料刪除的訊息會依照原始順序傳遞。可重複資料刪除的訊息
/// 保留其第一次出現的位置，其值被最新的覆蓋
/// 有效負載。未知/格式錯誤的 JSON → 通過。
fn dedupe_batch(batch: Vec<OutboundMsg>) -> Vec<OutboundMsg> {
    let mut out: Vec<OutboundMsg> = Vec::with_capacity(batch.len());
    let mut dedupe_idx: hashbrown::HashMap<DedupeKey, usize> = hashbrown::HashMap::new();
    for msg in batch {
        let (t, a, id) = peek_kind_and_id(&msg.msg);
        match (id, is_dedupable(&t, &a)) {
            (Some(entity_id), true) => {
                let key = DedupeKey {
                    msg_type: t,
                    action: a,
                    entity_id,
                };
                match dedupe_idx.get(&key) {
                    Some(&idx) => {
                        // 就地替換，以便保留重複資料刪除後的順序
                        // 確定性（首次出現的時隙，最新的有效負載）。
                        out[idx] = msg;
                    }
                    None => {
                        dedupe_idx.insert(key, out.len());
                        out.push(msg);
                    }
                }
            }
            // 不可重複資料刪除，或 id 欄位遺失/格式錯誤 → 傳遞。
            // 注意：id=0 是一個*合法的*specs::Entity 索引，所以我們只跳過
            // 當 id 欄位本身不存在時進行重複資料刪除（解析回傳 None），
            // 不在值 0 上。
            _ => out.push(msg),
        }
    }
    out
}

fn outbound_urgency(msg: &OutboundMsg) -> Urgency {
    if msg.lockstep_frame.is_some() {
        return Urgency::Urgent;
    }
    let (t, a, _) = peek_kind_and_id(&msg.msg);
    urgency(&t, &a)
}

/// 從 OutboundMsg JSON 中提取 (msg_type, action, Option<entity_id>)
/// 有效負載。 `entity_id = None` 表示解析失敗或 `d.id` 不存在；
/// 呼叫者將其視為不可重複資料。目前的「d.id = 0」是合法的
/// `specs::Entity` 索引並回傳為 `Some(0)`。
fn peek_kind_and_id(payload: &str) -> (String, String, Option<u64>) {
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(payload) else {
        return (String::new(), String::new(), None);
    };
    let t = parsed
        .get("t")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let a = parsed
        .get("a")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let id = parsed
        .get("d")
        .and_then(|d| d.get("id"))
        .and_then(|v| v.as_u64());
    (t, a, id)
}

/// 啟動KCP傳輸層。
///
/// 步驟 2 鎖定步驟：呼叫者傳遞共享的 `Arc<Mutex<InputBuffer>>` 並
/// `Arc<Mutex<LockstepState>>` 因此每個客戶端的讀取循環可以：
/// - 將 0x10 InputSubmit 有效負載推送到右側刻度處的緩衝區；
/// - 註冊玩家+在 0x13 JoinRequest 上回覆 0x14 GameStart；
/// - 在 0x15 SnapshotReq 上回覆 0x16 SnapshotResp。
///
/// 階段 5.3 新增了「lockstep_snapshot_store」：調度程式滴答循環
/// `state::core::tick()` 每隔一段時間就會將一個新的 `WorldSnapshot` 映像檔到這個 Arc 中
/// `SNAPSHOT_INTERVAL_TICKS`（= 30 秒 @ 120 Hz）。 0x15 SnapshotReq 處理程序
/// 克隆出最新的位元組並將它們作為 0x16 SnapshotResp 返回到
/// 請求觀察者客戶端。空字節 (`tick=0`) 是有效的 —
/// 觀察者在沒有引導程式的情況下從目前的tick開始播放。
pub async fn start(
    server_addr: String,
    server_port: String,
    lockstep_input_buffer: Arc<std::sync::Mutex<crate::lockstep::InputBuffer>>,
    lockstep_state: Arc<std::sync::Mutex<crate::lockstep::LockstepState>>,
    lockstep_snapshot_store: Arc<std::sync::Mutex<crate::comp::SnapshotStore>>,
) -> Result<TransportHandle, Error> {
    // 階段 5.x 反壓修復：在 TD_STRESS 下，主機滴答系統仍然存在
    // 發出遺留的每個實體事件（creep.M / Creep.H /Entity.F / Projectile.C
    // — 第 5 階段設計希望副本用戶端在本地計算這些數據，但
    // 生產商尚未全部被削減）。與 TickBroadcaster 結合
    // 120Hz 鎖步幀，峰值速率約 1000+ 條訊息/秒。舊的“有界（10000）”
    // 在大約 10 秒內飽和，「out_tx.send」（阻塞）使廣播公司陷入僵局
    // 任務 - 然後客戶端看到零個 TickBatches 和 sim_runner 被阻止
    // 輸入接收100k 緩衝區~100s 的淨空。真正的解決方法是放棄
    // 遺留事件完全廣播（第 5 階段範圍）和/或分割鎖步
    // 和遊戲事件頻道，因此一個頻道的緩慢消耗不會阻礙另一個頻道的發展。
    let (out_tx, out_rx): (Sender<OutboundMsg>, Receiver<OutboundMsg>) = bounded(100_000);
    let (in_tx, in_rx): (Sender<InboundMsg>, Receiver<InboundMsg>) = bounded(10000);
    let (query_tx, query_rx): (Sender<QueryRequest>, Receiver<QueryRequest>) = bounded(100);
    let (viewport_tx, viewport_rx): (Sender<ViewportMsg>, Receiver<ViewportMsg>) = bounded(1024);

    let sessions: Arc<Mutex<HashMap<String, ClientSession>>> = Arc::new(Mutex::new(HashMap::new()));

    // 每個事件位元組/訊息計數器。與廣播線程共享以便測試
    // 遊戲循環可以快照/重置觀察到的線量。
    let counter: Arc<KcpBytesCounter> = Arc::new(KcpBytesCounter::new());

    // P5：共享 AOI 寬相網格。遊戲循環每刻重建一次，傳輸
    // 執行緒讀取 `BroadcastPolicy::AoiEntity` 查找。 `std::sync::互斥體`
    // （不是`tokio::sync::Mutex`）因為兩個接觸點都是同步的
    // 代碼保持鎖定微秒——鎖定時沒有“.await”。
    let aoi: Arc<std::sync::Mutex<AoiGrid>> = Arc::new(std::sync::Mutex::new(AoiGrid::new()));

    // 後台執行緒：從out_rx讀取並廣播到所有會話
    let sessions_broadcast = sessions.clone();
    let counter_broadcast = counter.clone();
    let aoi_broadcast = aoi.clone();
    thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            // ===== P6: 兩層批次視窗 =====
            // 緊急事件（死亡/產卵/彈頭/爆炸/增益/塔升級/
            // cree.stall/game.*) 盡快刷新，以實現低於 10 毫秒的 UX 延遲。
            // 正常事件（creep.M/H/S、entity.F、hero.hot、heartbeat）進入
            // 10~33ms 的窗口，因此他們可以從重複資料刪除中受益。
            //
            // 演算法:
            // 1. 阻止第一條訊息。
            // 2. 如果第一個是緊急的→耗盡已經準備好的東西（try_recv
            // 循環），立即沖洗。
            // 3. 否則：批次處理 MIN_BATCH，然後繼續批次處理直至
            // MAX_BATCH — 但如果有任何緊急訊息在此之後到達
            // MIN_BATCH，立即刷新。
            //
            // MIN_BATCH = 10ms 的基本原則：為重複資料刪除提供真正的機會
            // 崩潰蠕變.M/.H 在同一滴答內發出爆發（伺服器
            // 以約 30fps 的速度運行，因此一次滴答的事件會在 <1ms 內觸發，但是
            // 連續刻度的法線仍然可以落在同一視窗中）。
            use std::time::{Duration, Instant};
            const MIN_BATCH: Duration = Duration::from_millis(10);
            const MAX_BATCH: Duration = Duration::from_millis(33);

            'outer: loop {
                // 等第一筆訊息（阻塞）
                let first = match out_rx.recv() {
                    Ok(m) => m,
                    Err(_) => {
                        info!("Outbound channel closed, stopping KCP broadcaster");
                        break 'outer;
                    }
                };
                let first_is_urgent = outbound_urgency(&first) == Urgency::Urgent;
                let mut batch: Vec<crate::transport::OutboundMsg> = vec![first];
                let window_start = Instant::now();

                if first_is_urgent {
                    // 緊急負責人：排空已經排隊的所有內容
                    // 堵塞，立即沖洗。保持延遲預算
                    // 此事件<1ms，同時仍批處理任何
                    // 碰巧和它一起到達。
                    while let Ok(m) = out_rx.try_recv() {
                        batch.push(m);
                    }
                } else {
                    // 普通頭：無條件批次為MIN_BATCH，然後
                    // MIN_BATCH..=MAX_BATCH 之間任何緊急情況都會儘早重新整理。
                    loop {
                        let now = Instant::now();
                        let elapsed = now.duration_since(window_start);
                        if elapsed >= MAX_BATCH {
                            break;
                        }
                        let timeout = MAX_BATCH - elapsed;
                        match out_rx.recv_timeout(timeout) {
                            Ok(m) => {
                                let is_lockstep = m.lockstep_frame.is_some();
                                let is_urg = outbound_urgency(&m) == Urgency::Urgent;
                                batch.push(m);
                                if is_lockstep {
                                    break;
                                }
                                // 如果緊急情況在 MIN_BATCH 之後到達，則不要
                                // 再等一下——批次已經
                                // 節省了重複資料刪除成本。
                                if is_urg && window_start.elapsed() >= MIN_BATCH {
                                    break;
                                }
                            }
                            Err(crossbeam_channel::RecvTimeoutError::Timeout) => break,
                            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break 'outer,
                        }
                    }
                }

                // 折疊多餘的最新勝利更新（creep.M/*.H/entity.F/creep.S/hero.stats）
                // 編碼之前。有關完整策略，請參閱“is_dedupable”。
                let batch = dedupe_batch(batch);

                // 處理整個批次
                for msg in batch {
                    // 階段 2 鎖步：當設定「lockstep_frame」時，發出
                    // 直接對應的標籤，繞過GameEvent
                    // 信封。目標：
                    // TickBatch / StateHash → 所有會話
                    //                           lockstep_joined=true
                    // GameStart / SnapshotResp → 單播到 client_session_id
                    if let Some(frame) = msg.lockstep_frame.clone() {
                        match frame {
                            crate::lockstep::LockstepFrame::TickBatch(batch_msg) => {
                                let payload = batch_msg.encode_to_vec();
                                let frame_bytes = build_framed_bytes(TAG_TICK_BATCH, &payload);
                                let frame_arc: Arc<[u8]> = Arc::from(frame_bytes.into_boxed_slice());
                                let sessions = sessions_broadcast.lock().await;
                                let mut to_remove = Vec::new();
                                for (sid, session) in sessions.iter() {
                                    if !session.lockstep_joined { continue; }
                                    if session.event_tx.try_send(frame_arc.clone()).is_err() {
                                        to_remove.push(sid.clone());
                                    }
                                }
                                drop(sessions);
                                if !to_remove.is_empty() {
                                    let mut sessions = sessions_broadcast.lock().await;
                                    for id in to_remove {
                                        sessions.remove(&id);
                                        info!("Removed disconnected KCP session: {}", id);
                                    }
                                }
                            }
                            crate::lockstep::LockstepFrame::StateHash(sh) => {
                                let payload = sh.encode_to_vec();
                                let frame_bytes = build_framed_bytes(TAG_STATE_HASH, &payload);
                                let frame_arc: Arc<[u8]> = Arc::from(frame_bytes.into_boxed_slice());
                                let sessions = sessions_broadcast.lock().await;
                                let mut to_remove = Vec::new();
                                for (sid, session) in sessions.iter() {
                                    if !session.lockstep_joined { continue; }
                                    if session.event_tx.try_send(frame_arc.clone()).is_err() {
                                        to_remove.push(sid.clone());
                                    }
                                }
                                drop(sessions);
                                if !to_remove.is_empty() {
                                    let mut sessions = sessions_broadcast.lock().await;
                                    for id in to_remove {
                                        sessions.remove(&id);
                                        info!("Removed disconnected KCP session: {}", id);
                                    }
                                }
                            }
                            crate::lockstep::LockstepFrame::GameStart { client_session_id, msg: gs } => {
                                let payload = gs.encode_to_vec();
                                let frame_bytes = build_framed_bytes(TAG_GAME_START, &payload);
                                let frame_arc: Arc<[u8]> = Arc::from(frame_bytes.into_boxed_slice());
                                let sessions = sessions_broadcast.lock().await;
                                if let Some(session) = sessions.get(&client_session_id) {
                                    let _ = session.event_tx.try_send(frame_arc);
                                } else {
                                    warn!("GameStart unicast: session '{}' not found", client_session_id);
                                }
                            }
                            crate::lockstep::LockstepFrame::SnapshotResp { client_session_id, msg: sr } => {
                                let payload = sr.encode_to_vec();
                                let frame_bytes = build_framed_bytes(TAG_SNAPSHOT_RESP, &payload);
                                let frame_arc: Arc<[u8]> = Arc::from(frame_bytes.into_boxed_slice());
                                let sessions = sessions_broadcast.lock().await;
                                if let Some(session) = sessions.get(&client_session_id) {
                                    let _ = session.event_tx.try_send(frame_arc);
                                } else {
                                    warn!("SnapshotResp unicast: session '{}' not found", client_session_id);
                                }
                            }
                        }
                        continue; // skip legacy GameEvent path
                    }
                    {
                        // 解析msg JSON以提取t、a、d字段
                        let (msg_type, action, data_bytes) = if let Ok(parsed) =
                            serde_json::from_str::<serde_json::Value>(&msg.msg)
                        {
                            let t = parsed.get("t").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            let a = parsed.get("a").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            let d = parsed.get("d").map(|v| v.to_string().into_bytes()).unwrap_or_default();
                            (t, a, d)
                        } else {
                            ("".to_string(), "".to_string(), msg.msg.as_bytes().to_vec())
                        };

                        let timestamp_ms = msg
                            .time
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis() as u64)
                            .unwrap_or(0);

                        // P9 信封條：每個事件都必須帶有印刷的
                        // 其中“有效負載”。當 `msg.typed` 為 None 時（舊版
                        // 發出網站），將 JSON 包裝在 `LegacyJson` 變體中
                        // 所以電線還是避開包絡線。
                        let payload = match msg.typed.as_ref() {
                            Some(TypedOutbound::Heartbeat(m))         => game_event::Payload::Heartbeat(m.clone()),
                            Some(TypedOutbound::HeroCreate(m))        => game_event::Payload::HeroCreate(m.clone()),
                            Some(TypedOutbound::UnitCreate(m))        => game_event::Payload::UnitCreate(m.clone()),
                            Some(TypedOutbound::BuffAdd(m))           => game_event::Payload::BuffAdd(m.clone()),
                            Some(TypedOutbound::BuffRemove(m))        => game_event::Payload::BuffRemove(m.clone()),
                            Some(TypedOutbound::GameLives(m))         => game_event::Payload::GameLives(m.clone()),
                            Some(TypedOutbound::GameEnd(m))           => game_event::Payload::GameEnd(m.clone()),
                            Some(TypedOutbound::LegacyJson(m))        => game_event::Payload::LegacyJson(m.clone()),
                            None => game_event::Payload::LegacyJson(LegacyJson {
                                msg_type: msg_type.clone(),
                                action: action.clone(),
                                data_json: data_bytes,
                            }),
                        };
                        let _ = timestamp_ms; // P9: timestamp now client-local

                        // P6：每個會話序列標記如下；模板已共用。
                        let event_template = GameEvent {
                            sequence: 0,
                            payload: Some(payload),
                        };

                        let sessions = sessions_broadcast.lock().await;
                        let mut to_remove = Vec::new();

                        // 解析廣播策略 → 目標會話 ID。
                        //
                        // 政策调度是明确的（P5）。如果 `msg.policy` 是
                        // 我們不會退回到傳統的主題為基礎的啟發式方法
                        // 對於未遷移的發射位點。遺留路徑也
                        // 尊重 AOI 的「entity_pos」視窗過濾 —
                        // 保持蠕動/拋射事件 AOI 門控
                        // 按站點遷移推出。
                        let targets: Vec<String> = match &msg.policy {
                            Some(BroadcastPolicy::All) => {
                                sessions.keys().cloned().collect()
                            }
                            Some(BroadcastPolicy::PlayerOnly(name)) => {
                                // 會話映射以 session_id (kcp_<addr>) 為鍵；
                                // 我們需要找到其player_name的會話
                                // 匹配。 O(N) 但 N = 玩家 (≤ ~32)。
                                sessions.iter()
                                    .filter(|(_, s)| &s.player_name == name)
                                    .map(|(id, _)| id.clone())
                                    .collect()
                            }
                            Some(BroadcastPolicy::AoiPoint(x, y)) => {
                                sessions.iter()
                                    .filter(|(_, s)| match &s.viewport {
                                        Some(vp) => vp.contains(*x, *y),
                                        None => true, // no viewport yet → pass through (heartbeat / join flow)
                                    })
                                    .map(|(id, _)| id.clone())
                                    .collect()
                            }
                            Some(BroadcastPolicy::AoiEntity(eid)) => {
                                // 透過AoiGrid解析entity_id → pos。如果
                                // 網格不知道這個實體（產生這個
                                // 重建後勾選，或已死亡），掉落
                                // 返回廣播以避免無聲掉線。
                                let pos_opt = aoi_broadcast.lock()
                                    .ok()
                                    .and_then(|g| g.lookup_pos(*eid));
                                match pos_opt {
                                    Some((x, y)) => sessions.iter()
                                        .filter(|(_, s)| match &s.viewport {
                                            Some(vp) => vp.contains(x, y),
                                            None => true,
                                        })
                                        .map(|(id, _)| id.clone())
                                        .collect(),
                                    None => sessions.keys().cloned().collect(),
                                }
                            }
                            None => {
                                // 傳統的基於主題的路由。 “/all/”⇒廣播；
                                // “/<player_name>/” ⇒ 每個玩家。實體位置
                                // 為兩者提供視口過濾器。
                                let is_broadcast = msg.topic.contains("/all/");
                                sessions.iter()
                                    .filter(|(_, s)| {
                                        let topic_ok = is_broadcast
                                            || msg.topic.contains(&format!("/{}/", s.player_name))
                                            || s.player_name.is_empty();
                                        if !topic_ok { return false; }
                                        match (msg.entity_pos, &s.viewport) {
                                            (Some((x, y)), Some(vp)) => vp.contains(x, y),
                                            _ => true,
                                        }
                                    })
                                    .map(|(id, _)| id.clone())
                                    .collect()
                            }
                        };

                        let is_per_player_topic = !msg.topic.contains("/all/") && msg.topic.starts_with("td/") && msg.topic.ends_with("/res");
                        let mut route_hits = 0u32;
                        // P6：每個會話編碼+壓縮+幀，因為每個
                        // 會話標記其自己的“序列”。有效負載位元組
                        // 僅在序列欄位中不同（通常為 1~2
                        // varint 位元組），因此 CPU 成本主要由 LZ4 主導，其中
                        // 我們按照目標重新運行。對於預計≤32名玩家
                        // 這完全在 33 毫秒的最大視窗範圍內。
                        //
                        // 未來的優化可能會提升壓縮
                        // 位元組減去序列和拼接——但是
                        // 使前列腺框架變得複雜，測量的成本是
                        // 已經可以接受了。
                        for target in &targets {
                            if let Some(session) = sessions.get(target) {
                                // 標記每個會話序列（單調，
                                // 無間隙－客戶使用這些來檢測損失
                                // 即使 AOI 可能會丟棄事件預標記）。
                                let seq_val = session.seq.fetch_add(1, Ordering::Relaxed);
                                let mut ev = event_template.clone();
                                ev.sequence = seq_val;
                                let payload = ev.encode_to_vec();

                                // 當 ≥ 閾值且
                                // 壓縮位元組更小；
                                // 否則會回到原始狀態。
                                // 與上面的 write_framed 保持同步。
                                let (frame_tag, frame_payload): (u8, std::borrow::Cow<'_, [u8]>) =
                                    if payload.len() >= LZ4_THRESHOLD {
                                        let c = lz4_flex::block::compress_prepend_size(&payload);
                                        if c.len() < payload.len() {
                                            (TAG_GAME_EVENT | COMPRESSION_FLAG, std::borrow::Cow::Owned(c))
                                        } else {
                                            (TAG_GAME_EVENT, std::borrow::Cow::Borrowed(&payload))
                                        }
                                    } else {
                                        (TAG_GAME_EVENT, std::borrow::Cow::Borrowed(&payload))
                                    };

                                let mut frame = Vec::with_capacity(1 + 4 + frame_payload.len());
                                frame.push(frame_tag);
                                frame.extend_from_slice(&(frame_payload.len() as u32).to_be_bytes());
                                frame.extend_from_slice(&frame_payload);

                                // 記錄每個會話觀察到的線字節，以便
                                // 計數器反映真實的線量（N個會話
                                // × 每 1 個編碼訊框）。
                                counter_broadcast.record(&msg_type, &action, frame.len());

                                let frame_arc: Arc<[u8]> = Arc::from(frame.into_boxed_slice());
                                if session.event_tx.try_send(frame_arc).is_err() {
                                    to_remove.push(target.clone());
                                } else {
                                    route_hits += 1;
                                }
                            }
                        }
                        if is_per_player_topic {
                            log::debug!("📡 routed per-player topic='{}' policy={:?} hits={}/{} (sessions={})",
                                msg.topic, msg.policy.as_ref().map(|_| "explicit").unwrap_or("legacy"),
                                route_hits, targets.len(), sessions.len());
                        }
                        drop(sessions);

                        if !to_remove.is_empty() {
                            let mut sessions = sessions_broadcast.lock().await;
                            for id in to_remove {
                                sessions.remove(&id);
                                info!("Removed disconnected KCP session: {}", id);
                            }
                        }
                    }
                }
            }
        });
    });

    // 解析綁定位址
    let bind_ip = match server_addr.as_str() {
        "localhost" | "127.0.0.1" => "0.0.0.0".to_string(),
        other => other.to_string(),
    };
    let addr = format!("{}:{}", bind_ip, server_port);
    let addr: std::net::SocketAddr = addr
        .parse()
        .map_err(|e| failure::err_msg(format!("Invalid address '{}': {}", addr, e)))?;

    let mut config = KcpConfig::default();
    config.nodelay = KcpNoDelayConfig::fastest();

    info!("Starting KCP server on {}", addr);

    let sessions_accept = sessions.clone();
    let in_tx_accept = in_tx.clone();
    let query_tx_accept = query_tx.clone();
    let viewport_tx_accept = viewport_tx.clone();
    let out_tx_accept = out_tx.clone();
    let lockstep_input_buffer_accept = lockstep_input_buffer.clone();
    let lockstep_state_accept = lockstep_state.clone();
    let lockstep_snapshot_store_accept = lockstep_snapshot_store.clone();

    // 同步綁定，因此如果連接埠被過時的實例佔用，啟動會快速失敗。
    let mut listener = KcpListener::bind(config, addr)
        .await
        .map_err(|e| failure::err_msg(format!("Failed to bind KCP listener on {}: {}", addr, e)))?;

    tokio::spawn(async move {
        loop {
            let (stream, peer_addr) = match listener.accept().await {
                Ok(s) => s,
                Err(e) => {
                    error!("KCP accept error: {}", e);
                    continue;
                }
            };

            info!("KCP client connected from {}", peer_addr);

            let sessions = sessions_accept.clone();
            let in_tx = in_tx_accept.clone();
            let query_tx = query_tx_accept.clone();
            let viewport_tx = viewport_tx_accept.clone();
            let out_tx = out_tx_accept.clone();
            let lockstep_input_buffer = lockstep_input_buffer_accept.clone();
            let lockstep_state = lockstep_state_accept.clone();
            let lockstep_snapshot_store = lockstep_snapshot_store_accept.clone();
            let session_id = format!("kcp_{}", peer_addr);

            tokio::spawn(async move {
                if let Err(e) = handle_client(
                    stream,
                    session_id,
                    sessions,
                    in_tx,
                    query_tx,
                    viewport_tx,
                    out_tx,
                    lockstep_input_buffer,
                    lockstep_state,
                    lockstep_snapshot_store,
                )
                .await
                {
                    warn!("KCP client handler error: {}", e);
                }
            });
        }
    });

    Ok(TransportHandle {
        tx: out_tx,
        rx: in_rx,
        query_rx,
        viewport_rx,
        counter,
        aoi,
    })
}

async fn handle_client(
    stream: KcpStream,
    session_id: String,
    sessions: Arc<Mutex<HashMap<String, ClientSession>>>,
    in_tx: Sender<InboundMsg>,
    query_tx: Sender<QueryRequest>,
    viewport_tx: Sender<ViewportMsg>,
    out_tx: Sender<OutboundMsg>,
    lockstep_input_buffer: Arc<std::sync::Mutex<crate::lockstep::InputBuffer>>,
    lockstep_state: Arc<std::sync::Mutex<crate::lockstep::LockstepState>>,
    lockstep_snapshot_store: Arc<std::sync::Mutex<crate::comp::SnapshotStore>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (mut reader, mut writer) = tokio::io::split(stream);

    // 每個會話的出站通道（惰性 — 僅在 SubscribeRequest 之後使用）。
    // P5：有效負載是 `Arc<[u8]>` — 廣播線程編碼的相同位元組
    // 在所有收件者會話之間共用。每個會話的作者任務
    // 取消引用 Arc 並將切片寫入 KCP — 無副本。
    let mut event_rx: Option<tokio::sync::mpsc::Receiver<Arc<[u8]>>> = None;
    let mut subscribed = false;
    // 追蹤訂閱的player_name，以便我們可以在斷開連接時發送刪除
    let mut player_name: Option<String> = None;

    // 主循環：從客戶端讀取，可選擇寫入出站事件
    loop {
        tokio::select! {
            result = read_framed(&mut reader) => {
                match result {
                    Ok(Some((tag, payload, _wire_bytes))) => {
                        match tag {
                            TAG_SUBSCRIBE_REQUEST => {
                                if let Ok(sub) = SubscribeRequest::decode(payload.as_slice()) {
                                    info!("🔌 KCP client subscribed as '{}' (session_id={})", sub.player_name, session_id);
                                    let (event_tx, rx) = tokio::sync::mpsc::channel::<Arc<[u8]>>(10000);
                                    event_rx = Some(rx);
                                    subscribed = true;
                                    player_name = Some(sub.player_name.clone());
                                    let mut sess = sessions.lock().await;
                                    sess.insert(
                                        session_id.clone(),
                                        ClientSession {
                                            player_name: sub.player_name,
                                            event_tx,
                                            viewport: None,
                                            // P6：每會話單調序列
                                            // 訂閱時從 0 開始；這
                                            // 第一個 GameEvent 發送至
                                            // 此會話攜帶序列=0。
                                            seq: Arc::new(AtomicU64::new(0)),
                                            // 第 2 階段：舊版 SubscribeRequest
                                            // 路徑 — 客戶端不在
                                            // 鎖步流直到發送
                                            // 加入請求 (0x13)。
                                            lockstep_joined: false,
                                        },
                                    );
                                }
                            }
                            TAG_PLAYER_COMMAND => {
                                if let Ok(cmd) = PlayerCommand::decode(payload.as_slice()) {
                                    let data_json: serde_json::Value = if cmd.data_json.is_empty() {
                                        serde_json::Value::Null
                                    } else {
                                        serde_json::from_slice(&cmd.data_json)
                                            .unwrap_or(serde_json::Value::Null)
                                    };

                                    let inbound = InboundMsg {
                                        name: cmd.player_name,
                                        t: cmd.msg_type,
                                        a: cmd.action,
                                        d: data_json,
                                    };

                                    let _ = in_tx.send(inbound);

                                    // 發送確認
                                    let ack = CommandAck {
                                        ok: true,
                                        message: "Command accepted".into(),
                                    };
                                    let ack_payload = ack.encode_to_vec();
                                    let _ = write_framed(&mut writer, TAG_COMMAND_ACK, &ack_payload).await;
                                }
                            }
                            TAG_GAME_STATE_REQUEST => {
                                if let Ok(req) = GameStateRequest::decode(payload.as_slice()) {
                                    // P6：處理客戶端發起的“seq-gap”
                                    // 重新同步。 player_name 帶有
                                    // 最後已知的 seq 作為十進位字串（到
                                    // 避免原型模式碰撞）。現在我們
                                    // 只是 LOG + ACK — 完整狀態快照
                                    // 答覆推遲到後續行動。
                                    // 發現差距的客戶將重試
                                    // 定期請求，直到伺服器
                                    // 有機地趕上。
                                    if req.query_type == "seq-gap" {
                                        warn!(
                                            "⚠️ seq-gap resync request from session={} last_seq={:?}",
                                            session_id, req.player_name
                                        );
                                        // 存根：ACK，以便客戶端知道伺服器
                                        // 收到請求。未來的補丁
                                        // 應該建立一個完整的視圖快照
                                        // （AOI 中的英雄/小兵/塔）和船隻
                                        // 它作為批量重播返回。
                                        let resp = GameStateResponse {
                                            success: true,
                                            error: String::new(),
                                            data_json: b"{\"stub\":true,\"note\":\"seq-gap snapshot not yet implemented - server logged request\"}".to_vec(),
                                        };
                                        let resp_payload = resp.encode_to_vec();
                                        let _ = write_framed(&mut writer, TAG_GAME_STATE_RESPONSE, &resp_payload).await;
                                        continue;
                                    }
                                    let (resp_tx, resp_rx) = tokio::sync::oneshot::channel();
                                    let query = QueryRequest {
                                        query_type: req.query_type,
                                        player_name: req.player_name,
                                        response_tx: resp_tx,
                                    };

                                    let _ = query_tx.send(query);

                                    if let Ok(response) = resp_rx.await {
                                        let resp = GameStateResponse {
                                            success: response.success,
                                            error: response.error,
                                            data_json: response.data_json,
                                        };
                                        let resp_payload = resp.encode_to_vec();
                                        let _ = write_framed(&mut writer, TAG_GAME_STATE_RESPONSE, &resp_payload).await;
                                    }
                                }
                            }
                            TAG_VIEWPORT_UPDATE => {
                                if let Ok(vp) = ViewportUpdate::decode(payload.as_slice()) {
                                    if subscribed {
                                        let viewport = Viewport::new(
                                            vp.center_x, vp.center_y, vp.half_width, vp.half_height,
                                        );
                                        let mut sess = sessions.lock().await;
                                        if let Some(s) = sess.get_mut(&session_id) {
                                            info!("🎥 Viewport update from '{}': center=({}, {}), half=({}, {}), padded=({}, {})",
                                                s.player_name, vp.center_x, vp.center_y,
                                                vp.half_width, vp.half_height,
                                                viewport.padded_hw, viewport.padded_hh);
                                            s.viewport = Some(viewport);
                                            // 通知遊戲循環，以便可見性差異可以使用它
                                            match viewport_tx.send(ViewportMsg::Set {
                                                player_name: s.player_name.clone(),
                                                viewport,
                                            }) {
                                                Ok(()) => info!("📤 Forwarded ViewportMsg::Set('{}') to game loop", s.player_name),
                                                Err(e) => warn!("Failed to forward ViewportMsg: {}", e),
                                            }
                                        } else {
                                            warn!("Viewport update but session '{}' not found", session_id);
                                        }
                                    } else {
                                        warn!("Viewport update before subscribe — ignored");
                                    }
                                } else {
                                    warn!("Failed to decode ViewportUpdate payload");
                                }
                            }
                            // ===== 第 2 階段鎖步標籤 =====
                            TAG_INPUT_SUBMIT => {
                                match InputSubmit::decode(payload.as_slice()) {
                                    Ok(req) => {
                                        let current_tick = lockstep_state.lock().unwrap().current_tick;
                                        let player_id = req.player_id;
                                        let target_tick = req.target_tick;
                                        let input_id = req.input_id;
                                        let input = req.input.unwrap_or_default();
                                        let accepted = lockstep_input_buffer
                                            .lock()
                                            .unwrap()
                                            .submit(current_tick, player_id, target_tick, input, input_id);
                                        if !accepted {
                                            warn!(
                                                "late InputSubmit from player {} input_id={} target_tick={} current_tick={}",
                                                player_id, input_id, target_tick, current_tick
                                            );
                                        }
                                    }
                                    Err(e) => warn!("Failed to decode InputSubmit: {}", e),
                                }
                            }
                            TAG_JOIN_REQUEST => {
                                match JoinRequest::decode(payload.as_slice()) {
                                    Ok(req) => {
                                        let role = match req.role {
                                            x if x == JoinRole::RoleObserver as i32 => crate::lockstep::JoinRoleEnum::Observer,
                                            _ => crate::lockstep::JoinRoleEnum::Player,
                                        };
                                        let (player_id, master_seed, start_tick) = {
                                            let mut s = lockstep_state.lock().unwrap();
                                            let pid = s.register_player(req.player_name.clone(), role);
                                            (pid, s.master_seed, s.current_tick)
                                        };
                                        // 將此會話標記為已加入
                                        // 鎖步流因此未來 TickBatch /
                                        // StateHash 廣播到達它。
                                        {
                                            let mut sess = sessions.lock().await;
                                            if let Some(s) = sess.get_mut(&session_id) {
                                                s.lockstep_joined = true;
                                                if s.player_name.is_empty() {
                                                    s.player_name = req.player_name.clone();
                                                }
                                            } else {
                                                // 沒有事先訂閱請求 —
                                                // 懶惰地創建會話
                                                // 僅鎖步客戶端（無
                                                // 舊版 GameEvent 頻道）可以
                                                // 仍然收到 TickBatch。
                                                let (event_tx, rx) = tokio::sync::mpsc::channel::<Arc<[u8]>>(10000);
                                                event_rx = Some(rx);
                                                subscribed = true;
                                                player_name = Some(req.player_name.clone());
                                                sess.insert(
                                                    session_id.clone(),
                                                    ClientSession {
                                                        player_name: req.player_name.clone(),
                                                        event_tx,
                                                        viewport: None,
                                                        seq: Arc::new(AtomicU64::new(0)),
                                                        lockstep_joined: true,
                                                    },
                                                );
                                            }
                                        }
                                        info!(
                                            "🎮 KCP lockstep JoinRequest player='{}' role={:?} → assigned player_id={} (session={})",
                                            req.player_name, role, player_id, session_id
                                        );
                                        // 透過單播方式傳送 GameStart
                                        // 廣播線程（所以它通過
                                        // 相同的每會話 event_tx
                                        // 客戶正在閱讀）。
                                        let game_start = GameStart {
                                            player_id,
                                            start_tick,
                                            master_seed,
                                            initial_state: Some(SimSnapshot {
                                                world_bytes: vec![],
                                                schema_version: 1,
                                            }),
                                        };
                                        let frame = crate::lockstep::LockstepFrame::GameStart {
                                            client_session_id: session_id.clone(),
                                            msg: game_start,
                                        };
                                        if let Err(e) = out_tx.send(OutboundMsg::lockstep_frame(frame)) {
                                            warn!("Failed to enqueue GameStart: {}", e);
                                        }
                                    }
                                    Err(e) => warn!("Failed to decode JoinRequest: {}", e),
                                }
                            }
                            TAG_SNAPSHOT_REQ => {
                                match SnapshotReq::decode(payload.as_slice()) {
                                    Ok(req) => {
                                        // 階段5.3：服務最新
                                        // bincode 序列化的世界快照
                                        // 來自共享 SnapshotStore。這
                                        // 調度程序滴答循環刷新此
                                        // 每 SNAPSHOT_INTERVAL_TICKS
                                        // （= 30 秒 @ 30 赫茲）。空字節的意思
                                        // 尚未拍攝任何快照 —
                                        // 觀察者重新開始玩耍
                                        // 從 `current_tick` 轉發，不含
                                        // 引導程式。
                                        let (snapshot_tick, snapshot_bytes) = {
                                            let store = lockstep_snapshot_store
                                                .lock()
                                                .expect("SnapshotStore mutex poisoned");
                                            (store.tick, store.bytes.clone())
                                        };
                                        let current_tick = lockstep_state.lock().unwrap().current_tick;
                                        info!(
                                            "📸 SnapshotReq from {} for from_tick={} → serving snapshot tick={} bytes={} (current_tick={})",
                                            session_id, req.from_tick, snapshot_tick, snapshot_bytes.len(), current_tick
                                        );
                                        let resp = SnapshotResp {
                                            tick: snapshot_tick,
                                            state: Some(SimSnapshot {
                                                world_bytes: snapshot_bytes,
                                                schema_version: crate::lockstep::SNAPSHOT_SCHEMA_VERSION,
                                            }),
                                        };
                                        let frame = crate::lockstep::LockstepFrame::SnapshotResp {
                                            client_session_id: session_id.clone(),
                                            msg: resp,
                                        };
                                        if let Err(e) = out_tx.send(OutboundMsg::lockstep_frame(frame)) {
                                            warn!("Failed to enqueue SnapshotResp: {}", e);
                                        }
                                    }
                                    Err(e) => warn!("Failed to decode SnapshotReq: {}", e),
                                }
                            }
                            TAG_PING_REQ => {
                                // 使用相同的 client_send_us 回顯 PingResponse
                                // 這樣客戶端就可以得到RTT。直接寫——
                                // 繞過廣播頻道以達到最低限度
                                // 週轉時間（out_tx 增加排隊延遲
                                // 污染 RTT 測量）。
                                match PingRequest::decode(payload.as_slice()) {
                                    Ok(req) => {
                                        let resp = PingResponse {
                                            client_send_us: req.client_send_us,
                                        };
                                        let resp_payload = resp.encode_to_vec();
                                        let _ = write_framed(&mut writer, TAG_PING_RESP, &resp_payload).await;
                                    }
                                    Err(e) => warn!("Failed to decode PingRequest: {}", e),
                                }
                            }
                            _ => {
                                warn!("Unknown tag from client: 0x{:02x}", tag);
                            }
                        }
                    }
                    Ok(None) => {
                        info!("KCP client disconnected: {}", session_id);
                        break;
                    }
                    Err(e) => {
                        warn!("KCP read error for {}: {}", session_id, e);
                        break;
                    }
                }
            }
            Some(frame) = async {
                match event_rx.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                // P5：“frame”是與所有其他會話共享的“Arc<[u8]>”
                // 發生了同樣的事件。寫入底層切片
                // 複製到 KCP 套接字緩衝區，但遊戲端
                // 分配被重新計數－在最後一個會話時被刪除
                // 臉紅。
                if writer.write_all(&frame).await.is_err() {
                    break;
                }
                let _ = writer.flush().await;
            }
        }
    }

    // 清理會議
    {
        let mut sess = sessions.lock().await;
        sess.remove(&session_id);
    }
    // 通知遊戲循環該玩家的視窗已消失
    if let Some(name) = player_name {
        let _ = viewport_tx.send(ViewportMsg::Remove { player_name: name });
    }
    info!("KCP session cleaned up: {}", session_id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make(t: &str, a: &str, v: serde_json::Value) -> OutboundMsg {
        OutboundMsg::new_s("td/all/res", t, a, v)
    }

    #[test]
    fn dedupe_collapses_creep_h_same_entity() {
        // 實體 42 的 3 個 Creep.H 更新應在一個視窗中到達
        // 折疊為攜帶最新 hp 值的單一訊息。
        let batch = vec![
            make(
                "creep",
                "H",
                json!({ "id": 42, "hp": 100.0, "max_hp": 200.0 }),
            ),
            make(
                "creep",
                "H",
                json!({ "id": 42, "hp": 80.0, "max_hp": 200.0 }),
            ),
            make(
                "creep",
                "H",
                json!({ "id": 42, "hp": 50.0, "max_hp": 200.0 }),
            ),
        ];
        let out = dedupe_batch(batch);
        assert_eq!(out.len(), 1);
        let parsed: serde_json::Value = serde_json::from_str(&out[0].msg).unwrap();
        assert_eq!(parsed["d"]["hp"].as_f64(), Some(50.0));
    }

    #[test]
    fn dedupe_preserves_different_entities() {
        let batch = vec![
            make(
                "creep",
                "H",
                json!({ "id": 42, "hp": 100.0, "max_hp": 200.0 }),
            ),
            make(
                "creep",
                "H",
                json!({ "id": 43, "hp":  90.0, "max_hp": 200.0 }),
            ),
        ];
        let out = dedupe_batch(batch);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn dedupe_preserves_different_actions() {
        // Creep.H 和 Creep.M 共用實體，但操作不同 → 都保留。
        let batch = vec![
            make(
                "creep",
                "H",
                json!({ "id": 42, "hp": 100.0, "max_hp": 200.0 }),
            ),
            make("creep", "M", json!({ "id": 42, "x": 1.0, "y": 2.0 })),
        ];
        let out = dedupe_batch(batch);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn non_dedupable_passes_through() {
        // 同一實體的兩個創建事件必須同時存在（創建
        // 是語意的，第二個可能攜帶真實數據，而
        // 首先只是一個佔位符——無論如何，我們不能崩潰）。
        let batch = vec![
            make("creep", "C", json!({ "id": 42, "kind": "orc" })),
            make("creep", "C", json!({ "id": 42, "kind": "orc" })),
        ];
        let out = dedupe_batch(batch);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn unknown_kind_passes_through() {
        // 格式錯誤的 JSON → peek 回傳 ("", "", 0)，重複資料刪除會跳過它（不驚慌）。
        // 未知（msg_type，action）對 → 不在 is_dedupable 中 → 傳遞。
        let mut raw_bad = OutboundMsg::new_s("td/all/res", "x", "y", json!({}));
        raw_bad.msg = "not json at all }}}".to_string();

        let batch = vec![
            raw_bad,
            make("game", "lives", json!({ "lives": 3 })), // unknown kind per policy
            make("buff", "buff_add", json!({ "id": 42, "buff": "slow" })),
        ];
        let out = dedupe_batch(batch);
        assert_eq!(out.len(), 3);
    }

    // ===== P5 BroadcastPolicy 調度測試 =====

    use std::collections::BTreeMap;

    fn mk_sessions(
        entries: &[(&str, &str, Option<Viewport>)],
    ) -> BTreeMap<String, (String, Option<Viewport>)> {
        entries
            .iter()
            .map(|(id, name, vp)| (id.to_string(), (name.to_string(), *vp)))
            .collect()
    }

    #[test]
    fn policy_all_reaches_every_session() {
        let sessions = mk_sessions(&[
            ("s1", "alice", Some(Viewport::new(0.0, 0.0, 100.0, 100.0))),
            (
                "s2",
                "bob",
                Some(Viewport::new(1000.0, 1000.0, 100.0, 100.0)),
            ),
            ("s3", "carol", None),
        ]);
        let targets = select_targets_for_policy(
            Some(&BroadcastPolicy::All),
            "td/all/res",
            None,
            &sessions,
            &|_| None,
        );
        assert_eq!(targets.len(), 3);
    }

    #[test]
    fn policy_player_only_hits_one_session() {
        let sessions = mk_sessions(&[("s1", "alice", None), ("s2", "bob", None)]);
        let targets = select_targets_for_policy(
            Some(&BroadcastPolicy::PlayerOnly("bob".into())),
            "td/bob/res",
            None,
            &sessions,
            &|_| None,
        );
        assert_eq!(targets, vec!["s2".to_string()]);
    }

    #[test]
    fn policy_aoi_point_filters_by_viewport() {
        let sessions = mk_sessions(&[
            ("s1", "alice", Some(Viewport::new(0.0, 0.0, 100.0, 100.0))),
            (
                "s2",
                "bob",
                Some(Viewport::new(1000.0, 1000.0, 100.0, 100.0)),
            ),
            ("s3", "no_vp", None),
        ]);
        // (10, 10) 處的事件 — alice 看到了，bob 沒看到，no_vp 透過
        // （策略將丟失的視口視為“尚未過濾”，因此心跳
        // /初始狀態仍然達到他們）。
        let targets = select_targets_for_policy(
            Some(&BroadcastPolicy::AoiPoint(10.0, 10.0)),
            "td/all/res",
            None,
            &sessions,
            &|_| None,
        );
        let mut sorted = targets.clone();
        sorted.sort();
        assert_eq!(sorted, vec!["s1".to_string(), "s3".to_string()]);
    }

    #[test]
    fn policy_aoi_entity_uses_grid_lookup() {
        let sessions = mk_sessions(&[
            (
                "s1",
                "alice",
                Some(Viewport::new(500.0, 500.0, 100.0, 100.0)),
            ),
            ("s2", "bob", Some(Viewport::new(0.0, 0.0, 100.0, 100.0))),
        ]);
        // 實體 42 位於 (500, 500) — 只有 Alice 的視口包含它。
        let lookup = |eid: u64| {
            if eid == 42 {
                Some((500.0, 500.0))
            } else {
                None
            }
        };
        let targets = select_targets_for_policy(
            Some(&BroadcastPolicy::AoiEntity(42)),
            "td/all/res",
            None,
            &sessions,
            &lookup,
        );
        assert_eq!(targets, vec!["s1".to_string()]);
    }

    #[test]
    fn policy_aoi_entity_unknown_falls_back_to_broadcast() {
        let sessions = mk_sessions(&[
            (
                "s1",
                "alice",
                Some(Viewport::new(500.0, 500.0, 100.0, 100.0)),
            ),
            ("s2", "bob", Some(Viewport::new(0.0, 0.0, 100.0, 100.0))),
        ]);
        // 實體 999 未知 → 廣播到每個會話（安全後備）。
        let targets = select_targets_for_policy(
            Some(&BroadcastPolicy::AoiEntity(999)),
            "td/all/res",
            None,
            &sessions,
            &|_| None,
        );
        assert_eq!(targets.len(), 2);
    }

    #[test]
    fn policy_none_preserves_legacy_topic_routing() {
        let sessions = mk_sessions(&[
            ("s1", "alice", Some(Viewport::new(0.0, 0.0, 100.0, 100.0))),
            (
                "s2",
                "bob",
                Some(Viewport::new(1000.0, 1000.0, 100.0, 100.0)),
            ),
        ]);
        // 舊版 /all/ topic +entity_pos → 應用視窗過濾器。
        // (0, 0) 處的事件 — alice 包含，bob 不包含。
        let targets =
            select_targets_for_policy(None, "td/all/res", Some((0.0, 0.0)), &sessions, &|_| None);
        assert_eq!(targets, vec!["s1".to_string()]);

        // 每個玩家的主題「td/bob/res」 → 僅 bob 的會話。
        let targets = select_targets_for_policy(None, "td/bob/res", None, &sessions, &|_| None);
        assert_eq!(targets, vec!["s2".to_string()]);
    }

    #[test]
    fn policy_none_no_pos_reaches_all_matching_topic() {
        let sessions = mk_sessions(&[
            ("s1", "alice", Some(Viewport::new(0.0, 0.0, 100.0, 100.0))),
            (
                "s2",
                "bob",
                Some(Viewport::new(1000.0, 1000.0, 100.0, 100.0)),
            ),
        ]);
        // /all/ topic + 無entity_pos → 每個會話都通過。
        let targets = select_targets_for_policy(None, "td/all/res", None, &sessions, &|_| None);
        assert_eq!(targets.len(), 2);
    }

    // ===== P6 兩層批次視窗測試 =====
    //
    // 我們不是旋轉整個運行時，而是將演算法提取到
    // 小幫手，使用 crossbeam 接收器並返回收集到的數據
    // 批。這讓我們可以提供合成（定時）輸入並斷言刷新
    // 定時。與上面的真實廣播循環保持同步。
    use crossbeam_channel::{bounded, RecvTimeoutError};
    use std::time::{Duration, Instant};

    fn collect_batch(
        rx: &crossbeam_channel::Receiver<OutboundMsg>,
        min_batch: Duration,
        max_batch: Duration,
    ) -> Option<Vec<OutboundMsg>> {
        let first = rx.recv().ok()?;
        let first_urg = outbound_urgency(&first);
        let mut batch = vec![first];
        let start = Instant::now();
        if first_urg == Urgency::Urgent {
            while let Ok(m) = rx.try_recv() {
                batch.push(m);
            }
            return Some(batch);
        }
        loop {
            let now = Instant::now();
            let elapsed = now.duration_since(start);
            if elapsed >= max_batch {
                break;
            }
            let timeout = max_batch - elapsed;
            match rx.recv_timeout(timeout) {
                Ok(m) => {
                    let is_lockstep = m.lockstep_frame.is_some();
                    let is_urg = outbound_urgency(&m) == Urgency::Urgent;
                    batch.push(m);
                    if is_lockstep || (is_urg && start.elapsed() >= min_batch) {
                        break;
                    }
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        Some(batch)
    }

    #[test]
    fn urgent_first_flushes_immediately() {
        // 第一個訊息是緊急的 → 演算法耗盡就緒訊息並返回。
        // 即使 MIN_BATCH 為 10 毫秒，也必須在 MIN_BATCH 下順利完成。
        let (tx, rx) = bounded::<OutboundMsg>(32);
        tx.send(make("creep", "D", json!({ "id": 42 }))).unwrap();
        tx.send(make("creep", "H", json!({ "id": 42, "hp": 10.0 })))
            .unwrap();
        let t0 = Instant::now();
        let batch =
            collect_batch(&rx, Duration::from_millis(10), Duration::from_millis(33)).unwrap();
        let elapsed = t0.elapsed();
        assert!(
            elapsed < Duration::from_millis(5),
            "urgent flush took {:?}",
            elapsed
        );
        // 兩則訊息均已傳入（普通訊息透過 try_recv 進行捎帶）。
        assert_eq!(batch.len(), 2);
    }

    #[test]
    fn lockstep_frame_flushes_immediately() {
        use crate::lockstep::{LockstepFrame, TickBatch};

        let (tx, rx) = bounded::<OutboundMsg>(32);
        tx.send(OutboundMsg::lockstep_frame(LockstepFrame::TickBatch(
            TickBatch {
                tick: 1,
                inputs: Vec::new(),
                server_events: Vec::new(),
            },
        )))
        .unwrap();
        let t0 = Instant::now();
        let batch =
            collect_batch(&rx, Duration::from_millis(10), Duration::from_millis(33)).unwrap();
        let elapsed = t0.elapsed();
        assert!(
            elapsed < Duration::from_millis(5),
            "lockstep flush took {:?}",
            elapsed
        );
        assert_eq!(batch.len(), 1);
    }

    #[test]
    fn lockstep_frame_short_circuits_normal_batch() {
        use crate::lockstep::{LockstepFrame, TickBatch};

        let (tx, rx) = bounded::<OutboundMsg>(32);
        tx.send(make("creep", "H", json!({ "id": 42, "hp": 10.0 })))
            .unwrap();
        tx.send(OutboundMsg::lockstep_frame(LockstepFrame::TickBatch(
            TickBatch {
                tick: 1,
                inputs: Vec::new(),
                server_events: Vec::new(),
            },
        )))
        .unwrap();
        let t0 = Instant::now();
        let batch =
            collect_batch(&rx, Duration::from_millis(10), Duration::from_millis(33)).unwrap();
        let elapsed = t0.elapsed();
        assert!(
            elapsed < Duration::from_millis(5),
            "lockstep did not short-circuit: {:?}",
            elapsed
        );
        assert_eq!(batch.len(), 2);
    }

    #[test]
    fn normal_first_waits_at_least_min_batch() {
        // 當頭部正常且沒有緊急情況到達時，我們堅持到
        // 最大批。在第一個訊息之後，我們點擊了一個空通道
        // 恰好在 MAX_BATCH 處超時（模 OS 調度程式 slop）。
        let (tx, rx) = bounded::<OutboundMsg>(32);
        tx.send(make("creep", "H", json!({ "id": 42, "hp": 10.0 })))
            .unwrap();
        let t0 = Instant::now();
        let batch =
            collect_batch(&rx, Duration::from_millis(10), Duration::from_millis(33)).unwrap();
        let elapsed = t0.elapsed();
        // 預期 ≥ MAX_BATCH（在寬鬆的容差範圍內 - Windows 調度程序
        // 量子約為 15ms，因此下限是我們真正關心的）。
        assert!(
            elapsed >= Duration::from_millis(25),
            "normal flush fired too early: {:?}",
            elapsed
        );
        assert_eq!(batch.len(), 1);
    }

    #[test]
    fn urgent_after_min_batch_short_circuits() {
        // 正常頭，然後在 MIN_BATCH 後緊急到達 → 應該刷新
        // 在 MAX_BATCH 之前。驅動一個單獨的線程來傳遞緊急的
        // 訊息約 15 毫秒。
        let (tx, rx) = bounded::<OutboundMsg>(32);
        tx.send(make("creep", "H", json!({ "id": 42, "hp": 10.0 })))
            .unwrap();
        let tx2 = tx.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(15));
            tx2.send(make("creep", "D", json!({ "id": 42 }))).unwrap();
        });
        let t0 = Instant::now();
        let batch =
            collect_batch(&rx, Duration::from_millis(10), Duration::from_millis(33)).unwrap();
        let elapsed = t0.elapsed();
        // 必須在 Urgent 到達之後（~15ms）但在 MAX_BATCH（33ms）之前刷新。
        assert!(
            elapsed < Duration::from_millis(30),
            "didn't short-circuit on Urgent: {:?}",
            elapsed
        );
        assert!(
            elapsed >= Duration::from_millis(14),
            "flushed before MIN_BATCH: {:?}",
            elapsed
        );
        assert_eq!(batch.len(), 2);
    }

    #[test]
    fn urgent_before_min_batch_still_holds() {
        // 如果緊急情況在 MIN_BATCH 之前到達，我們將繼續進行批次處理，直到
        // 最少 MIN_BATCH — 緊急情況僅為一次短路訊號
        // 我們已經攤銷了批次成本。
        let (tx, rx) = bounded::<OutboundMsg>(32);
        tx.send(make("creep", "H", json!({ "id": 42, "hp": 10.0 })))
            .unwrap();
        let tx2 = tx.clone();
        std::thread::spawn(move || {
            // 緊急抵達時間約 3 毫秒 — 完全在 MIN_BATCH 之內。
            std::thread::sleep(Duration::from_millis(3));
            tx2.send(make("creep", "D", json!({ "id": 42 }))).unwrap();
        });
        let t0 = Instant::now();
        let batch =
            collect_batch(&rx, Duration::from_millis(10), Duration::from_millis(33)).unwrap();
        let elapsed = t0.elapsed();
        // 即使看到“緊急”，也應保持批次超過 MIN_BATCH
        // （然後點擊 MAX_BATCH，因為沒有其他東西到達）。
        assert!(
            elapsed >= Duration::from_millis(25),
            "flushed on early Urgent: {:?}",
            elapsed
        );
        assert_eq!(batch.len(), 2);
    }

    #[test]
    fn dedupe_preserves_order_for_mixed_traffic() {
        // 不可重複資料刪除保持其地位；重複資料刪除保留其首次出現的位置。
        let batch = vec![
            make("creep", "H", json!({ "id": 42, "hp": 100.0 })), // slot 0
            make("game", "lives", json!({ "lives": 3 })),         // slot 1 (pass-through)
            make("creep", "H", json!({ "id": 42, "hp": 50.0 })),  // dedupes into slot 0
            make("creep", "M", json!({ "id": 42, "x": 1.0, "y": 2.0 })), // slot 2
        ];
        let out = dedupe_batch(batch);
        assert_eq!(out.len(), 3);
        let first: serde_json::Value = serde_json::from_str(&out[0].msg).unwrap();
        assert_eq!(first["t"].as_str(), Some("creep"));
        assert_eq!(first["a"].as_str(), Some("H"));
        assert_eq!(first["d"]["hp"].as_f64(), Some(50.0)); // latest value
        let second: serde_json::Value = serde_json::from_str(&out[1].msg).unwrap();
        assert_eq!(second["t"].as_str(), Some("game"));
        let third: serde_json::Value = serde_json::from_str(&out[2].msg).unwrap();
        assert_eq!(third["t"].as_str(), Some("creep"));
        assert_eq!(third["a"].as_str(), Some("M"));
    }

    // ===== 第 2 階段鎖步往返測試 =====

    #[test]
    fn lockstep_input_submit_decode_roundtrip() {
        // 對 InputSubmit 進行編碼，然後解碼 + 斷言欄位。煙霧測試
        // 每個客戶端讀取循環的「InputSubmit::decode(payload.as_slice())」。
        let original = InputSubmit {
            player_id: 42,
            target_tick: 1234,
            input: Some(PlayerInput {
                action: Some(player_input::Action::NoOp(NoOp {})),
            }),
            input_id: 99,
        };
        let bytes = original.encode_to_vec();
        let decoded = InputSubmit::decode(bytes.as_slice()).expect("decode");
        assert_eq!(decoded.player_id, 42);
        assert_eq!(decoded.target_tick, 1234);
        assert_eq!(decoded.input_id, 99);
        assert!(decoded.input.is_some());
    }

    #[test]
    fn lockstep_join_request_role_mapping() {
        // ROLE_PLAYER (1) 和 ROLE_OBSERVER (2) 都應該往返；這
        // 伺服器的比賽臂（在handle_client中）將未知的整數視為玩家。
        let player = JoinRequest {
            player_name: "alice".into(),
            role: JoinRole::RolePlayer as i32,
        };
        let observer = JoinRequest {
            player_name: "bob".into(),
            role: JoinRole::RoleObserver as i32,
        };
        let p_bytes = player.encode_to_vec();
        let o_bytes = observer.encode_to_vec();
        let p_dec = JoinRequest::decode(p_bytes.as_slice()).unwrap();
        let o_dec = JoinRequest::decode(o_bytes.as_slice()).unwrap();
        assert_eq!(p_dec.role, JoinRole::RolePlayer as i32);
        assert_eq!(o_dec.role, JoinRole::RoleObserver as i32);
        assert_eq!(p_dec.player_name, "alice");
        assert_eq!(o_dec.player_name, "bob");
    }

    #[test]
    fn lockstep_build_framed_bytes_layout() {
        // build_framed_bytes 必須產生 [tag][4B BE len][payload] 匹配
        // 現有的 write_framed 有線格式。低於閾值=無壓縮。
        let payload = b"hello".to_vec();
        let frame = build_framed_bytes(TAG_TICK_BATCH, &payload);
        assert_eq!(frame[0], TAG_TICK_BATCH);
        assert_eq!(frame[0] & COMPRESSION_FLAG, 0);
        let len = u32::from_be_bytes(frame[1..5].try_into().unwrap()) as usize;
        assert_eq!(len, payload.len());
        assert_eq!(&frame[5..], payload.as_slice());
    }

    #[test]
    fn lockstep_build_framed_bytes_compresses_large_redundant() {
        // 1KB 冗餘有效負載 → COMPRESSION_FLAG 設定 + 總線路位元組數下降。
        let payload = vec![0xABu8; 1000];
        let frame = build_framed_bytes(TAG_TICK_BATCH, &payload);
        assert_eq!(frame[0] & COMPRESSION_FLAG, COMPRESSION_FLAG);
        assert_eq!(frame[0] & 0x7F, TAG_TICK_BATCH);
        assert!(
            frame.len() < 500,
            "expected compressed frame < 500B, got {}",
            frame.len()
        );
    }
}
