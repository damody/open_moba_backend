# Ability.json 技能定義文件說明

本文檔詳細說明 `ability.json` 的結構與功能，該文件定義了所有遊戲單位的技能，包括英雄技能、敵方技能、中立生物技能和召喚物技能等。

## 📋 檔案結構概覽

```json
{
    "heroAbilities": {},      // 英雄技能
    "enemyAbilities": {},     // 敵方技能
    "neutralAbilities": {},   // 中立生物技能
    "summonAbilities": {},    // 召喚物技能
    "modifiers": {}           // 效果修飾符
}
```

## 🦸 英雄技能 (Hero Abilities)

### 雜賀孫市技能詳解

根據提供的技能資料，雜賀孫市擁有以下四個技能：

#### W - 狙擊模式
```json
{
    "W": {
        "id": "sniper_mode",
        "name": "狙擊模式",
        "behaviorType": "Toggle",        // 切換技能
        "cooldown": [10, 10, 10, 10],   // 冷卻時間10秒
        "manaCost": [50, 50, 50, 50],   // 耗魔50
        "effects": {
            "attackRangeBonus": [950, 1150, 1350, 1600], // 攻擊距離
            "moveSpeedReduction": 0.7,                    // 犧牲70%移速
            "lockDuration": [2, 3, 4, 5]                 // 無法取消時間
        }
    }
}
```

**技能機制說明：**
- 轉換為狙擊狀態後，攻擊距離大幅增加
- 犧牲70%移動速度作為代價
- 在指定時間內無法取消狙擊模式
- 是一個可切換的狀態技能

#### E - 雜賀眾
```json
{
    "E": {
        "id": "saika_reinforcements",
        "name": "雜賀眾",
        "behaviorType": "Active",       // 主動技能
        "cooldown": [10, 10, 10, 10],  // 冷卻時間10秒
        "manaCost": [100, 100, 100, 100], // 耗魔100
        "effects": {
            "summonCount": [2, 3, 4, 5],     // 召喚數量
            "summonAttackBonus": [15, 30, 45, 60] // 攻擊力加成
        }
    }
}
```

**技能機制說明：**
- 召喚雜賀眾同伴加入戰鬥
- 每級增加召喚數量和攻擊力
- 召喚物會自動攻擊附近敵人

#### R - 雨鐵炮
```json
{
    "R": {
        "id": "rain_iron_cannon",
        "name": "雨鐵炮",
        "behaviorType": "Passive",      // 被動技能
        "effects": {
            "procChance": 0.45,         // 45%觸發機率
            "buildingDamageBonus": [0.2, 0.3, 0.4],   // 對建築20/30/40%
            "heroDamageBonus": [0.4, 0.5, 0.6],       // 對英雄40/50/60%
            "creepDamageBonus": [1.0, 1.4, 1.8]       // 對部隊100/140/180%
        }
    }
}
```

**技能機制說明：**
- 攻擊時有45%機率觸發額外物理傷害
- 對不同目標類型有不同的傷害加成
- 對小兵的傷害加成最高，適合清理大量單位

#### T - 三段
```json
{
    "T": {
        "id": "three_stage_technique",
        "name": "三段",
        "behaviorType": "Active",       // 主動終極技能
        "cooldown": [135, 123, 111],   // 冷卻時間遞減
        "manaCost": [300, 350, 400],   // 耗魔遞增
        "effects": {
            "damageMultiplier": 3.0,    // 200%額外傷害 = 300%總計
            "duration": [7, 11, 13]     // 持續時間遞增
        }
    }
}
```

**技能機制說明：**
- 終極技能，大幅提升攻擊傷害
- 持續時間內所有攻擊獲得200%額外傷害
- 冷卻時間長，需要謹慎使用時機

## 🏗️ 技能數據結構

### 基本技能屬性

```json
{
    "id": "unique_skill_id",           // 技能唯一識別符
    "name": "技能顯示名稱",             // 遊戲內顯示名稱
    "description": "技能描述文字",      // 詳細說明
    "icon": "skill_icon_path",         // 圖標路徑
    "abilityType": "技能目標類型",      // 見下方說明
    "targetType": "目標類型",          // 目標分類
    "behaviorType": "行為類型"         // 技能行為
}
```

### 技能目標類型 (Ability Type)

