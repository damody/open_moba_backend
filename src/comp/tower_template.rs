//! TD 模式的 4 種基礎塔模板。
//!
//! 塔的屬性由 [`TowerKind`] 決定，從 [`TowerKind::template`] 讀取。
//! 實際 spawn 走 [`spawn_td_tower`]，被 `state::resource_management::create_tower`
//! 在 `GameMode::TowerDefense` 時呼叫。

use serde::{Deserialize, Serialize};
use specs::{Builder, Component, DenseVecStorage, Entity, VecStorage, World, WorldExt};
use vek::Vec2;

use super::*;

/// 四種 TD 基礎塔。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TowerKind {
    /// 單體快射
    Dart,
    /// AoE 範圍傷害
    Bomb,
    /// 一次 8 發放射狀、近距離
    Tack,
    /// 範圍減速
    Ice,
}

impl Component for TowerKind {
    type Storage = DenseVecStorage<Self>;
}

/// Tack 放射針的命中半徑（backend 單位）；projectile_tick 與 handle_projectile_directional
/// 共用，前端也會收這個值畫圈讓玩家看見命中範圍。
pub const TACK_NEEDLE_HIT_RADIUS: f32 = 80.0;

/// 減速 debuff：命中的 creep 在 `remaining` 秒內，移動速度乘上 `factor`。
/// 由 `projectile_tick` 在命中有 slow_factor>0 的 projectile 時 push `Outcome::ApplySlow`，
/// GameProcessor 再附加這個 Component；`slow_buff_tick` 每 tick 扣 remaining，歸零移除。
#[derive(Clone, Copy, Debug)]
pub struct SlowBuff {
    pub factor: f32,
    pub remaining: f32,
}

impl Component for SlowBuff {
    type Storage = VecStorage<Self>;
}

impl TowerKind {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "dart" => Some(TowerKind::Dart),
            "bomb" => Some(TowerKind::Bomb),
            "tack" => Some(TowerKind::Tack),
            "ice" => Some(TowerKind::Ice),
            _ => None,
        }
    }

    pub fn key(&self) -> &'static str {
        match self {
            TowerKind::Dart => "dart",
            TowerKind::Bomb => "bomb",
            TowerKind::Tack => "tack",
            TowerKind::Ice => "ice",
        }
    }

    pub fn all() -> &'static [TowerKind] {
        &[TowerKind::Dart, TowerKind::Bomb, TowerKind::Tack, TowerKind::Ice]
    }

    pub fn template(&self) -> TowerTemplate {
        match self {
            TowerKind::Dart => TowerTemplate {
                kind: *self,
                label: "Dart Monkey",
                cost: 200,
                atk: 10.0,
                asd: 0.8,
                range: 350.0,
                hp: 1.0,
                bullet_speed: 1200.0,
                footprint: 40.0,
                projectiles_per_shot: 1,
                splash_radius: 0.0,
                slow_factor: 0.0,
                slow_duration: 0.0,
                turn_speed_deg: 360.0,
            },
            TowerKind::Bomb => TowerTemplate {
                kind: *self,
                label: "Bomb Shooter",
                cost: 650,
                atk: 30.0,
                asd: 1.5,
                range: 400.0,
                hp: 1.0,
                bullet_speed: 900.0,
                footprint: 50.0,
                projectiles_per_shot: 1,
                splash_radius: 200.0, // 2x
                slow_factor: 0.0,
                slow_duration: 0.0,
                turn_speed_deg: 360.0,
            },
            TowerKind::Tack => TowerTemplate {
                kind: *self,
                label: "Tack Shooter",
                cost: 400,
                atk: 8.0,
                asd: 1.2,
                range: 380.0, // 加大以利八方向放針效果
                hp: 1.0,
                bullet_speed: 1400.0,
                footprint: 40.0,
                projectiles_per_shot: 8,
                splash_radius: 0.0,
                slow_factor: 0.0,
                slow_duration: 0.0,
                turn_speed_deg: 3600.0, // 瞬轉
            },
            TowerKind::Ice => TowerTemplate {
                kind: *self,
                label: "Ice Monkey",
                cost: 400,
                atk: 3.0,
                asd: 1.5,
                range: 180.0,
                hp: 1.0,
                bullet_speed: 600.0,
                footprint: 40.0,
                projectiles_per_shot: 1,
                splash_radius: 90.0,
                slow_factor: 0.5,
                slow_duration: 2.0,
                turn_speed_deg: 360.0,
            },
        }
    }
}

