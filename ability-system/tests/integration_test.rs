//! 技能系統過渡測試
//!
//! 原本這裡有 8 個英雄 handler（Saika×4、Date×4）的 Rust-native 註冊測試。
//! 所有 handler 已搬至 `scripts/base_content/src/heroes/`，改以 `AbilityScript`
//! FFI trait + abi_stable DLL 方式註冊；本 sub-crate 的 `AbilityProcessor`
//! 降為過渡殼，預設空 registry。
//!
//! 真正的英雄技能測試請看：
//! - `scripts/base_content/tests/`（DLL 端單元測試，Phase 3+ 補齊）
//! - MCP `list_abilities` 整合驗證（server 啟動後查詢 metadata）

use ability_system::AbilityProcessor;

#[test]
fn test_ability_processor_starts_empty() {
    let processor = AbilityProcessor::new();
    let registry = processor.get_registry();
    assert!(
        registry.is_empty(),
        "AbilityProcessor should start with empty registry after handler migration to scripts/base_content"
    );
}
