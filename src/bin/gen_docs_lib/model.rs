use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnitKind { Tower, Hero, Creep, Unknown }

#[derive(Debug, Clone, Default)]
pub struct TowerStats {
    pub atk: f32,
    pub asd_interval: f32,
    pub range: f32,
    pub bullet_speed: f32,
    pub splash_radius: f32,
    pub hit_radius: f32,
    pub slow_factor: f32,
    pub slow_duration: f32,
    pub cost: i32,
    pub footprint: f32,
    pub hp: f32,
    pub turn_speed_deg: f32,
    pub label: String,
}

#[derive(Debug, Clone, Default)]
pub struct HeroInfo {
    pub name: String,
    pub title: String,
    pub background: String,
    pub strength: f32,
    pub agility: f32,
    pub intelligence: f32,
    pub primary_attribute: String,
    pub attack_range: f32,
    pub base_damage: f32,
    pub base_armor: f32,
    pub base_hp: f32,
    pub base_mana: f32,
    pub move_speed: f32,
    pub turn_speed: f32,
    pub abilities: Vec<String>,
    pub level_growth: serde_json::Value,
}

#[derive(Debug, Clone, Default)]
pub struct CreepInfo {
    pub name: String,
    pub enemy_type: String,
    pub hp: f32,
    pub armor: f32,
    pub magic_resistance: f32,
    pub damage: f32,
    pub attack_range: f32,
    pub move_speed: f32,
    pub ai_type: String,
    pub abilities: Vec<String>,
    pub exp_reward: i32,
    pub gold_reward: i32,
}

#[derive(Debug, Clone)]
pub struct UnitEntry {
    pub id: String,
    pub kind: UnitKind,
    pub label: Option<String>,
    pub tower: Option<TowerStats>,
    pub hero: Option<HeroInfo>,
    pub creep: Option<CreepInfo>,
    pub abilities: Vec<String>,
    pub overrides: Vec<String>,
    pub world_calls: BTreeSet<String>,
    pub source_file: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AbilityEntry {
    pub id: String,
    pub def_json: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ApiGroup {
    UnitHook,
    AbilityHook,
    WorldQuery,
    WorldMutate,
    WorldTower,
    WorldStats,
    WorldRng,
    WorldLog,
    WorldVfx,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StatSection {
    All,
    NonBuilding,
    Visual,
}

#[derive(Debug, Clone)]
pub struct ApiMethod {
    pub name: String,
    pub signature: String,
    pub doc: String,
    pub group: ApiGroup,
    pub sub_group: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StatKey {
    pub const_name: String,
    pub string_value: String,
    pub doc: String,
    pub section: StatSection,
    pub sub_group: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ApiSpec {
    pub unit_hooks: Vec<ApiMethod>,
    pub ability_hooks: Vec<ApiMethod>,
    pub world_methods: Vec<ApiMethod>,
    pub stat_keys: Vec<StatKey>,
}

#[derive(Debug, Clone)]
pub struct Warning {
    pub source: String,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct BuildMeta {
    pub timestamp: String,
    pub git_sha: String,
    pub story: String,
    pub sources: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Catalog {
    pub units: Vec<UnitEntry>,
    pub abilities: Vec<AbilityEntry>,
    pub api: ApiSpec,
    pub warnings: Vec<Warning>,
    pub meta: BuildMeta,
}
