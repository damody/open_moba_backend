# Open MOBA Backend (omobab)

一個使用 Rust 開發的高效能 MOBA 遊戲後端服務器，採用 ECS (Entity Component System) 架構，支援即時多人對戰、塔防機制和豐富的遊戲道具系統。

> **專案名稱**: omobab (Open MOBA Backend 的縮寫)

## 🎮 遊戲特色

- **ECS 架構設計**：使用 Specs ECS 框架實現高效能的遊戲邏輯處理
- **即時多人對戰**：基於 MQTT 協議的低延遲通信系統
- **模組化架構**：🆕 完全重構的模組化設計，提升可維護性
- **獨立狀態管理**：🆕 State 模組完全獨立，支援彈性配置
- **高性能視野系統**：🆕 四叉樹空間分割與陰影計算優化
- **豐富道具系統**：包含卷軸、藥劑、武器等多種類型道具
- **塔防機制**：支援防禦塔、小兵波次、投射物等核心遊戲機制
- **屬性系統**：力量/敏捷/智力三圍屬性系統，支援主屬性加成
- **技能系統**：支援 JSON 配置的技能系統，可動態載入和擴展技能
- **事件驅動架構**：🆕 統一的事件分派系統處理複雜遊戲邏輯

## 🏗️ 技術架構

### 核心技術棧

- **語言**: Rust 2021 Edition
- **ECS 框架**: Specs (自定義版本，支援 Entity 序列化)
- **通信協議**: MQTT (使用 rumqttc 客戶端)
- **序列化**: Serde + JSON
- **並發處理**: Rayon 線程池 + voracious_radix_sort 多緒排序
- **日誌系統**: log4rs
- **數學運算**: vek 向量數學庫
- **技能系統**: ability-system 子系統 (支援 JSON 配置)

### 系統架構

