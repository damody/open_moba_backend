//! `ScriptUnitTag` marks an entity as having a native script bound to it.
//! `unit_id` is the key into `ScriptRegistry`.

use specs::storage::VecStorage;
use specs::Component;

#[derive(Clone, Debug)]
pub struct ScriptUnitTag {
    pub unit_id: String,
}

impl Component for ScriptUnitTag {
    type Storage = VecStorage<Self>;
}
