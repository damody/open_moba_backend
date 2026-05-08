//! 階段 3.4：確定性 ECS 狀態雜湊產生器。
//!
//! 行走權威規範「World」並提供穩定的實體子集
//! 狀態（實體 id + `Pos.x/y` raw + `CProperty.hp` raw）透過
//! `omoba_sim::state_hash::hash_sorted_by_id` 用於產生單一 u64
//! 由鎖步客戶端進行非同步檢測。
//!
//! # 為什麼不在 `TickBroadcaster` 內完成？
//!
//! `TickBroadcaster::run()` 是一個 tokio 任務；規範“World”是“!Send”
//! （一些與腳本相關的儲存包含非「發送」內部）。生產者運作
//! 在 `State::tick()` （調度程序線程）中，透過以下方式發布哈希值
//! `crossbeam_channel`，廣播者 `try_recv` 是最新值
//! 以 120Hz 的節奏。
//!
//! # 確定性合約
//!
//! - `Pos.0.x.raw()` / `Pos.0.y.raw()` 是 `i64` Q53.10 定點 —
//! 運行相同的表示伺服器和客戶端。
//! - `CProperty.hp.raw()` 與 `Fixed64` 原始 `i64` 相同（沒有
//! `CProperty` 得到 `0` 所以塔/彈不會漂移哈希輸出
//! 在以不同方式儲存它們的機器之間）。
//! - `hash_sorted_by_id` 中的排序步驟使雜湊不變
//! ECS 儲存/連線順序 — 只有狀態值才重要。
//!
//! 階段 3.4 僅對 `Pos` + `hp` 進行哈希處理。第 4+ 階段可能會增加「Facing」、「Vel」、
//! 能力冷卻時間等 - 但添加字段會破壞固定，所以應該是
//! 在一次遷移中完成。

use specs::{Join, World, WorldExt};

use omoba_sim::state_hash::hash_sorted_by_id;

use crate::comp::creep::CProperty;
use crate::comp::facing::Facing;
use crate::comp::phys::{Pos, Vel};

/// 每個狀態哈希滴答的穩定子集進行哈希處理。 `#[derive(Hash)]` 訂單匹配
/// 現場申報單；在不破壞協議的情況下不要重新安排
/// 版本（客戶端與這個確切的位元組序列進行比較）。
///
/// 第 4 階段從「(id, pos.x, pos.y, hp)」擴大到增加速度 + 朝向
/// （勾選）。 BuffStore 聚合仍然被排除在外——它們是一個資源
/// 不是每個實體元件，且 BuffStore 線路負載遷移是
/// 計劃進行第 4d 階段（76 個第 2 階段標記清理）。
#[derive(std::hash::Hash)]
struct HashItem {
    id: u32,
    pos_x_raw: i64,
    pos_y_raw: i64,
    vel_x_raw: i64,
    vel_y_raw: i64,
    facing_ticks: i32,
    hp_raw: i64,
}

