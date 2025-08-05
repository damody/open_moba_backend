# Component 系統說明

本目錄包含了所有 ECS (Entity Component System) 組件的定義。每個組件代表一個遊戲實體的特定屬性或行為。

## 📁 目錄結構

### 核心組件

- **phys.rs** - 物理相關組件
  - `Pos` - 位置組件 (x, y, z 座標)
  - `Vel` - 速度組件 (vx, vy, vz 向量)
  - `SearchRef` - 用於快速搜尋的參考

- **resources.rs** - 資源管理
  - `Time` - 遊戲時間
  - `DeltaTime` - 幀間隔時間
  - `Searcher` - 實體搜尋索引系統

- **state.rs** - 遊戲狀態管理
  - `State` - 全局遊戲狀態控制
  - 包含玩家管理、地圖載入等功能

### 戰鬥系統組件

- **attack.rs** - 攻擊系統
  - `TAttack` - 攻擊屬性（攻擊力、速度、範圍等）
  - 包含攻擊計算和目標判定

- **damage.rs** - 傷害系統
  - `DamageInstance` - 傷害實例
  - `DamageTypes` - 傷害類型（物理、魔法、純粹）
  - `DamageSource` - 傷害來源追蹤

- **projectile.rs** - 投射物系統
  - `Projectile` - 投射物組件
  - 處理飛行軌跡和碰撞檢測

### 實體組件

- **hero.rs** - 英雄組件
  - `Hero` - 英雄屬性（等級、經驗、三圍等）
  - 支援力量、敏捷、智力三種主屬性

- **creep.rs** - 小兵組件
  - `Creep` - 小兵類型和行為
  - `CProperty` - 小兵屬性（血量、防禦等）

- **tower.rs** - 防禦塔組件
  - `Tower` - 塔的等級和狀態
  - 包含攻擊邏輯和目標選擇

- **unit.rs** - 通用單位組件
  - `Unit` - 所有可控制單位的基礎組件
  - 統一的單位管理介面

### 技能系統

- **skill.rs** - 技能組件
  - `Skill` - 技能狀態（冷卻、等級、充能等）
  - `SkillInput` - 技能輸入請求
  - `SkillEffect` - 技能效果定義
  - `SkillEffectData` - 技能數值修改

- **ability.rs** - 技能定義
  - `Ability` - 技能配置數據
  - 包含技能的所有靜態屬性

- **ability_comp.rs** - ability-system 整合組件
  - `AbilityComponent` - 技能狀態管理（技能ID映射、等級管理）
  - `AbilityRequestComponent` - 技能請求佇列
  - `AbilityResultComponent` - 技能處理結果
  - 與 ability-system 子系統的 ECS 整合介面

### 其他系統

- **player.rs** - 玩家系統
  - `Player` - 玩家身份和狀態
  - `PlayerId` - 玩家唯一標識

- **outcome.rs** - 事件結果
  - `Outcome` - 遊戲事件（攻擊、死亡、升級等）
  - 用於系統間通信

- **enemy.rs** - 敵人系統
  - `Enemy` - 敵人標記組件
  - `DamageType` - 傷害類型枚舉

- **ecs.rs** - ECS 系統定義
  - `System` trait - 自定義系統介面
  - `Job` - 系統執行任務

## 🔧 使用方式

### 創建新組件

1. 在對應類別的檔案中定義組件結構
2. 實作 `Component` trait
3. 在 `mod.rs` 中導出組件
4. 在 `State::register_components` 中註冊

### 組件設計原則

- **單一職責**：每個組件只負責一個特定功能
- **數據導向**：組件只包含數據，不包含邏輯
- **可序列化**：使用 Serde 支援網路傳輸
- **高效存儲**：選擇適當的 Storage 類型

### Storage 類型選擇

- `VecStorage`: 密集組件（大部分實體都有）
- `HashMapStorage`: 稀疏組件（少數實體擁有）
- `DenseVecStorage`: 頻繁增刪的密集組件
- `NullStorage`: 標記組件（不含數據）

## 📝 注意事項

1. 組件應該保持簡單，複雜邏輯放在 System 中
2. 避免組件間的直接依賴
3. 使用適當的數據類型以優化記憶體
4. 考慮網路同步的需求