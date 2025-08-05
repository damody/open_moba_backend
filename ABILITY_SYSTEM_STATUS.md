# Ability System 重構狀態記錄

## 當前狀態記錄

### 已完成的工作
1. **重構ability-system為純邏輯庫** ✅
   - 完全重寫了 `/mnt/d/Nobu/open_moba_backend/ability-system/src/lib.rs`
   - 移除了複雜的trait抽象，直接使用specs::Entity
   - 創建了簡單的數據結構：AbilityConfig, AbilityRequest, AbilityEffect, AbilityState
   - 實現了AbilityProcessor純邏輯處理器

2. **創建ECS整合組件** ✅
   - 新增 `/mnt/d/Nobu/open_moba_backend/src/comp/ability_comp.rs`
   - 包含AbilityComponent, AbilityRequestComponent, AbilityResultComponent
   - 已加入到mod.rs中

3. **創建新技能系統** ✅
   - 新增 `/mnt/d/Nobu/open_moba_backend/src/tick/new_ability_tick.rs`
   - 實現NewAbilitySystem，使用JsonPreprocessor載入配置
   - 包含完整的技能處理邏輯和效果應用

4. **創建助手函數** ✅
   - 新增 `/mnt/d/Nobu/open_moba_backend/src/util/ability_helper.rs`
   - 提供便利的技能請求創建函數

5. **JSON配置完成** ✅
   - `/mnt/d/Nobu/open_moba_backend/ability-configs/sniper_abilities.json` 包含完整的狙擊手技能配置

### 修復的依賴問題
- 添加了 `serde` feature 到 specs 依賴
- 添加了 `fastrand = "2.0"` 依賴
- 修復了JsonPreprocessor的import路徑

### 最新修復（2024年）
✅ **所有編譯錯誤已完全修復！**
1. 按照用戶要求，整合ability-system到現有的skill_tick.rs中，而非創建新的系統
2. 修復SystemData derive宏問題：啟用specs的"derive"功能
3. 修復DamageType缺少Hash和Eq traits
4. 修復CProperty字段名稱（health→hp, max_health→mhp）
5. 修復Entity創建方法
6. 添加缺失依賴：sorted_intersection
7. 修復round::floor調用問題
8. 更新voracious_radix_sort到多緒執行版本，使用voracious_mt_sort(4)

### 性能優化更新
✅ **voracious_radix_sort多緒優化**
- Cargo.toml: 添加 `features = ["voracious_multithread"]`
- nearby_tick.rs: 更新為 `voracious_mt_sort(4)` 使用4個執行緒
- 提升排序性能，特別是在處理大量實體時

### 當前整合架構
按照用戶指示，**不創建新的ability_tick**，而是將ability-system整合到現有的skill_tick.rs中：
- 在skill_tick.rs的Sys中添加AbilityProcessor
- 先嘗試使用ability-system處理技能，失敗則回退到硬編碼邏輯
- 支援JSON配置的技能與原有硬編碼技能共存
- 提供SkillInput↔AbilityRequest和AbilityEffect↔SkillEffect轉換

### TODO狀態
```
✅ 重構ability-system為純邏輯庫 (completed)
✅ 創建支援註解的JSON配置 (completed)
✅ 修復編譯錯誤 (completed) - 所有錯誤已完全修復！
✅ 整合specs ECS與技能系統 (completed) - 整合到skill_tick.rs
🔄 測試新技能系統 (in_progress)
```

### 下次繼續需要做的
1. ✅ 已完成：修復所有編譯錯誤 
2. ✅ 已完成：整合到現有skill_tick.rs系統中
3. 🔄 正在進行：測試整合後的技能系統功能
4. ⏳ 待完成：驗證JSON配置技能是否正常工作

### 關鍵文件位置
- 核心邏輯：`ability-system/src/lib.rs`
- ECS組件：`src/comp/ability_comp.rs`
- 技能系統整合：`src/tick/skill_tick.rs`
- JSON配置範例：`ability-configs/sniper_abilities.json`

---

## 📖 如何添加新技能到子系統 - Step by Step 教學

### Step 1: 創建技能JSON配置檔案

