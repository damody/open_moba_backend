# Mission.json 戰役配置文件說明

本文檔詳細說明 `mission.json` 的結構與功能，該文件定義了戰役的流程、目標、規則和評分系統等所有任務相關配置。

## 📋 檔案結構概覽

```json
{
    "campaign": {},           // 戰役基本資訊
    "stages": [],            // 關卡列表
    "campaignRules": {},     // 戰役規則
    "scoring": {},           // 評分系統
    "uiSettings": {},        // UI/UX 設定
    "replaySettings": {},    // 回放系統
    "tutorialHints": {}      // 教學提示
}
```

## 🎯 戰役基本資訊 (Campaign)

定義戰役的基本屬性和元數據。

### 結構說明

```json
{
    "campaign": {
        "id": "B01_1",                    // 戰役唯一識別符
        "name": "千里狙殺",               // 戰役名稱
        "subtitle": "雜賀孫市的狙擊試煉", // 副標題
        "description": "戰役描述文字",    // 詳細描述
        "heroId": "B01_SaikaMagoichi",   // 使用的英雄ID
        "difficulty": "Normal|Hard|Expert", // 難度等級
        "category": "Tutorial|Story|Challenge", // 戰役類型
        "estimatedTime": "10-15 minutes", // 預估完成時間
        "version": "1.0.0"               // 版本號
    }
}
```

### 戰役類型說明

| 類型 | 說明 | 特色 |
|------|------|------|
| **Tutorial** | 教學戰役 | 新手引導，簡化機制 |
| **Story** | 劇情戰役 | 豐富劇情，中等難度 |
| **Challenge** | 挑戰戰役 | 高難度，特殊規則 |

## 🏆 關卡系統 (Stages)

定義戰役中的各個關卡配置。

### 關卡基本結構

```json
{
    "id": "S0",                    // 關卡ID
    "name": "遠距補刀",            // 關卡名稱
    "description": "關卡描述",     // 關卡說明
    "type": "LastHit|Combat|Defense|Boss", // 關卡類型
    "timeLimit": 60,              // 時間限制（秒）
    "mapId": "training_ground_basic", // 使用的地圖ID
    "objectives": {},             // 目標系統
    "rewards": {}                 // 完成獎勵
}
```

### 關卡類型詳解

#### 1. LastHit（補刀訓練）
專注於補刀技巧的訓練關卡。

```json
{
    "type": "LastHit",
    "spawnSettings": {
        "creepWaves": [
            {
                "waveId": "basic_creep_wave",
                "interval": 3,                    // 波次間隔
                "creepTypes": ["melee", "ranged"], // 小兵類型
                "count": [3, 1]                   // 各類型數量
            }
        ]
    }
}
```

#### 2. Combat（戰鬥關卡）
包含敵人戰鬥的關卡。

```json
{
    "type": "Combat",
    "enemySpawns": [
        {
            "unitId": "enemy_mage_basic",
            "position": {"x": 500, "y": 0},
            "spawnTime": 5,
            "behavior": "defensive|aggressive|passive"
        }
    ]
}
```

#### 3. Defense（防守關卡）
保護基地免受敵人攻擊。

```json
{
    "type": "Defense",
    "waveSettings": {
        "waves": [
            {
                "waveNumber": 1,
                "startTime": 10,
                "units": [
                    {"unitId": "dire_melee_creep", "count": 6, "interval": 1}
                ]
            }
        ]
    }
}
```

### 目標系統 (Objectives)

每個關卡可包含主要目標和次要目標。

#### 主要目標 (Primary)
```json
{
    "primary": [
        {
            "id": "cs_target",
            "description": "60秒內至少補刀25個小兵",
            "type": "LastHit|Kill|Survival|Collect",
            "target": 25,                    // 目標數值
            "timeLimit": 60,                // 時間限制
            "required": true,               // 是否必須完成
            "specialCondition": "optional"  // 特殊條件
        }
    ]
}
```

#### 次要目標 (Secondary)
```json
{
    "secondary": [
        {
            "id": "perfect_cs",
            "description": "補刀率達到90%以上",
            "type": "Efficiency",
            "target": 0.9,
            "bonus": 50                     // 完成獎勵
        }
    ]
}
```

### 目標類型說明

| 類型 | 說明 | 參數 |
|------|------|------|
| **LastHit** | 補刀目標 | target: 數量 |
| **Kill** | 擊殺目標 | target: 數量, unitType: 敵人類型 |
| **Survival** | 生存目標 | duration: 持續時間 |
| **Efficiency** | 效率目標 | target: 成功率(0-1) |
| **Collect** | 收集目標 | target: 收集數量 |

### 特殊規則 (Special Rules)

某些關卡可能有特殊的遊戲規則。

```json
{
    "specialRules": [
        {
            "type": "rangeIndicator",
            "description": "1000距離以上時準星變綠",
            "threshold": 1000
        },
        {
            "type": "weakpointSystem",
            "description": "目標頭部弱點造成2倍傷害",
            "multiplier": 2.0
        }
    ]
}
```

### 環境效果 (Environmental Effects)

關卡可能包含環境因素影響遊戲玩法。

```json
{
    "environmentalEffects": [
        {
            "type": "wind",
            "direction": {"x": 1, "y": 0},  // 風向
            "strength": 150,                // 風力強度
            "affectsProjectiles": true      // 是否影響投射物
        }
    ]
}
```

## ⚖️ 戰役規則 (Campaign Rules)

定義整個戰役的遊戲規則和限制。

