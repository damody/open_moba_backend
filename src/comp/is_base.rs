use specs::storage::NullStorage;
use specs::Component;

/// 基地標記（ZST）— 死亡時觸發勝負判定
#[derive(Clone, Copy, Debug, Default)]
pub struct IsBase;

impl Component for IsBase {
    type Storage = NullStorage<Self>;
}