在 `ability-configs/` 目錄下創建新的JSON檔案，例如 `warrior_abilities.json`：

```json
{
    // 技能配置支援C風格註解
    "abilities": [
        {
            "id": "shield_bash",               // 技能唯一ID
            "name": "盾擊",                    // 技能名稱
            "description": "用盾牌猛擊敵人",   // 技能描述
            "type": "instant",                 // 技能類型：instant/channel/toggle
            "max_level": 4,                    // 最大等級
            "cooldown": [10.0, 9.0, 8.0, 7.0], // 各等級冷卻時間
            "cost": [50, 60, 70, 80],          // 各等級消耗（魔力/能量）
            "range": [150.0, 150.0, 150.0, 150.0], // 各等級施法距離
            
            // 技能數值配置
            "damage": [100.0, 150.0, 200.0, 250.0],  // 各等級傷害
            "stun_duration": [1.0, 1.5, 2.0, 2.5],   // 各等級暈眩時間
            
            // 額外屬性（可選）
            "properties": {
                "damage_type": "physical",      // 傷害類型
                "can_miss": false,              // 是否可被閃避
                "pierce_immunity": false        // 是否穿透免疫
            }
        }
    ]
}
```

### Step 2: 在AbilityProcessor中實現技能邏輯

編輯 `ability-system/src/lib.rs`，在 `generate_effects` 方法中添加新技能邏輯：

```rust
fn generate_effects(&self, request: &AbilityRequest, _config: &AbilityConfig, level_data: &AbilityLevelData) -> Vec<AbilityEffect> {
    let mut effects = Vec::new();

    match request.ability_id.as_str() {
        // 現有技能...
        
        "shield_bash" => {
            // 從JSON配置中獲取數值
            let damage = level_data.properties.get("damage")
                .and_then(|v| v.as_f64())
                .unwrap_or(100.0) as f32;
            let stun_duration = level_data.properties.get("stun_duration")
                .and_then(|v| v.as_f64())
                .unwrap_or(1.0) as f32;
            
            if let Some(target) = request.target_entity {
                // 造成傷害
                effects.push(AbilityEffect::Damage {
                    target,
                    amount: damage,
                });
                
                // 施加暈眩效果
                effects.push(AbilityEffect::StatusModifier {
                    target,
                    modifier_type: "stun".to_string(),
                    value: 1.0,  // 暈眩強度
                    duration: Some(stun_duration),
                });
            }
        }
        
        _ => {}
    }
    
    effects
}
```

### Step 3: 確保JSON檔案被載入

在 `src/tick/skill_tick.rs` 的 `ensure_initialized` 方法中添加新的配置檔案：

```rust
fn ensure_initialized(&mut self) -> Result<(), Box<dyn std::error::Error>> {
    if self.initialized {
        return Ok(());
    }

    let mut processor = AbilityProcessor::new();
    
    // 載入技能配置文件
    let config_files = vec![
        "ability-configs/sniper_abilities.json",
        "ability-configs/warrior_abilities.json",  // 新增配置檔案
        // 其他配置檔案...
    ];
    
    for config_path in config_files {
        if let Ok(content) = fs::read_to_string(config_path) {
            let processed_content = JsonPreprocessor::remove_comments(&content);
            if let Err(e) = processor.load_from_json(&processed_content) {
                error!("載入技能配置失敗 {}: {}", config_path, e);
            } else {
                info!("成功載入技能配置: {}", config_path);
            }
        }
    }
    
    self.ability_processor = Some(processor);
    self.initialized = true;
    Ok(())
}
```

### Step 4: 在遊戲中設定技能

確保英雄或單位的 `Skill` 組件設定了正確的 `ability_id`：

```rust
// 在英雄初始化時
let shield_bash = Skill {
    ability_id: "shield_bash".to_string(),  // 對應JSON中的ID
    owner: warrior_entity,
    current_level: 1,
    max_level: 4,
    cooldown_remaining: 0.0,
    charges: 1,
    max_charges: 1,
    toggle_state: false,
};
```

### Step 5: 處理特殊效果（如果需要）

如果技能有特殊效果需要在ECS中處理，可以在 `apply_ability_effect_as_skill_effect` 中添加：