| 類型 | 說明 | 使用場景 |
|------|------|----------|
| **NoTarget** | 無需目標 | 自我增益、範圍技能 |
| **TargetUnit** | 指定單位 | 單體攻擊、治療 |
| **TargetPoint** | 指定地點 | 範圍攻擊、召喚 |
| **TargetDirection** | 指定方向 | 直線技能、錐形攻擊 |
| **Passive** | 被動技能 | 永久效果、觸發效果 |

### 行為類型 (Behavior Type)

| 類型 | 說明 | 特點 |
|------|------|------|
| **Active** | 主動技能 | 需要手動釋放，消耗資源 |
| **Passive** | 被動技能 | 自動生效，無需操作 |
| **Toggle** | 切換技能 | 可開關的狀態技能 |

### Level Data（等級數據）

```json
{
    "levelData": {
        "maxLevel": 4,                    // 最大等級
        "cooldown": [10, 9, 8, 7],       // 各等級冷卻時間
        "manaCost": [50, 60, 70, 80],    // 各等級魔法消耗
        "castRange": [600, 700, 800, 900], // 各等級施法距離
        "castPoint": 0.3,                // 施法前搖
        "channelTime": 0,                // 引導時間
        "projectileSpeed": 1000          // 投射物速度（如適用）
    }
}
```

### Effects（技能效果）

```json
{
    "effects": {
        "damage": [100, 150, 200, 250],    // 傷害數值
        "damageType": "Physical|Magical|Pure", // 傷害類型
        "duration": 5.0,                   // 持續時間
        "radius": 300,                     // 作用半徑
        "stunDuration": 1.5,               // 眩暈時間
        "slow": 0.3,                       // 減速百分比
        "healAmount": 200,                 // 治療量
        "buffType": "攻擊力|移速|護甲"      // 增益類型
    }
}
```

## 🎭 修飾符系統 (Modifiers)

技能效果通過修飾符系統實現，定義各種增益和減益效果。

### 修飾符基本結構

```json
{
    "modifier_id": {
        "name": "修飾符顯示名稱",
        "description": "效果描述",
        "type": "buff|debuff",           // 增益或減益
        "dispellable": true,             // 是否可驅散
        "duration": "permanent|temporary", // 持續類型
        "stackable": false,              // 是否可疊加
        "icon": "modifier_icon_path"     // 圖標路徑
    }
}
```

### 常見修飾符類型

#### 增益效果 (Buffs)
```json
{
    "damage_boost": {
        "name": "傷害提升",
        "type": "buff",
        "effects": {
            "attackDamageBonus": 50,      // 攻擊力加成
            "attackDamageMultiplier": 1.5  // 攻擊力倍率
        }
    }
}
```

#### 減益效果 (Debuffs)
```json
{
    "frost_slow": {
        "name": "冰霜減速",
        "type": "debuff",
        "effects": {
            "moveSpeedMultiplier": 0.7,    // 移速倍率
            "attackSpeedReduction": 0.2    // 攻速減少
        }
    }
}
```

## 💫 特殊屬性與機制

### 1. 觸發條件 (Proc Conditions)

```json
{
    "procChance": 0.25,              // 觸發機率25%
    "procOnAttack": true,            // 攻擊時觸發
    "procOnCrit": true,              // 暴擊時觸發
    "procOnKill": true,              // 擊殺時觸發
    "cooldownBetweenProcs": 3.0      // 觸發間隔
}
```

### 2. 目標過濾 (Target Filters)

```json
{
    "targetFilters": {
        "validTargets": ["Hero", "Creep"],     // 有效目標
        "invalidTargets": ["Building"],        // 無效目標
        "affectsAllies": false,                // 是否影響友軍
        "affectsEnemies": true,                // 是否影響敵軍
        "affectsSelf": true                    // 是否影響自己
    }
}
```

### 3. 視覺和音效 (Visual & Audio)

```json
{
    "visualEffects": {
        "castEffect": "lightning_cast",        // 施放特效
        "impactEffect": "explosion_impact",    // 命中特效
        "buffEffect": "power_aura",           // 增益光環
        "projectileEffect": "magic_missile"    // 投射物特效
    },
    
    "soundEffects": {
        "castSound": "thunder_cast.wav",       // 施放音效
        "impactSound": "explosion.wav",        // 命中音效
        "loopSound": "power_loop.wav"         // 循環音效
    }
}
```

## 🔧 技能設計指南

### 1. 數值設計原則

#### 冷卻時間設計
- **短冷卻** (2-6秒): 基礎技能，頻繁使用
- **中冷卻** (8-15秒): 重要技能，戰術運用
- **長冷卻** (20-60秒): 關鍵技能，改變戰局
- **終極技能** (60-120秒): 決定性技能

