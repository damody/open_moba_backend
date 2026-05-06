# Tick System 說明

本目錄包含了遊戲的所有更新系統（System），每個系統負責處理特定類型的遊戲邏輯。系統以 10 TPS（每秒10次）的頻率運行。

## 🎮 核心概念

### ECS System 架構

每個 System 都實作了 `crate::comp::ecs::System` trait，包含：
- `SystemData`: 定義系統需要的組件數據
- `run()`: 每幀執行的更新邏輯
- 嚴格分離的讀取和寫入權限，避免數據競爭

#### 🔒 SystemData 架構規範

**嚴格的 Read/Write 分離**：
- `Read` 結構：只能包含 `Read<>`, `ReadStorage<>`, `Entities<'a>`
- `Write` 結構：只能包含 `Write<>`, `WriteStorage<>`, `Entities<'a>`
- 禁止在 `Write` 結構中混用 `ReadStorage`，避免借用衝突

#### 🌊 事件驅動架構

**核心原則**：
- ✅ **組件內容修改**：可以在 tick 中直接修改組件屬性值
- ❌ **實體操作**：實體的創建和刪除必須通過 `Vec<Outcome>` 事件系統

**事件類型**：
- `Outcome::Death` - 實體死亡
- `Outcome::Damage` - 傷害處理
- `Outcome::Heal` - 治療效果
- `Outcome::ProjectileLine2` - 投射物生成
- `Outcome::CreepStop` - 小兵阻擋
- `Outcome::GainExperience` - 經驗獲取

### 執行順序

系統按照依賴關係順序執行，確保數據一致性：
1. 輸入處理（player_tick）
2. 狀態更新（skill_tick, creep_wave）
3. 行為執行（hero_tick, creep_tick, tower_tick）
4. 物理計算（projectile_tick, nearby_tick）
5. 結果處理（damage_tick, death_tick）

## 📁 系統說明

### 玩家與輸入系統

**player_tick.rs**
- 處理玩家輸入指令
- 管理玩家連接狀態
- 轉換 MQTT 消息為遊戲行動

### 戰鬥系統

**hero_tick.rs**
- 英雄行為邏輯（移動、攻擊）
- 經驗值和升級處理
- 英雄特殊機制

**tower_tick.rs**
- 防禦塔自動攻擊
- 目標選擇優先級
- 塔的狀態管理

**creep_tick.rs**
- 小兵 AI 行為
- 路徑尋找和移動
- 戰鬥邏輯

**projectile_tick.rs**
- 投射物飛行軌跡
- 碰撞檢測
- 命中處理

### 技能系統

**skill_tick.rs** ⭐
- 技能系統核心
- 整合 ability-system 子系統
- 處理技能輸入和效果
- 支援 JSON 配置技能與硬編碼技能
- 管理技能冷卻和效果持續時間

### 傷害與死亡系統

**damage_tick.rs**
- 傷害計算和減免
- 暴擊和閃避判定
- 傷害類型處理（物理/魔法/純粹）

* *死亡_tick.rs**
- 死亡判定和處理
- 經驗值和金錢獎勵
- 重生邏輯

### 輔助系統

**nearby_tick.rs**
- 維護空間索引
- 快速鄰近實體查詢
- 使用 voracious_radix_sort 多緒優化
- 支援高效的範圍搜尋

* *蠕動波.rs**
- 小兵生成波次控制
- 兵線平衡管理
- 遊戲節奏調控

## 🔧 開發指南

### 創建新 System

```rust
use specs::{System, SystemData, Read, WriteStorage, ReadStorage, Entities, Join};
use crate::comp::*;

# [匯出（系統資料）]
pub struct MySystemRead<'a> {
    entities: Entities<'a>,
    time: Read<'a, Time>,
    dt: Read<'a, DeltaTime>,
    positions: ReadStorage<'a, Pos>,
    // 所有只讀組件放在這裡
}

# [匯出（系統資料）]
pub struct MySystemWrite<'a> {
    outcomes: Write<'a, Vec<Outcome>>,
    velocities: WriteStorage<'a, Vel>,
    // 所有需要修改的組件放在這裡
}

# [導出（預設）]
pub struct Sys;

impl<'a> crate::comp::ecs::System<'a> for Sys {
    type SystemData = (MySystemRead<'a>, MySystemWrite<'a>);
    const NAME: &'static str = "my_system";
    
    fn run(_job: &mut Job<Self>, (tr, mut tw): Self::SystemData) {
        let dt = tr.dt.0;
        
        // 直接修改組件內容 ✅
        for (entity, pos, vel) in (&tr.entities, &tr.positions, &mut tw.velocities).join() {
            vel.0 += pos.0 * dt; // 直接修改組件
        }
        
        // 實體操作使用事件 ✅
        if some_condition {
            tw.outcomes.push(Outcome::Death {
                pos: some_position,
                ent: some_entity,
            });
        }
    }
}
```

### 性能優化技巧

1. **使用 ParJoin**: 平行處理獨立實體
```rust
(&entities, &positions, &mut velocities)
    .par_join()
    .for_each(|(e, pos, vel)| {
        // 平行處理
    });
```

2. **批次處理**: 收集變更後統一應用
```rust
let mut outcomes = Vec::new();
// 收集所有結果
for (...) { 
    outcomes.push(outcome);
}
// 批次處理
tw.outcomes.extend(outcomes);
```

3. **空間索引**: 使用 nearby_tick 的索引進行範圍查詢
```rust
let nearby = searcher.find_in_radius(pos, radius);
```

### System 設計原則

1. **單一職責**: 每個 System 專注一個功能領域
2. **嚴格數據分離**: Read 結構只讀，Write 結構只寫，禁止混用
3. **事件驅動**: 實體操作通過 `Vec<Outcome>` 事件，組件修改可直接進行
4. **無狀態設計**: System 本身不儲存狀態（除了初始化配置）
5. **單例模式**: 重量級資源（如 AbilityProcessor）使用全局單例避免重複初始化
6. **錯誤處理**: 優雅處理異常情況，不使系統崩潰

### ⚠️ 常見錯誤避免

❌ **錯誤做法**：
```rust
// 在 Write 結構中混用 ReadStorage
pub struct BadWrite<'a> {
    outcomes: Write<'a, Vec<Outcome>>,
    velocities: WriteStorage<'a, Vel>,
    positions: ReadStorage<'a, Pos>, // ❌ 不應該在這裡
}

// 重複借用同一組件
pub struct BadRead<'a> {
    heroes: ReadStorage<'a, Hero>,
}
pub struct BadWrite<'a> {
    hero_storage: ReadStorage<'a, Hero>, // ❌ 重複借用
}
```

✅ **正確做法**：
```rust
// 嚴格分離 Read/Write
pub struct GoodRead<'a> {
    entities: Entities<'a>,
    positions: ReadStorage<'a, Pos>,
    heroes: ReadStorage<'a, Hero>,
}

pub struct GoodWrite<'a> {
    outcomes: Write<'a, Vec<Outcome>>,
    velocities: WriteStorage<'a, Vel>,
}
```

## 📊 性能考量

- **nearby_tick**: 使用多緒排序（4執行緒）優化大量實體
- **skill_tick**: 預載入 JSON 配置，執行時零解析
- **並行處理**: 大部分系統使用 ParJoin 進行平行運算
- **快取友好**: 組件數據連續存儲，提高快取命中率

## 🚀 未來擴展

- 更多技能效果類型（位移、控制、召喚等）
- AI 系統增強（更智能的小兵和塔行為）
- 優化空間分區（四叉樹或八叉樹）
- 預測性網路同步
