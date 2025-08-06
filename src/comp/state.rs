/// 遊戲狀態模組 - 重新導出獨立的 state 模組
/// 
/// 為了保持向後兼容性，這個文件重新導出新的獨立 state 模組

// 重新導出獨立的 State 結構和相關類型
pub use crate::state::{
    State, 
    StateInitializer,
    TimeManager,
    ResourceManager,
    SystemDispatcher,
};
pub use crate::state::core::StateConfig;