```
omobab/
├── src/
│   ├── state/          # 🆕 獨立狀態管理模組 (重構後)
│   │   ├── core.rs     # 核心狀態結構
│   │   ├── initialization.rs  # ECS 世界初始化
│   │   ├── time_management.rs  # 時間管理系統
│   │   ├── resource_management.rs # 資源管理器
│   │   ├── system_dispatcher.rs   # 系統分派器
│   │   └── mod.rs      # 模組導出
│   ├── comp/           # ECS 組件定義
│   │   ├── attack.rs   # 攻擊系統
│   │   ├── base.rs     # 基地建築
│   │   ├── creep.rs    # 小兵系統
│   │   ├── player.rs   # 玩家組件
│   │   ├── tower.rs    # 防禦塔
│   │   ├── hero.rs     # 🆕 英雄系統（增強版）
│   │   ├── skill.rs    # 技能組件
│   │   ├── vision/     # 🆕 視野系統模組
│   │   │   ├── calculator.rs    # 視野計算核心
│   │   │   ├── components.rs    # 視野組件
│   │   │   ├── result_manager.rs # 視野結果管理
│   │   │   └── shadow_system.rs  # 陰影系統
│   │   ├── outcome_system/  # 🆕 事件系統模組
│   │   │   ├── event_dispatcher.rs # 事件分派器
│   │   │   ├── combat_events.rs    # 戰鬥事件
│   │   │   ├── movement_events.rs  # 移動事件
│   │   │   └── creation_events.rs  # 創建事件
│   │   └── state.rs    # 重新導出 (向後兼容)
│   ├── tick/           # 遊戲循環邏輯
│   │   ├── creep_tick.rs    # 小兵更新
│   │   ├── tower_tick.rs    # 防禦塔更新
│   │   ├── player_tick.rs   # 玩家更新
│   │   ├── skill_system/    # 🆕 技能系統模組
│   │   │   ├── abilities.rs     # 技能管理
│   │   │   ├── effects.rs       # 效果處理
│   │   │   ├── input_handler.rs # 輸入處理
│   │   │   └── processor.rs     # 技能處理器
│   │   └── skill_tick.rs    # 技能系統主入口
│   ├── vision/         # 🆕 視野計算系統
│   │   ├── shadow_calculator.rs # 陰影計算器
│   │   ├── quadtree.rs         # 四叉樹空間分割
│   │   ├── shadow_calculation.rs # 陰影計算邏輯
│   │   ├── vision_cache.rs     # 視野緩存
│   │   └── geometry_utils.rs   # 幾何工具
│   ├── config/         # 配置管理
│   ├── ue4/           # UE4 地圖導入
│   └── main.rs        # 主程序入口
├── ability-system/     # 技能子系統
│   ├── src/lib.rs     # 技能處理核心邏輯
│   └── Cargo.toml     # 子系統依賴
├── ability-configs/    # 技能配置檔案
│   └── sniper_abilities.json  # 狙擊手技能配置
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
   CLIENT_ID = "omobab"
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
| CLIENT_ID | MQTT 客戶端 ID | "omobab" |

#### log4rs.yml 日誌配置

系統使用 log4rs 進行日誌管理，支援多級別日誌輸出和文件輪轉。

## 🔧 開發指南

### ECS 組件系統

系統採用 Specs ECS 架構，主要組件類型：

- **Pos**: 位置組件，包含 x, y, z 坐標
- **Vel**: 速度組件，控制實體移動
- **CProperty**: 戰鬥屬性組件（生命值、防禦力等）
- **TAttack**: 攻擊能力組件
- **Hero**: 英雄特有組件
- **Unit**: 通用單位組件
- **Creep**: 小兵組件
- **Tower**: 防禦塔組件
- **Projectile**: 投射物組件
- **Skill**: 技能組件

#### 🔒 架構改進與規範

**🆕 重大重構成果 (2024)**：
- **State 模組完全獨立化**：將原本 1000+ 行的巨大檔案拆分為 5 個專業模組
  - `core.rs`: 核心狀態結構與 API
  - `initialization.rs`: ECS 世界設置和遊戲場景初始化
  - `time_management.rs`: 時間循環、日夜週期管理
  - `resource_management.rs`: 資源處理和玩家請求管理
  - `system_dispatcher.rs`: 系統調度和執行緒池管理
- **模組化視野系統**：將 895 行的 `shadow_calculator.rs` 重構為高性能模組
  - 四叉樹空間分割優化
  - 陰影計算與緩存系統
  - 視野結果管理與性能監控
- **事件驅動重構**：統一的事件分派系統，支援戰鬥、移動、創建事件
- **技能系統模組化**：將 639 行的技能系統拆分為專業化模組

**嚴格的 ECS SystemData 分離**：
- 實施嚴格的 Read/Write 結構分離，避免借用衝突
- 禁止在 Write 結構中混用 ReadStorage，提高並發安全性
- 統一事件驅動架構，所有實體操作通過 `Vec<Outcome>` 事件系統

**事件驅動設計**：
- ✅ **組件內容修改**：可在 tick 中直接修改屬性值（如 `hp -= 10`）
- ❌ **實體生命週期**：創建/刪除實體必須通過事件系統處理
- 統一的 `Outcome` 事件類型處理複雜的跨系統操作

### 遊戲循環 (Tick System)

系統以 10 TPS (每秒10次更新) 運行，主要更新模塊：

1. **creep_tick**: 小兵 AI 和移動
2. **tower_tick**: 防禦塔攻擊邏輯
3. **player_tick**: 玩家操作處理
4. **projectile_tick**: 投射物物理計算
5. **nearby_tick**: 鄰近實體檢測 (使用多緒排序優化)
6. **skill_tick**: 技能系統處理 (支援 JSON 配置技能)
7. **damage_tick**: 傷害計算與應用
8. **death_tick**: 死亡處理與重生邏輯

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

### 技能系統開發

#### 添加新技能

1. **創建 JSON 配置檔案**
   在 `ability-configs/` 目錄下創建新的 JSON 檔案：
   ```json
   {
     "abilities": [{
       "id": "skill_id",
       "name": "技能名稱",
       "type": "instant",
       "cooldown": [10.0, 9.0, 8.0, 7.0],
       "damage": [100.0, 150.0, 200.0, 250.0]
     }]
   }
   ```

2. **實現技能邏輯**
   在 `ability-system/src/lib.rs` 的 `generate_effects` 方法中添加技能處理

3. **載入配置**
   在 `skill_tick.rs` 的 `ensure_initialized` 方法中添加配置檔案路徑

4. **測試技能**
   確保英雄的 Skill 組件設定正確的 `ability_id`

## 📊 效能特色

### 🚀 核心性能優化
- **多線程處理**: 使用 Rayon 線程池最大化 CPU 利用率
- **多緒排序優化**: voracious_radix_sort 使用 4 個執行緒並行排序
- **記憶體最佳化**: ECS 架構確保資料局部性
- **非同步通信**: MQTT 客戶端支援非阻塞 I/O
- **批次處理**: 遊戲邏輯批次更新減少開銷

### 🆕 重構後的性能提升
- **模組化加載**: 獨立狀態模組支援按需初始化，減少啟動時間
- **空間分割優化**: 四叉樹結構大幅提升空間查詢效率
- **視野緩存系統**: 智能緩存機制減少重複計算
- **事件分派優化**: 按優先級分派事件，關鍵事件優先處理
- **系統調度器**: 彈性的執行緒池管理，動態調整並發度

### 🔧 傳統優化保留
- **技能系統優化**: JSON 配置預載入，執行時零解析開銷
- **單例模式**: 重量級資源（如 AbilityProcessor）使用全局單例避免重複初始化
- **借用衝突消除**: 嚴格的 Read/Write 分離避免 Rust 借用檢查器衝突
- **事件驅動最佳化**: 統一的事件系統減少跨系統通信開銷
- **直接組件修改**: 允許直接修改組件內容，避免不必要的事件開銷

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

- 主要開發者：damody <t1238142000@gmail.com>
- 貢獻者：見 [CONTRIBUTORS.md](CONTRIBUTORS.md)

## 🔗 相關連結

- [Specs ECS 文檔](https://specs.amethyst.rs/)
- [MQTT 協議規範](https://mqtt.org/)
- [Rust 程式語言](https://www.rust-lang.org/)

---

如有任何問題或建議，歡迎提交 Issue 或聯繫開發團隊。