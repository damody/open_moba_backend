//! `ScriptUnitTag` 將實體標記為綁定了本機腳本。
//! `unit_id` 是 `ScriptRegistry` 的關鍵。

use specs::storage::VecStorage;
use specs::Component;

#[derive(Clone, Debug)]
pub struct ScriptUnitTag {
    pub unit_id: String,
}

impl Component for ScriptUnitTag {
    type Storage = VecStorage<Self>;
}
