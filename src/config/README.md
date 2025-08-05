# Config 系統說明

本目錄負責管理遊戲的所有配置系統，包括服務器設定、遊戲參數等。

## 📁 檔案結構

### server_config.rs

管理服務器的全局配置，使用 lazy_static 實現單例模式。

#### 主要功能

- **配置載入**: 從 `game.toml` 讀取配置
- **全局訪問**: 通過 `CONFIG` 靜態變量訪問
- **熱更新支援**: 可在運行時重新載入配置

#### 配置項目

```rust
pub struct ServerConfig {
    // 地圖配置
    pub map: String,           // 地圖檔案路徑
    
    // 服務器設定
    pub max_player: u32,       // 最大玩家數
    pub server_ip: String,     // MQTT Broker IP
    pub server_port: String,   // MQTT Broker 端口
    pub client_id: String,     // MQTT 客戶端 ID
    
    // 遊戲參數（可擴展）
    // pub tick_rate: u32,     // 更新頻率
    // pub spawn_interval: f32, // 小兵生成間隔
}
```

## 🔧 使用方式

### 讀取配置

```rust
use crate::config::server_config::CONFIG;

// 獲取最大玩家數
let max_players = CONFIG.max_player;

// 獲取服務器地址
let broker_addr = format!("{}:{}", CONFIG.server_ip, CONFIG.server_port);
```

### 配置檔案格式 (game.toml)

```toml
[server]
MAP = "map.json"
MAX_PLAYER = 10000
SERVER_IP = "127.0.0.1"
SERVER_PORT = "1883"
CLIENT_ID = "omobab"

# 未來可擴展的配置
# [game]
# TICK_RATE = 10
# SPAWN_INTERVAL = 30.0
# 
# [balance]
# TOWER_DAMAGE_MULTIPLIER = 1.0
# CREEP_HEALTH_MULTIPLIER = 1.0
```

## 🚀 擴展指南

### 添加新配置

1. **修改 ServerConfig 結構**
```rust
#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    // 現有欄位...
    
    // 新增配置
    pub new_setting: String,
}
```

2. **更新 game.toml**
```toml
[server]
NEW_SETTING = "value"
```

3. **使用新配置**
```rust
let setting = &CONFIG.new_setting;
```

### 配置分類建議

建議將配置按功能分類：

- **server**: 服務器相關設定
- **game**: 遊戲規則參數
- **balance**: 平衡性數值
- **network**: 網路相關設定
- **performance**: 性能調優參數

## 📊 配置管理最佳實踐

1. **默認值**: 為所有配置提供合理的默認值
2. **驗證**: 載入時驗證配置的有效性
3. **文檔**: 為每個配置項添加詳細註釋
4. **版本控制**: 記錄配置格式的版本
5. **環境分離**: 支援不同環境的配置（開發/測試/生產）

## 🔍 常見配置場景

### 開發環境
```toml
[server]
SERVER_IP = "localhost"
MAX_PLAYER = 10
CLIENT_ID = "dev_server"
```

### 生產環境
```toml
[server]
SERVER_IP = "mqtt.production.com"
MAX_PLAYER = 10000
CLIENT_ID = "prod_server_01"
```

### 壓力測試
```toml
[server]
MAX_PLAYER = 50000
# 其他性能相關配置
```

## ⚠️ 注意事項

1. **配置熱更新**: 某些配置可能需要重啟服務器才能生效
2. **配置驗證**: 確保數值在合理範圍內（如端口號 1-65535）
3. **安全性**: 不要在配置中存儲敏感信息（密碼、密鑰等）
4. **向後兼容**: 更新配置格式時考慮兼容性