```json
{
    "campaignRules": {
        "heroRespawn": false,           // 英雄是否可重生
        "itemShopEnabled": true,        // 是否啟用商店
        "levelCap": 6,                 // 等級上限
        "startingGold": 600,           // 初始金錢
        "startingLevel": 1,            // 初始等級
        "passiveGoldRate": 1,          // 每秒被動金錢收入
        "creepGoldMultiplier": 1.0,    // 小兵金錢倍率
        "experienceMultiplier": 1.2,   // 經驗值倍率
        "difficultyScaling": false     // 是否啟用難度縮放
    }
}
```

### 規則說明

| 規則 | 說明 | 建議值 |
|------|------|--------|
| **heroRespawn** | 英雄死亡是否可復活 | Tutorial: true, Story: false |
| **levelCap** | 英雄最高等級限制 | 通常 6-10 級 |
| **startingGold** | 初始金錢影響道具購買 | 300-1000 |
| **passiveGoldRate** | 被動收入影響經濟節奏 | 1-3 金/秒 |

## ⭐ 評分系統 (Scoring)

定義關卡完成後的評分標準和獎勵。

### 星級評分

```json
{
    "starRating": {
        "3stars": {
            "requirements": [
                "完成所有主要目標",
                "完成至少80%次要目標",
                "用時少於目標時間的120%"
            ],
            "bonusGold": 200,
            "bonusExperience": 100
        }
    }
}
```

### 獎勵目標

```json
{
    "bonusObjectives": [
        {
            "id": "perfect_accuracy",
            "name": "神射手",
            "description": "命中率達到95%以上",
            "condition": {
                "type": "accuracy",
                "threshold": 0.95
            },
            "reward": 300
        }
    ]
}
```

### 評分條件類型

| 條件類型 | 說明 | 參數 |
|----------|------|------|
| **accuracy** | 命中率 | threshold: 0-1 |
| **healthLoss** | 血量損失 | threshold: 損失血量 |
| **totalTime** | 總用時 | threshold: 秒數 |
| **itemUsage** | 道具使用 | requiredItems: 道具列表 |

## 🎮 UI/UX 設定 (UI Settings)

定義用戶界面的行為和顯示選項。

### 範圍指示器

```json
{
    "rangeIndicator": {
        "enabled": true,
        "longRangeThreshold": 1000,     // 遠程閾值
        "colorChange": "green"          // 顏色變化
    }
}
```

### 傷害數字顯示

```json
{
    "damageNumbers": {
        "enabled": true,
        "criticalDamageColor": "yellow",    // 暴擊傷害顏色
        "longRangeBonusColor": "blue"       // 遠程加成顏色
    }
}
```

### 小地圖設定

```json
{
    "minimapSettings": {
        "showEnemyMovement": true,      // 顯示敵人移動
        "showAttackRange": true,        // 顯示攻擊範圍
        "showObjectives": true          // 顯示目標位置
    }
}
```

## 📹 回放系統 (Replay Settings)

定義遊戲回放和高光時刻的記錄。

```json
{
    "replaySettings": {
        "autoRecord": true,             // 自動錄製
        "recordHighlights": [
            "long_range_kills",         // 遠程擊殺
            "consecutive_kills",        // 連續擊殺
            "critical_hits",           // 暴擊
            "objective_completion"      // 目標完成
        ],
        
        "highlightThresholds": {
            "longRangeKill": 1200,     // 遠程擊殺距離閾值
            "criticalHitDamage": 400,  // 暴擊傷害閾值
            "consecutiveKillWindow": 5  // 連殺時間窗口
        }
    }
}
```

## 💡 教學提示 (Tutorial Hints)

為每個關卡提供操作提示和策略建議。

```json
{
    "tutorialHints": {
        "S0": [
            "利用射程優勢，在安全距離補刀",
            "觀察小兵血量，在最後一擊時攻擊",
            "遠程小兵的獎勵更高，優先擊殺"
        ],
        "S1b": [
            "超過1000距離時準星會變綠",
            "使用W技能標記目標增加傷害",
            "連殺需要在短時間內完成"
        ]
    }
}
```

## 🔧 技術實現建議

### 1. 關卡加載
```javascript
// 偽代碼示例
function loadStage(stageId) {
    const stage = mission.stages.find(s => s.id === stageId);
    
    // 加載地圖
    loadMap(stage.mapId);
    
    // 設置目標
    setupObjectives(stage.objectives);
    
    // 配置敵人生成
    if (stage.enemySpawns) {
        setupEnemySpawns(stage.enemySpawns);
    }
    
    // 應用特殊規則
    if (stage.specialRules) {
        applySpecialRules(stage.specialRules);
    }
}
```

### 2. 目標追蹤
```javascript
// 目標完成檢查
function checkObjectiveCompletion(objectiveId, value) {
    const objective = currentStage.objectives.primary
        .find(obj => obj.id === objectiveId);
    
    if (objective && value >= objective.target) {
        completeObjective(objectiveId);
        
        if (allPrimaryObjectivesComplete()) {
            completeStage();
        }
    }
}
```

### 3. 評分計算
```javascript
// 星級評分計算
function calculateStarRating() {
    let stars = 1; // 完成即1星
    
    const secondaryCompleted = getSecondaryObjectiveCompletion();
    const timeRatio = actualTime / targetTime;
    
    if (secondaryCompleted >= 0.5) stars = 2;
    if (secondaryCompleted >= 0.8 && timeRatio <= 1.2) stars = 3;
    
    return stars;
}
```

## 📊 數據分析與平衡

### 1. 關卡難度曲線
- 逐步增加複雜度
- 合理的時間限制設定
- 適當的獎勵分配

### 2. 目標設計原則
- 主要目標：基本技能掌握
- 次要目標：進階技巧挑戰
- 特殊目標：創意和完美執行

### 3. 評分平衡
- 星級分布要合理
- 獎勵目標有挑戰性但可達成
- 時間壓力適中

---

此文檔將隨遊戲內容更新持續維護，確保戰役配置的準確性和完整性。