```rust
fn apply_ability_effect_as_skill_effect(
    effect: AbilityEffect,
    tr: &SkillRead,
    tw: &mut SkillWrite,
) {
    match effect {
        // 現有效果處理...
        
        AbilityEffect::StatusModifier { target, modifier_type, value, duration } => {
            match modifier_type.as_str() {
                "stun" => {
                    // 創建暈眩效果
                    let stun_effect = SkillEffect::new(
                        "stun_effect".to_string(),
                        target,
                        SkillEffectType::Debuff,
                        duration.unwrap_or(1.0),
                    );
                    // 設定暈眩效果數據
                    // stun_effect.data.stun = true;
                    
                    let effect_entity = tr.entities.create();
                    tw.skill_effects.insert(effect_entity, stun_effect);
                }
                // 其他狀態效果...
                _ => {}
            }
        }
        _ => {}
    }
}
```

### 📋 完整流程總結

1. **創建JSON配置** → 定義技能屬性和數值
2. **實現技能邏輯** → 在AbilityProcessor中編寫效果生成
3. **載入配置檔案** → 確保系統初始化時讀取JSON
4. **設定技能組件** → 為單位添加對應的ability_id
5. **處理特殊效果** → 在ECS層面實現特定的效果邏輯

### 🔧 除錯提示

- 查看日誌確認JSON是否成功載入：`"成功載入技能配置: ability-configs/xxx.json"`
- 檢查 `try_process_with_ability_system` 是否返回 true
- 確認技能ID在JSON和代碼中完全一致
- 使用 `--log-level debug` 查看詳細的技能處理流程

### 💡 進階功能

1. **條件觸發**：在JSON中添加 `conditions` 欄位定義施法條件
2. **連擊系統**：使用 `combo_window` 和 `next_ability` 欄位
3. **充能技能**：設定 `max_charges > 1` 和 `charge_restore_time`
4. **範圍效果**：使用 `AbilityEffect::AreaEffect` 處理AOE技能
- 配置文件：`ability-configs/sniper_abilities.json`
- 助手函數：`src/util/ability_helper.rs`

### 🧹 清理工作（2024年最新）
✅ **已刪除不需要的檔案**：
- 刪除了 `src/tick/ability_tick.rs` - 舊的複雜ability系統
- 刪除了 `src/tick/new_ability_tick.rs` - 不需要新系統，已整合到skill_tick.rs
- 刪除了 `src/comp/ability_bridge.rs` - 不需要的事件橋接系統

**最終架構：極簡且高效**
- 所有技能邏輯都在 `skill_tick.rs` 中處理
- AbilityProcessor 作為子系統被整合使用
- JSON配置技能與硬編碼技能無縫共存

### 技術架構說明
系統架構已經按照用戶要求簡化：
- **只處理技能邏輯**：ability-system crate 僅包含技能配置、狀態和效果處理
- **與specs ECS整合**：直接使用 specs::Entity，通過組件系統整合
- **JSON配置**：使用支援C風格註解的JSON配置文件
- **純邏輯庫**：ability-system 不包含ECS依賴，返回效果數據由主程序處理

### 用戶反饋紀錄
- ❌ 原先的複雜trait設計被拒絕：「你這個設計不符合我的想法」
- ✅ 要求「技能子系統應該只處理技能，且要與RUST ecs crate specs 要能整合在一起」
- ✅ 改用「可用註解的json」而非YAML

### 下次啟動時的重點
重新啟動時請優先處理編譯錯誤，特別是舊ability系統代碼的清理，然後繼續整合測試新系統。

---

## 🚧 當前運行時問題（2024-08-05）

### 多線程資源借用衝突

當前存在一個持續的資源借用問題：

1. **問題描述**: 多個 ECS 系統同時嘗試訪問相同資源導致 "already borrowed" 錯誤
2. **影響範圍**: 
   - `CProperty` 組件在多個系統中都需要寫入訪問
   - `Vec<Outcome>` 資源在多個系統中都需要寫入訪問
3. **錯誤信息**: `Tried to fetch data of type "alloc::boxed::Box<dyn shred::world::Resource>", but it was already borrowed.`

