# MQTT 測試介面說明

## 簡介

MQTT 測試介面提供了一個通過 MQTT 協議與 open_moba_backend 遊戲系統進行自動化測試的方法。這個介面支援技能系統測試、召喚系統測試、性能監控和基準測試。

## 功能特性

- **技能系統測試**: 測試雜賀孫一和伊達政宗的所有技能
- **召喚系統測試**: 測試各種召喚單位的創建
- **性能監控**: 收集執行時間和成功率統計
- **基準測試**: 運行性能基準測試
- **系統狀態查詢**: 獲取系統運行狀態和可用功能

## MQTT 主題

- **命令主題**: `ability_test/command` - 發送測試命令
- **回應主題**: `ability_test/response` - 接收測試結果

## 支援的命令

### 1. 查詢系統狀態
```json
{"command": "QueryStatus"}
```

### 2. 查詢可用技能
```json
{"command": "QueryAbilities"}
```

### 3. 測試技能執行
```json
{
  "command": "TestAbility",
  "data": {
    "ability_id": "flame_blade",
    "level": 1,
    "target_position": [150.0, 200.0],
    "target_entity": 5
  }
}
```

**可用技能列表**:
- `sniper_mode` - 雜賀孫一的狙擊模式
- `saika_reinforcements` - 雜賀孫一的雜賀眾
- `rain_iron_cannon` - 雜賀孫一的雨鐵炮
- `three_stage_technique` - 雜賀孫一的三段擊
- `flame_blade` - 伊達政宗的火焰刀
- `fire_dash` - 伊達政宗的火焰衝刺
- `flame_assault` - 伊達政宗的火焰突擊
- `matchlock_gun` - 伊達政宗的火繩槍

### 4. 測試召喚系統
```json
{
  "command": "TestSummon",
  "data": {
    "unit_type": "saika_gunner",
    "position": [100.0, 100.0],
    "count": 3
  }
}
```

**可用召喚單位**:
- `saika_gunner` - 雜賀鐵炮兵
- `archer` - 弓箭手
- `swordsman` - 劍士
- `mage` - 法師

### 5. 運行基準測試
```json
{
  "command": "RunBenchmark",
  "data": {
    "test_name": "ability_execution_speed",
    "iterations": 100
  }
}
```

### 6. 查詢性能統計
```json
{"command": "QueryMetrics"}
```

### 7. 重置測試環境
```json
{"command": "Reset"}
```

## 回應格式

所有回應都遵循統一格式：

```json
{
  "command": "test_ability",
  "success": true,
  "data": {
    "ability_id": "flame_blade",
    "level": 1,
    "effects_count": 2,
    "error": null
  },
  "timestamp": 1703123456,
  "execution_time_ms": 15
}
```

## 使用方法

### 1. 啟動遊戲服務

首先啟動 open_moba_backend：

```bash
cd /mnt/d/Nobu/open_moba_backend
cargo run
```

遊戲啟動後會自動啟動 MQTT 測試介面，監聽 `ability_test/command` 主題。

### 2. 使用 Python 測試客戶端

提供了一個 Python 測試客戶端來進行自動化測試：

```bash
# 安裝依賴
pip install paho-mqtt

# 運行測試套件
python examples/mqtt_test_client.py [broker_host] [broker_port]

# 示例：連接到本地 MQTT broker
python examples/mqtt_test_client.py localhost 1883
```

### 3. 使用 MQTT 客戶端工具

你也可以使用任何 MQTT 客戶端工具（如 mosquitto_pub/sub、MQTT Explorer 等）來手動發送命令。

**發送命令示例**:
```bash
mosquitto_pub -h localhost -p 1883 -t "ability_test/command" -m '{"command": "QueryStatus"}'
```

**接收回應**:
```bash
mosquitto_sub -h localhost -p 1883 -t "ability_test/response"
```

## 測試場景

### 基本功能測試
1. 查詢系統狀態
2. 查詢可用技能列表
3. 測試每個技能的執行
4. 測試召喚系統

### 性能測試
1. 運行技能執行速度基準測試
2. 查詢統計數據
3. 分析性能指標

### 壓力測試
1. 批量執行技能測試
2. 並發召喚測試
3. 長時間運行測試

## 故障排除

### 1. 連接問題
- 確認 MQTT broker 運行正常
- 檢查防火牆設定
- 驗證網路連接

### 2. 命令失敗
- 檢查 JSON 格式是否正確
- 驗證技能 ID 和參數
- 查看遊戲日誌獲取詳細錯誤信息

### 3. 回應超時
- 確認遊戲服務正常運行
- 檢查系統資源使用情況
- 增加超時時間

## 日誌和調試

遊戲服務會在控制台輸出詳細的測試介面日誌：

```
INFO  MQTT 測試介面管理器已創建
INFO  MQTT 測試介面已連接到 localhost:1883
DEBUG 收到測試命令: {"command":"QueryStatus"}
```

## 性能指標

測試介面會收集以下性能指標：

- **總命令數**: 執行的測試命令總數
- **成功命令數**: 成功執行的命令數
- **失敗命令數**: 執行失敗的命令數
- **平均回應時間**: 命令執行的平均時間
- **技能執行統計**: 每個技能的執行次數
- **召喚統計**: 各種召喚單位的創建次數

## 開發和擴展

### 添加新的測試命令

1. 在 `TestCommand` 枚舉中添加新變體
2. 在 `handle_command` 方法中添加處理邏輯
3. 實現對應的處理函數
4. 更新文檔和測試客戶端

### 集成到 CI/CD

可以將 MQTT 測試介面集成到持續集成流程中：

```bash
# 啟動遊戲服務（背景運行）
cargo run &
GAME_PID=$!

# 等待服務啟動
sleep 10

# 運行自動化測試
python examples/mqtt_test_client.py

# 清理
kill $GAME_PID
```

## 安全考慮

- 測試介面僅用於開發和測試環境
- 生產環境應禁用測試介面
- 考慮添加認證和授權機制
- 限制測試命令的執行頻率