#### 魔法消耗平衡
```javascript
// 建議公式
基礎消耗 = 英雄等級 × 10 + 技能威力係數
威力係數 = 傷害值 / 10 + 特殊效果加成
```

#### 傷害數值建議
```javascript
// 不同技能類型的傷害範圍
基礎攻擊技能: 英雄攻擊力 × 1.2-1.8
範圍傷害技能: 英雄攻擊力 × 0.8-1.2
終極技能: 英雄攻擊力 × 2.0-4.0
```

### 2. 技能組合設計

#### 技能配合原則
1. **主動 + 被動**: 主動技能提供爆發，被動提供持續效果
2. **控制 + 傷害**: 控制技能限制敵人，傷害技能造成威脅
3. **增益 + 消耗**: 強力增益配合高資源消耗
4. **風險 + 回報**: 高風險技能提供高回報

#### 雜賀孫市技能組合分析
- **W (狙擊模式)**: 提供射程優勢，犧牲機動性
- **E (雜賀眾)**: 提供戰場控制，分散敵人注意力  
- **R (雨鐵炮)**: 被動傷害加成，提高輸出效率
- **T (三段)**: 終極爆發技能，決定戰鬥結果

### 3. 平衡考量

#### 強度評估指標
```javascript
// 技能強度計算
技能分數 = (傷害 + 控制效果 + 增益效果) / (冷卻時間 + 魔法消耗)

// 各項權重
傷害權重 = 1.0
控制權重 = 1.5  // 控制比傷害更有價值
增益權重 = 1.2
```

#### 反制機制
- 每個強力技能都應有對應的反制手段
- 長冷卻技能需要明顯的使用時機窗口
- 強力被動技能需要激活條件或內建冷卻

## 🛠️ 實現建議

### 1. 技能系統架構

```javascript
// 偽代碼示例
class Ability {
    constructor(abilityData) {
        this.id = abilityData.id;
        this.level = 1;
        this.cooldownRemaining = 0;
        this.data = abilityData;
    }
    
    canCast(caster, target = null) {
        return this.cooldownRemaining <= 0 && 
               caster.currentMana >= this.getManaCost() &&
               this.isValidTarget(target);
    }
    
    cast(caster, target = null) {
        if (!this.canCast(caster, target)) return false;
        
        // 消耗資源
        caster.currentMana -= this.getManaCost();
        this.cooldownRemaining = this.getCooldown();
        
        // 執行效果
        this.executeEffect(caster, target);
        return true;
    }
}
```

### 2. 修飾符系統

```javascript
class Modifier {
    constructor(modifierData, source, target) {
        this.data = modifierData;
        this.source = source;
        this.target = target;
        this.remainingDuration = modifierData.duration;
        this.stackCount = 1;
    }
    
    apply() {
        // 應用修飾符效果
        this.target.addModifier(this);
    }
    
    remove() {
        // 移除修飾符效果
        this.target.removeModifier(this);
    }
    
    update(deltaTime) {
        if (this.remainingDuration > 0) {
            this.remainingDuration -= deltaTime;
            if (this.remainingDuration <= 0) {
                this.remove();
            }
        }
    }
}
```

### 3. 技能效果處理

```javascript
// 技能效果處理器
const SkillEffectHandlers = {
    damage: (caster, target, effectData) => {
        const damage = calculateDamage(caster, target, effectData);
        target.takeDamage(damage, effectData.damageType);
    },
    
    heal: (caster, target, effectData) => {
        const healAmount = calculateHeal(caster, effectData);
        target.heal(healAmount);
    },
    
    buff: (caster, target, effectData) => {
        const modifier = new Modifier(effectData.modifier, caster, target);
        modifier.apply();
    }
};
```

## 📊 數據驗證與測試

### 1. 數值驗證檢查表
- [ ] 冷卻時間合理性檢查
- [ ] 魔法消耗與英雄魔力池匹配
- [ ] 傷害數值與防禦力平衡
- [ ] 技能組合的協同效果
- [ ] 反制手段的有效性

### 2. 遊戲性測試
- [ ] 技能使用頻率統計
- [ ] 玩家滿意度調查
- [ ] 競技平衡性分析
- [ ] AI 使用效果評估

---

此文檔將隨技能系統的發展持續更新，確保技能設計的一致性和平衡性。