### 已嘗試的解決方案

1. ✅ **系統依賴順序** - 設置了正確的系統執行依賴關係
2. ✅ **修復 death_tick** - 將 `CProperty` 從 `DeathWrite` 移至 `DeathRead` 
3. ✅ **使用原本的 Dispatcher** - 恢復用戶原本的 dispatch 寫法，加上相依性
4. ❌ **多種嘗試均失敗** - 問題持續存在

### 當前狀態

- ✅ 程序可以成功編譯
- ✅ 系統成功載入了技能配置檔案 `ability-configs/sniper_abilities.json`
- ✅ 戰役資料和場景創建成功
- ❌ 在第一次 ECS tick 時發生資源借用衝突導致 panic

### 🎯 問題根本原因已找到！

**用戶提供的關鍵信息：**
> "所有的內容修改都必需要存到Outcome給 state.rs 在 process_outcomes 去更新，你不能在自己的tick去更新，你只能把更新事件塞給 Vec<Outcome>"

### 架構設計原則

這個專案使用**事件驅動架構**：
1. ❌ **錯誤做法**: 各個 tick 系統直接修改組件（如 `WriteStorage<CProperty>`）
2. ✅ **正確做法**: 所有狀態修改必須通過 `Vec<Outcome>` 事件系統
3. 🔄 **統一處理**: `state.rs` 的 `process_outcomes()` 負責處理所有狀態變更

### 修復計劃

1. **移除直接寫入訪問**: 所有系統的 `WriteStorage` 都要改為 `ReadStorage`
2. **使用事件模式**: 所有狀態變更改為生成對應的 `Outcome` 事件
3. **集中處理**: 確保 `process_outcomes()` 能處理所有需要的事件類型

### 影響範圍

需要修改的系統：
- `skill_tick.rs` - 技能效果不能直接修改 CProperty，要生成 Outcome 事件
- `damage_tick.rs` - 傷害處理改為事件驅動
- `hero_tick.rs` - 英雄狀態變更改為事件
- `creep_tick.rs` - 小兵狀態變更改為事件
- `death_tick.rs` - 死亡處理改為事件（部分已修復）

這解釋了為什麼會有資源借用衝突 - 多個系統同時嘗試寫入相同組件，違反了專案的事件驅動架構設計！

### 🔄 當前修復進度（2024-08-05 更新）

#### ✅ 已完成的修復

1. **添加 Damage 和 Heal 事件處理** - 在 `process_outcomes` 中實現
2. **修復 damage_tick.rs** - 改為生成 `Damage` 事件而非直接修改 CProperty
3. **修復 skill_tick.rs** - 治療改為生成 `Heal` 事件
4. **修復 creep_tick.rs** - 傷害計算改為生成 `Damage` 事件
5. **修復 hero_tick.rs** - CProperty 改為只讀訪問
6. **修復 death_tick.rs** - CProperty 改為只讀訪問（之前已修復）

#### 🚧 發現的剩餘問題

**多個系統同時寫入相同組件**，導致資源借用衝突：

1. **TAttack 組件衝突**:
   - `hero_tick.rs` 和 `skill_tick.rs` 都有 `WriteStorage<'a, TAttack>`

2. **Hero 組件衝突**:
   - `death_tick.rs` 和 `hero_tick.rs` 都有 `WriteStorage<'a, Hero>`

3. **Tower 組件衝突**:
   - `nearby_tick.rs`, `player_tick.rs`, `tower_tick.rs` 都有 `WriteStorage<'a, Tower>`

4. **Pos/位置組件衝突**:
   - `projectile_tick.rs` 和 `creep_tick.rs` 都有 `WriteStorage<'a, Pos>`

#### 📋 待修復清單

1. 分析每個系統是否真的需要對這些組件的寫入訪問
2. 將不必要的寫入改為只讀，或改為事件驅動模式
3. 為可能需要的組件更新創建新的 `Outcome` 事件類型
4. 逐步消除所有組件寫入衝突

#### 💡 解決策略

根據事件驅動架構原則，大多數組件更新都應該通過 `Outcome` 事件在 `process_outcomes()` 中集中處理，而不是在各個 tick 系統中直接修改。