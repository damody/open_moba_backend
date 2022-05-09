use serde::{Deserialize, Serialize};
#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub enum AttackType {
    Melee,
    Projectile,
    Beam,
    Shockwave,
    Explosion,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Attack {
    damages: Vec<AttackDamage>,
}

impl Default for Attack {
    fn default() -> Self {
        Self {
            damages: Vec::new(),
        }
    }
}

impl Attack {
    #[must_use]
    pub fn with_damage(mut self, damage: AttackDamage) -> Self {
        self.damages.push(damage);
        self
    }
}
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Damage {
    pub source: DamageSource,
    pub kind: DamageKind,
    pub value: f32,
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub struct AttackDamage {
    damage: Damage,
}

impl AttackDamage {
    pub fn new(damage: Damage) -> Self {
        Self {
            damage,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Explosion {
    pub radius: f32,
}

#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub enum DamageSource {
    Melee,
    Projectile,
    Explosion,
    Falling,
    Shockwave,
    Energy,
    Other,
}

/// DamageKind for the purpose of differentiating damage reduction
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub enum DamageKind {
    /// Bypasses some protection from armor
    Piercing,
    /// Reduces energy of target, dealing additional damage when target energy
    /// is 0
    Slashing,
    /// Deals additional poise damage the more armored the target is
    Crushing,
    /// Catch all for remaining damage kinds (TODO: differentiate further with
    /// staff/sceptre reworks
    Energy,
}