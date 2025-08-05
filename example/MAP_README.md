# Map.json 地圖配置文件說明

本文檔詳細說明 `map.json` 的結構與功能，該文件定義了 MOBA 遊戲的地圖配置，包括路徑、小兵、檢查點、防禦塔和小兵波次等核心遊戲元素。

## 📋 檔案格式支援

### C-Style 註解支援

`map.json` 支援使用 C-style 註解來增加可讀性：
- 單行註解：`// 這是單行註解`
- 多行註解：`/* 這是多行註解 */`

註解會在讀取時由預處理器自動移除，不影響 JSON 解析。

## 🏗️ JSON 結構概覽

```json
{
    "Path": [],        // 路徑定義
    "Creep": [],       // 小兵類型定義
    "CheckPoint": [],  // 檢查點定義
    "Tower": [],       // 防禦塔定義
    "CreepWave": []    // 小兵波次定義
}
```

## 📍 主要元素說明

### 1. Path（路徑）

定義小兵移動的路線，由一系列檢查點組成。

**欄位說明：**
| 欄位 | 類型 | 說明 |
|------|------|------|
| Name | string | 路徑唯一識別名稱 |
| Points | string[] | 檢查點名稱陣列，按順序排列 |

**範例：**
```json
{
    "Name": "path1",
    "Points": ["c1", "c2", "c3", "c4", "c5"]  // 小兵會依序經過這些檢查點
}
```

### 2. Creep（小兵）

定義不同類型小兵的屬性。

**欄位說明：**
| 欄位 | 類型 | 說明 |
|------|------|------|
| Name | string | 小兵類型名稱 |
| DefendPhysic | number | 物理防禦力 |
| DefendMagic | number | 魔法防禦力 |
| HP | number | 生命值 |
| MoveSpeed | number | 移動速度 |
| coins | number | 擊殺獎勵金幣 |

**範例：**
```json
{
    "Name": "cp1",           // 普通小兵
    "DefendPhysic": 0,       // 無物理防禦
    "DefendMagic": 0,        // 無魔法防禦
    "HP": 100,              // 100 生命值
    "MoveSpeed": 10,        // 移動速度 10
    "coins": 10             // 擊殺獲得 10 金幣
}
```

### 3. CheckPoint（檢查點）

定義地圖上的關鍵位置點。

**欄位說明：**
| 欄位 | 類型 | 說明 |
|------|------|------|
| Name | string | 檢查點名稱 |
| Class | string | 檢查點類型（Start/CheckPoint/End） |
| X | number | X 座標 |
| Y | number | Y 座標 |

**檢查點類型：**
- `Start`: 小兵出生點
- `CheckPoint`: 中途路徑點
- `End`: 終點（通常是基地）

**範例：**
```json
{
    "Name": "c1",
    "Class": "Start",    // 起始點
    "X": 0,
    "Y": 0
}
```

### 4. Tower（防禦塔）

定義可建造的防禦塔類型及其屬性。

**欄位說明：**
| 欄位 | 類型 | 說明 |
|------|------|------|
| Name | string | 防禦塔類型名稱 |
| Property | object | 防禦塔基礎屬性 |
| Property.Hp | number | 防禦塔生命值 |
| Property.Block | number | 阻擋能力 |
| Attack | object | 攻擊屬性 |
| Attack.Range | number | 攻擊範圍 |
| Attack.AttackSpeed | number | 攻擊速度（次/秒） |
| Attack.Physic | number | 物理傷害 |
| Attack.Magic | number | 魔法傷害 |
| Attack.cost | number | 建造成本 |

**範例：**
```json
{
    "Name": "arrow1",
    "Property": {
        "Hp": 10,        // 10 點生命值
        "Block": 1       // 可阻擋 1 個單位
    },
    "Attack": {
        "Range": 300,         // 攻擊範圍 300
        "AttackSpeed": 0.5,   // 每 2 秒攻擊一次
        "Physic": 3,          // 3 點物理傷害
        "Magic": 0,           // 無魔法傷害
        "cost": 3            // 建造花費 3 金幣
    }
}
```

### 5. CreepWave（小兵波次）

定義小兵的出現時機和順序。

**欄位說明：**
| 欄位 | 類型 | 說明 |
|------|------|------|
| Name | string | 波次名稱 |
| StartTime | number | 波次開始時間（秒） |
| Detail | array | 各路徑的小兵詳情 |

**Detail 欄位：**
| 欄位 | 類型 | 說明 |
|------|------|------|
| Path | string | 使用的路徑名稱 |
| Creeps | array | 小兵出現時間表 |

**Creeps 欄位：**
| 欄位 | 類型 | 說明 |
|------|------|------|
| Time | number | 相對於波次開始的時間（秒） |
| Creep | string | 小兵類型名稱 |

**範例：**
```json
{
    "Name": "Wave1",
    "StartTime": 1,      // 遊戲開始後 1 秒
    "Detail": [
        {
            "Path": "path1",     // 使用 path1 路徑
            "Creeps": [
                {"Time": 0, "Creep": "cp1"},    // 立即出現 cp1
                {"Time": 2, "Creep": "cp2"},    // 2 秒後出現 cp2
                {"Time": 3, "Creep": "cp2"}     // 3 秒後再出現 cp2
            ]
        }
    ]
}
```

## 📝 完整範例（支援註解）