/// 對每個具有“Pos”的實體計算確定性狀態哈希
/// 成分。沒有 `Vel` / `Facing` / `CProperty` 的實體取代零
/// 所以它們的缺席/存在（例如塔與小兵）不會改變哈希值
/// 僅用於外觀差異。
pub fn compute_state_hash(world: &World) -> u64 {
    let entities = world.entities();
    let pos_storage = world.read_storage::<Pos>();
    let vel_storage = world.read_storage::<Vel>();
    let facing_storage = world.read_storage::<Facing>();
    let cprop_storage = world.read_storage::<CProperty>();

    let items: Vec<HashItem> = (&entities, &pos_storage)
        .join()
        .map(|(e, pos)| {
            let (vel_x_raw, vel_y_raw) = vel_storage
                .get(e)
                .map(|v| (v.0.x.raw(), v.0.y.raw()))
                .unwrap_or((0, 0));
            let facing_ticks = facing_storage
                .get(e)
                .map(|f| f.0.ticks())
                .unwrap_or(0);
            HashItem {
                id: e.id(),
                pos_x_raw: pos.0.x.raw(),
                pos_y_raw: pos.0.y.raw(),
                vel_x_raw,
                vel_y_raw,
                facing_ticks,
                hp_raw: cprop_storage.get(e).map(|c| c.hp.raw()).unwrap_or(0),
            }
        })
        .collect();

    hash_sorted_by_id(&items, |i| i.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use omoba_sim::{Fixed64, Vec2 as SimVec2};
    use specs::{Builder, World, WorldExt};

    fn make_world() -> World {
        let mut w = World::new();
        w.register::<Pos>();
        w.register::<Vel>();
        w.register::<Facing>();
        w.register::<CProperty>();
        w
    }

    fn cprop(hp: i32, mhp: i32) -> CProperty {
        CProperty {
            hp: Fixed64::from_i32(hp),
            mhp: Fixed64::from_i32(mhp),
            msd: Fixed64::ZERO,
            def_physic: Fixed64::ZERO,
            def_magic: Fixed64::ZERO,
        }
    }

    fn pos_xy(x: i32, y: i32) -> Pos {
        Pos(SimVec2 {
            x: Fixed64::from_i32(x),
            y: Fixed64::from_i32(y),
        })
    }

    #[test]
    fn empty_world_hashes_deterministically() {
        let w1 = make_world();
        let w2 = make_world();
        assert_eq!(compute_state_hash(&w1), compute_state_hash(&w2));
    }

    #[test]
    fn pos_change_changes_hash() {
        let mut w1 = make_world();
        w1.create_entity().with(pos_xy(10, 20)).build();
        let h1 = compute_state_hash(&w1);

        let mut w2 = make_world();
        w2.create_entity().with(pos_xy(11, 20)).build();
        let h2 = compute_state_hash(&w2);

        assert_ne!(h1, h2, "moving an entity must change the hash");
    }

    #[test]
    fn hp_change_changes_hash() {
        let mut w1 = make_world();
        w1.create_entity().with(pos_xy(0, 0)).with(cprop(100, 100)).build();
        let h1 = compute_state_hash(&w1);

        let mut w2 = make_world();
        w2.create_entity().with(pos_xy(0, 0)).with(cprop(99, 100)).build();
        let h2 = compute_state_hash(&w2);

        assert_ne!(h1, h2, "HP change must affect hash");
    }

    #[test]
    fn vel_change_changes_hash() {
        let mut w1 = make_world();
        w1.create_entity()
            .with(pos_xy(0, 0))
            .with(Vel(SimVec2 { x: Fixed64::from_i32(1), y: Fixed64::ZERO }))
            .build();
        let h1 = compute_state_hash(&w1);

        let mut w2 = make_world();
        w2.create_entity()
            .with(pos_xy(0, 0))
            .with(Vel(SimVec2 { x: Fixed64::from_i32(2), y: Fixed64::ZERO }))
            .build();
        let h2 = compute_state_hash(&w2);

        assert_ne!(h1, h2, "Vel change must affect hash");
    }

    #[test]
    fn facing_change_changes_hash() {
        use omoba_sim::Angle;
        let mut w1 = make_world();
        w1.create_entity()
            .with(pos_xy(0, 0))
            .with(Facing(Angle::from_ticks(0)))
            .build();
        let h1 = compute_state_hash(&w1);

        let mut w2 = make_world();
        w2.create_entity()
            .with(pos_xy(0, 0))
            .with(Facing(Angle::from_ticks(1024)))
            .build();
        let h2 = compute_state_hash(&w2);

        assert_ne!(h1, h2, "Facing change must affect hash");
    }

    #[test]
    fn missing_cproperty_uses_zero() {
        // 兩個世界，一個有 hp=0 顯式，一個沒有 CProperty：應該
        // 產生相同的雜湊，因為我們用 hp_raw=0 代替丟失。
        let mut w1 = make_world();
        w1.create_entity().with(pos_xy(5, 5)).build();
        let h1 = compute_state_hash(&w1);

        let mut w2 = make_world();
        w2.create_entity().with(pos_xy(5, 5)).with(cprop(0, 0)).build();
        let h2 = compute_state_hash(&w2);

        assert_eq!(h1, h2, "no-CProperty must hash same as hp=0");
    }
}
