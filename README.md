# Open MOBA Backend

一個使用 Rust 開發的高效能 MOBA 遊戲後端服務器，採用 ECS (Entity Component System) 架構，支援即時多人對戰、塔防機制和豐富的遊戲道具系統。

## 🎮 遊戲特色

- **ECS 架構設計**：使用 Specs ECS 框架實現高效能的遊戲邏輯處理
- **即時多人對戰**：基於 MQTT 協議的低延遲通信系統
- **豐富道具系統**：包含卷軸、藥劑、武器等多種類型道具
- **塔防機制**：支援防禦塔、小兵波次、投射物等核心遊戲機制
- **屬性系統**：力量/敏捷/智力三圍屬性系統，支援主屬性加成

## 🏗️ 技術架構

### 核心技術棧

- **語言**: Rust 2021 Edition
- **ECS 框架**: Specs
- **通信協議**: MQTT (使用 rumqttc 客戶端)
- **序列化**: Serde + JSON
- **並發處理**: Rayon 線程池
- **日誌系統**: log4rs
- **數學運算**: vek 向量數學庫

### 系統架構

```
open_moba_backend/
├── src/
│   ├── comp/           # ECS 組件定義
│   │   ├── attack.rs   # 攻擊系統
│   │   ├── base.rs     # 基地建築
│   │   ├── creep.rs    # 小兵系統
│   │   ├── player.rs   # 玩家組件
│   │   ├── tower.rs    # 防禦塔
│   │   ├── projectile.rs # 投射物
│   │   └── ...
│   ├── tick/           # 遊戲循環邏輯
│   │   ├── creep_tick.rs    # 小兵更新
│   │   ├── tower_tick.rs    # 防禦塔更新
│   │   ├── player_tick.rs   # 玩家更新
│   │   └── ...
│   ├── config/         # 配置管理
│   ├── ue4/           # UE4 地圖導入
│   └── main.rs        # 主程序入口
├── Cargo.toml         # 依賴配置
├── game.toml          # 遊戲配置
├── map.json           # 地圖數據
└── log4rs.yml         # 日誌配置
```

## 🚀 快速開始

### 環境要求

- Rust 1.70+ 
- MQTT Broker (如 Mosquitto)

### 安裝與運行

1. **克隆專案**
   ```bash
   git clone <repository-url>
   cd open_moba_backend
   ```

2. **配置遊戲參數**
   
   編輯 `game.toml` 文件：
   ```toml
   [server]
   MAP = "map.json"
   MAX_PLAYER = 10000
   SERVER_IP = "your-broker-ip"
   SERVER_PORT = "1883"
   CLIENT_ID = "specs_td"
   ```

3. **編譯並運行**
   ```bash
   cargo build --release
   cargo run --release
   ```

### 配置說明

#### game.toml 配置項

| 參數 | 說明 | 預設值 |
|------|------|--------|
| MAP | 地圖文件路徑 | "map.json" |
| MAX_PLAYER | 最大玩家數量 | 10000 |
| SERVER_IP | MQTT Broker IP | "45.32.32.40" |
| SERVER_PORT | MQTT Broker 端口 | "1883" |
| CLIENT_ID | MQTT 客戶端 ID | "specs_td" |

#### log4rs.yml 日誌配置

系統使用 log4rs 進行日誌管理，支援多級別日誌輸出和文件輪轉。

## 🔧 開發指南

### ECS 組件系統

系統採用 Specs ECS 架構，主要組件類型：

- **Pos**: 位置組件，包含 x, y, z 坐標
- **Vel**: 速度組件，控制實體移動
- **Health**: 生命值組件
- **Attack**: 攻擊能力組件
- **Player**: 玩家特有組件
- **Creep**: 小兵組件
- **Tower**: 防禦塔組件
- **Projectile**: 投射物組件

### 遊戲循環 (Tick System)

系統以 10 TPS (每秒10次更新) 運行，主要更新模塊：

1. **creep_tick**: 小兵 AI 和移動
2. **tower_tick**: 防禦塔攻擊邏輯
3. **player_tick**: 玩家操作處理
4. **projectile_tick**: 投射物物理計算
5. **nearby_tick**: 鄰近實體檢測

### MQTT 通信協議

- **訂閱主題**: `td/+/send`
- **發布格式**: JSON
- **QoS 等級**: AtMostOnce (0)

玩家數據格式：
```json
{
  "player_id": "string",
  "action": "move|attack|cast",
  "data": { ... }
}
```

### 添加新功能

1. **新組件**: 在 `src/comp/` 添加組件定義
2. **新系統**: 在 `src/tick/` 添加更新邏輯
3. **註冊組件**: 在 `comp/mod.rs` 中導出
4. **註冊系統**: 在相應的 tick 模塊中整合

## 📊 效能特色

- **多線程處理**: 使用 Rayon 線程池最大化 CPU 利用率
- **記憶體最佳化**: ECS 架構確保資料局部性
- **非同步通信**: MQTT 客戶端支援非阻塞 I/O
- **批次處理**: 遊戲邏輯批次更新減少開銷

## 🐛 除錯與監控

### 日誌系統

系統提供多層級日誌：
- **ERROR**: 嚴重錯誤
- **WARN**: 警告訊息
- **INFO**: 一般資訊
- **DEBUG**: 除錯資訊
- **TRACE**: 詳細追蹤

### 常見問題排除

1. **MQTT 連接失敗**
   - 檢查 `game.toml` 中的服務器配置
   - 確認 MQTT Broker 運行狀態
   - 檢查網路連通性

2. **效能問題**
   - 監控 CPU 使用率
   - 檢查記憶體洩漏
   - 調整線程池大小

3. **遊戲邏輯錯誤**
   - 啟用 DEBUG 日誌級別
   - 檢查 ECS 組件狀態
   - 驗證數據序列化

## 🤝 貢獻指南

1. Fork 本專案
2. 創建功能分支 (`git checkout -b feature/amazing-feature`)
3. 提交變更 (`git commit -m 'Add amazing feature'`)
4. 推送到分支 (`git push origin feature/amazing-feature`)
5. 創建 Pull Request

### 程式碼規範

- 遵循 Rust 官方程式碼風格
- 使用 `cargo fmt` 格式化程式碼
- 使用 `cargo clippy` 檢查程式碼品質
- 添加適當的單元測試

## 📄 授權條款

本專案採用 MIT 授權條款 - 詳見 [LICENSE](LICENSE) 文件

## 👥 開發團隊

- 主要開發者：[Your Name]
- 貢獻者：見 [CONTRIBUTORS.md](CONTRIBUTORS.md)

## 🔗 相關連結

- [Specs ECS 文檔](https://specs.amethyst.rs/)
- [MQTT 協議規範](https://mqtt.org/)
- [Rust 程式語言](https://www.rust-lang.org/)

---

如有任何問題或建議，歡迎提交 Issue 或聯繫開發團隊。