```json
{
    // 路徑定義 - 定義小兵的移動路線
    "Path": [
        {
            "Name": "path1",
            "Points": ["c1", "c2", "c3", "c4", "c5"]  // 主要路徑
        },
        {
            "Name": "path2",
            "Points": ["c1", "c4", "c3", "c2", "c5"]  // 替代路徑
        }
    ],
    
    /* 小兵類型定義
       包含兩種基本小兵類型 */
    "Creep": [
        {
            "Name": "cp1",           // 重裝小兵
            "DefendPhysic": 0,
            "DefendMagic": 0,
            "HP": 100,              // 高血量
            "MoveSpeed": 10,        // 低速度
            "coins": 10             // 高獎勵
        },
        {
            "Name": "cp2",           // 快速小兵
            "DefendPhysic": 0,
            "DefendMagic": 0,
            "HP": 50,               // 低血量
            "MoveSpeed": 50,        // 高速度
            "coins": 3              // 低獎勵
        }
    ],
    
    // 檢查點定義 - 地圖上的關鍵位置
    "CheckPoint": [
        {"Name": "c1", "Class": "Start", "X": 0, "Y": 0},           // 起點
        {"Name": "c2", "Class": "CheckPoint", "X": 0, "Y": 1000},   // 第一個轉彎點
        {"Name": "c3", "Class": "CheckPoint", "X": 1000, "Y": 2000}, // 第二個轉彎點
        {"Name": "c4", "Class": "CheckPoint", "X": 0, "Y": 3000},   // 第三個轉彎點
        {"Name": "c5", "Class": "End", "X": 0, "Y": 4000}           // 終點（基地）
    ],
    
    // 防禦塔定義
    "Tower": [
        {
            "Name": "arrow1",        // 基礎箭塔
            "Property": {
                "Hp": 10,
                "Block": 1
            },
            "Attack": {
                "Range": 300,        // 短距離
                "AttackSpeed": 0.5,
                "Physic": 3,
                "Magic": 0,
                "cost": 3           // 便宜
            }
        },
        {
            "Name": "arrow2",        // 進階箭塔
            "Property": {
                "Hp": 10,
                "Block": 1
            },
            "Attack": {
                "Range": 500,        // 長距離
                "AttackSpeed": 0.5,
                "Physic": 3,
                "Magic": 0,
                "cost": 5           // 較貴
            }
        }
    ],
    
    /* 小兵波次配置
       第一波會在遊戲開始後 1 秒啟動 */
    "CreepWave": [
        {
            "Name": "Wave1",
            "StartTime": 1,
            "Detail": [
                {
                    "Path": "path1",
                    "Creeps": [
                        {"Time": 0, "Creep": "cp1"},    // 先鋒部隊
                        // 快速小兵群
                        {"Time": 2, "Creep": "cp2"},
                        {"Time": 3, "Creep": "cp2"},
                        {"Time": 4, "Creep": "cp2"},
                        {"Time": 5, "Creep": "cp2"},
                        {"Time": 6, "Creep": "cp2"},
                        {"Time": 7, "Creep": "cp2"},
                        {"Time": 8, "Creep": "cp2"},
                        // 第二批快速小兵
                        {"Time": 12, "Creep": "cp2"},
                        {"Time": 13, "Creep": "cp2"},
                        {"Time": 14, "Creep": "cp2"},
                        {"Time": 15, "Creep": "cp2"},
                        {"Time": 16, "Creep": "cp2"},
                        {"Time": 17, "Creep": "cp2"},
                        {"Time": 18, "Creep": "cp2"}
                    ]
                },
                {
                    "Path": "path2",     // 第二條路線
                    "Creeps": [
                        {"Time": 0, "Creep": "cp1"},   // 間隔 5 秒
                        {"Time": 5, "Creep": "cp1"},   // 出現重裝兵
                        {"Time": 10, "Creep": "cp1"}
                    ]
                }
            ]
        }
    ]
}
```

## 🔧 JSON 預處理器實現

為了支援 C-style 註解，需要在讀取 JSON 前進行預處理。以下是 Rust 實現範例：

```rust
use regex::Regex;

/// 移除 JSON 字串中的 C-style 註解
pub fn remove_json_comments(json_str: &str) -> String {
    // 移除單行註解 //
    let single_line_comment = Regex::new(r"//.*").unwrap();
    let json_str = single_line_comment.replace_all(&json_str, "");
    
    // 移除多行註解 /* */
    let multi_line_comment = Regex::new(r"/\*[\s\S]*?\*/").unwrap();
    let json_str = multi_line_comment.replace_all(&json_str, "");
    
    json_str.to_string()
}

/// 讀取並解析支援註解的 JSON 文件
pub fn read_json_with_comments(file_path: &str) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let json_str = std::fs::read_to_string(file_path)?;
    let cleaned_json = remove_json_comments(&json_str);
    let json_value = serde_json::from_str(&cleaned_json)?;
    Ok(json_value)
}
```

## 💡 使用建議

1. **路徑設計**：合理設計多條路徑增加遊戲策略性
2. **小兵平衡**：調整不同小兵的屬性創造多樣性
3. **波次節奏**：通過時間間隔控制遊戲節奏
4. **防禦塔定位**：不同射程和傷害類型的塔提供戰術選擇
5. **註解使用**：善用註解說明設計意圖和參數含義

## 🔍 驗證工具

建議實現一個 JSON 驗證工具來檢查：
- 路徑中的檢查點是否都已定義
- 波次中的小兵類型是否存在
- 座標值是否合理
- 時間順序是否正確

這樣可以在遊戲運行前發現配置錯誤，提高開發效率。