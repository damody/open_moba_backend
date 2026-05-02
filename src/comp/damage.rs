use specs::storage::VecStorage;
use specs::{Component, Entity};
use serde::{Deserialize, Serialize};
use omoba_sim::Fixed64;

/// 傷害來源信息
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DamageSource {
    pub source_entity: Entity,     // 傷害來源實體
    pub source_type: DamageSourceType,
    pub ability_id: Option<String>, // 如果是技能造成的傷害
}

/// 傷害來源類型
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum DamageSourceType {
    Attack,          // 普通攻擊
    Ability,         // 技能傷害
    Item,            // 物品傷害
    Environment,     // 環境傷害（毒、燃燒等）
    Reflect,         // 反射傷害
}

/// 傷害實例 - 包含完整的傷害信息
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DamageInstance {
    pub target: Entity,
    pub source: DamageSource,
    pub damage_types: DamageTypes,
    pub is_critical: bool,
    pub is_dodged: bool,
    pub damage_flags: DamageFlags,
}

/// 傷害類型組合
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DamageTypes {
    pub physical: Fixed64,
    pub magical: Fixed64,
    pub pure: Fixed64,      // 純粹傷害，無視防禦
}

/// 傷害標記
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DamageFlags {
    pub can_crit: bool,         // 可以暴擊
    pub can_dodge: bool,        // 可以閃避
    pub ignore_armor: bool,     // 無視護甲
    pub ignore_magic_resist: bool, // 無視魔抗
    pub lifesteal: Fixed64,     // 生命偷取比例
    pub spell_vamp: Fixed64,    // 法術吸血比例
}

/// 傷害結果 - 計算後的實際傷害
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DamageResult {
    pub target: Entity,
    pub source: DamageSource,
    pub original_damage: DamageTypes,    // 原始傷害
    pub actual_damage: DamageTypes,      // 實際造成傷害
    pub total_damage: Fixed64,           // 總傷害
    pub absorbed: Fixed64,               // 被護甲/魔抗吸收的傷害
    pub is_critical: bool,
    pub is_dodged: bool,
    pub healing: Fixed64,                // 生命偷取/法術吸血的治療量
}

impl Component for DamageInstance {
    type Storage = VecStorage<Self>;
}

impl Component for DamageResult {
    type Storage = VecStorage<Self>;
}

impl DamageTypes {
    pub fn new(physical: Fixed64, magical: Fixed64, pure: Fixed64) -> Self {
        DamageTypes { physical, magical, pure }
    }

    pub fn physical_only(damage: Fixed64) -> Self {
        DamageTypes { physical: damage, magical: Fixed64::ZERO, pure: Fixed64::ZERO }
    }

    pub fn magical_only(damage: Fixed64) -> Self {
        DamageTypes { physical: Fixed64::ZERO, magical: damage, pure: Fixed64::ZERO }
    }

    pub fn pure_only(damage: Fixed64) -> Self {
        DamageTypes { physical: Fixed64::ZERO, magical: Fixed64::ZERO, pure: damage }
    }

    pub fn total(&self) -> Fixed64 {
        self.physical + self.magical + self.pure
    }

    pub fn is_zero(&self) -> bool {
        self.physical <= Fixed64::ZERO && self.magical <= Fixed64::ZERO && self.pure <= Fixed64::ZERO
    }
}

impl DamageFlags {
    pub fn default_attack() -> Self {
        DamageFlags {
            can_crit: true,
            can_dodge: true,
            ignore_armor: false,
            ignore_magic_resist: false,
            lifesteal: Fixed64::ZERO,
            spell_vamp: Fixed64::ZERO,
        }
    }

    pub fn ability_damage() -> Self {
        DamageFlags {
            can_crit: false,
            can_dodge: false,
            ignore_armor: false,
            ignore_magic_resist: false,
            lifesteal: Fixed64::ZERO,
            spell_vamp: Fixed64::ZERO,
        }
    }

    pub fn true_damage() -> Self {
        DamageFlags {
            can_crit: false,
            can_dodge: false,
            ignore_armor: true,
            ignore_magic_resist: true,
            lifesteal: Fixed64::ZERO,
            spell_vamp: Fixed64::ZERO,
        }
    }
}

impl DamageInstance {
    /// 創建普通攻擊傷害
    pub fn new_attack(source: Entity, target: Entity, physical_damage: Fixed64) -> Self {
        DamageInstance {
            target,
            source: DamageSource {
                source_entity: source,
                source_type: DamageSourceType::Attack,
                ability_id: None,
            },
            damage_types: DamageTypes::physical_only(physical_damage),
            is_critical: false,
            is_dodged: false,
            damage_flags: DamageFlags::default_attack(),
        }
    }
    
    /// 創建技能傷害
    pub fn new_ability(source: Entity, target: Entity, damage_types: DamageTypes, ability_id: String) -> Self {
        DamageInstance {
            target,
            source: DamageSource {
                source_entity: source,
                source_type: DamageSourceType::Ability,
                ability_id: Some(ability_id),
            },
            damage_types,
            is_critical: false,
            is_dodged: false,
            damage_flags: DamageFlags::ability_damage(),
        }
    }
}

impl Default for DamageTypes {
    fn default() -> Self {
        DamageTypes::new(Fixed64::ZERO, Fixed64::ZERO, Fixed64::ZERO)
    }
}

impl Default for DamageFlags {
    fn default() -> Self {
        DamageFlags::default_attack()
    }
}