pub struct TowerTemplate {
    pub kind: TowerKind,
    pub label: &'static str,
    pub cost: i32,
    /// 攻擊力（每次命中對單體的物理傷害）
    pub atk: f32,
    /// 攻擊間隔（秒）
    pub asd: f32,
    /// 射程
    pub range: f32,
    pub hp: f32,
    pub bullet_speed: f32,
    /// 放置時的圓形碰撞半徑；用於蓋塔重疊檢查
    pub footprint: f32,
    /// 一次射擊發射的彈數（Tack = 8，其他 = 1）
    pub projectiles_per_shot: u32,
    /// 命中後的 AoE 半徑（0 表示單體）
    pub splash_radius: f32,
    /// 命中目標的移動速度乘數（0 表示不減速，0.5 表示減速到 50%）
    pub slow_factor: f32,
    /// 減速持續秒數
    pub slow_duration: f32,
    /// 塔轉向速度（度/秒）；Tack 需要極大值（3600 = 幾乎瞬發）
    pub turn_speed_deg: f32,
}

/// 在指定位置 spawn 一座 TD 塔。回傳新建的 entity。
/// 走最簡流程：Pos + Tower + TProperty + TAttack + Faction(Player) + Facing + CollisionRadius + Vision。
pub fn spawn_td_tower(world: &mut World, pos: Vec2<f32>, kind: TowerKind) -> Entity {
    let tpl = kind.template();
    let tprop = TProperty::new(tpl.hp, 0, 120.0);
    let tatk = TAttack::new(tpl.atk, tpl.asd, tpl.range, tpl.bullet_speed);
    let faction = Faction::new(FactionType::Player, 0);
    let vision = CircularVision::new(tpl.range + 100.0, 40.0).with_precision(120);
    // 塔的 HP 借用 CProperty.hp 以供既有傷害系統讀（此處設 1 表示塔不會被傷，因為氣球不會攻擊）
    let cprop = CProperty {
        hp: tpl.hp,
        mhp: tpl.hp,
        msd: 0.0,
        def_physic: 0.0,
        def_magic: 0.0,
    };

    let mut builder = world
        .create_entity()
        .with(Pos(pos))
        .with(Tower::new())
        .with(tprop)
        .with(cprop)
        .with(tatk)
        .with(faction)
        .with(vision)
        .with(Facing(0.0))
        .with(TurnSpeed(tpl.turn_speed_deg.to_radians()))
        .with(CollisionRadius(tpl.footprint))
        .with(kind); // 供 handle_projectile 查 splash/slow、tower_tick 查 multi-shot

    // 4 塔全部走腳本驅動；找不到腳本時 dispatch 會 no-op，塔就會變成啞巴（log::warn）
    let script_id: &'static str = match kind {
        TowerKind::Dart => "tower_dart",
        TowerKind::Bomb => "tower_bomb",
        TowerKind::Tack => "tower_tack",
        TowerKind::Ice  => "tower_ice",
    };
    builder = builder.with(crate::scripting::ScriptUnitTag { unit_id: script_id.to_string() });

    let entity = builder.build();

    // 排入 Spawn 事件，讓腳本 on_spawn 初始化 stats（atk/asd/range）。
    // 下一個 tick 的 run_script_dispatch 會先處理 Spawn，再跑 on_tick。
    world.write_resource::<crate::scripting::ScriptEventQueue>()
        .push(crate::scripting::ScriptEvent::Spawn { e: entity });

    entity
}
