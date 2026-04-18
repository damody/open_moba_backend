use specs::storage::VecStorage;
use specs::Component;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
pub struct ItemEffects {
    pub bonus_atk: f32,
    pub bonus_hp: f32,
    pub bonus_mp: f32,
    pub bonus_ms: f32,
    pub bonus_armor: f32,
    pub bonus_mp_regen: f32,
    /// 上次套用到 CProperty/TAttack 的 bonus，用於重算時扣回再加新值
    pub applied_atk: f32,
    pub applied_hp: f32,
    pub applied_ms: f32,
    pub applied_armor: f32,
    /// 重算旗標 — 購買/賣出/使用裝備時設為 true，由 item_tick 處理
    pub dirty: bool,
}

impl Component for ItemEffects {
    type Storage = VecStorage<Self>;
}
