//! 階段 5.3：觀察者重新加入的確定性 ECS 世界快照。
//!
//! 產生權威規範的緊湊二進制碼序列化子集
//! `World`（實體 id + pos + vel + faces + hp/mhp + kind 標籤）以及
//! `master_seed` 和 `tick` 這樣加入遊戲中期的觀察者就可以引導
//! 其 sim_runner 狀態然後透過後續的 TickBatches 向前播放。
//!
//! 從調度程序滴答循環（在“State::tick()”中）運行
//! `SNAPSHOT_INTERVAL_TICKS` 刻度（= 30 s @ 30 Hz 調度程式）。輸出
//! 位元組儲存在「SnapshotStore」資源中； KCP 運輸
//! 0x16 SnapshotResp 處理程序從共用「Arc<Mutex<SnapshotStore>>」讀取。
//!
//! # 模式版本控制
//!
//! `WorldSnapshot::schema_version` 固定在 `SCHEMA_VERSION = 1`。這
//! omfx 端 LockstepClient 根據其編譯的預期檢查此內容
//! 應用位元組之前的版本；不匹配的情況會從
//! 沒有引導的當前刻度。 **將欄位新增至末尾
//! 僅“EntitySnapshot”，並在執行此操作時碰撞 SCHEMA_VERSION**，因為
//! bincode 是位置敏感的。
//!
//! 階段 5.3 發布 **伺服器端只寫** — omfx 觀察者
//! 消費者目前僅記錄（反序列化 + 應用是階段 5+
//! 一旦實際執行觀察者模式，就進行後續操作）。

use specs::{Join, World, WorldExt};
use serde::{Deserialize, Serialize};

use crate::comp::creep::CProperty;
use crate::comp::facing::Facing;
use crate::comp::hero::Hero;
use crate::comp::phys::{Pos, Vel};
use crate::comp::projectile::Projectile;
use crate::comp::resources::{MasterSeed, Tick};
use crate::comp::tower::Tower;

/// 線上架構版本。新增/重新排序欄位時出現碰撞
/// “EntitySnapshot”或“WorldSnapshot”。客戶拒絕申請不匹配
/// 版本並回退到無開機重新加入。
pub const SCHEMA_VERSION: u32 = 1;

/// 實體類型標籤 — 與 omfx 端 `EntityKind` 判別式匹配
/// 觀察者重新加入可以將每個實體分派到正確的 sprite/渲染
/// 路徑而無需重新查詢腳本登錄。訂單已固定為 bincode。
#[repr(u8)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum EntityKindTag {
    Other = 0,
    Hero = 1,
    Tower = 2,
    Creep = 3,
    Projectile = 4,
}

/// 每個實體的狀態在快照中傳送。
///
/// **在末尾添加字段，切勿對現有字段重新排序** - bincode 是
/// 位置敏感。任何更改都會影響“SCHEMA_VERSION”。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EntitySnapshot {
    pub id: u32,
    pub pos_x_raw: i64,
    pub pos_y_raw: i64,
    pub vel_x_raw: i64,
    pub vel_y_raw: i64,
    pub facing_ticks: i32,
    pub hp_raw: i64,
    pub mhp_raw: i64,
    pub kind: EntityKindTag,
}

/// 頂級快照框架。 `master_seed` 讓觀察者重新加入
/// 重新播種其“SimRng”流以匹配權威伺服器。
#[derive(Serialize, Deserialize, Debug)]
pub struct WorldSnapshot {
    pub schema_version: u32,
    pub tick: u32,
    pub master_seed: u64,
    pub entities: Vec<EntitySnapshot>,
}

/// 走遍世界，用「Pos」收集每個實體，透過
/// 存在“Hero”/“Tower”/“Projectile”/“CProperty”存儲，以及
/// bincode 透過 `omoba_sim::snapshot::serialize` 對結果進行序列化。
///
/// 序列化失敗時返回空的“Vec”；呼叫者將其視為
/// 作為“沒有快照保存此刻度”和之前的（可能是空的）位元組
/// 留在“SnapshotStore”中。
pub fn serialize_snapshot(world: &World) -> Vec<u8> {
    let entities = world.entities();
    let pos_storage = world.read_storage::<Pos>();
    let vel_storage = world.read_storage::<Vel>();
    let facing_storage = world.read_storage::<Facing>();
    let cprop_storage = world.read_storage::<CProperty>();
    let hero_storage = world.read_storage::<Hero>();
    let tower_storage = world.read_storage::<Tower>();
    let proj_storage = world.read_storage::<Projectile>();

    let snapshot_entities: Vec<EntitySnapshot> = (&entities, &pos_storage)
        .join()
        .map(|(e, pos)| {
            let kind = if hero_storage.get(e).is_some() {
                EntityKindTag::Hero
            } else if tower_storage.get(e).is_some() {
                EntityKindTag::Tower
            } else if proj_storage.get(e).is_some() {
                EntityKindTag::Projectile
            } else if cprop_storage.get(e).is_some() {
                EntityKindTag::Creep
            } else {
                EntityKindTag::Other
            };

            let (vel_x_raw, vel_y_raw) = vel_storage
                .get(e)
                .map(|v| (v.0.x.raw(), v.0.y.raw()))
                .unwrap_or((0, 0));
            let facing_ticks = facing_storage
                .get(e)
                .map(|f| f.0.ticks())
                .unwrap_or(0);
            let (hp_raw, mhp_raw) = cprop_storage
                .get(e)
                .map(|c| (c.hp.raw(), c.mhp.raw()))
                .unwrap_or((0, 0));

            EntitySnapshot {
                id: e.id(),
                pos_x_raw: pos.0.x.raw(),
                pos_y_raw: pos.0.y.raw(),
                vel_x_raw,
                vel_y_raw,
                facing_ticks,
                hp_raw,
                mhp_raw,
                kind,
            }
        })
        .collect();

    let snapshot = WorldSnapshot {
        schema_version: SCHEMA_VERSION,
        tick: world.read_resource::<Tick>().0 as u32,
        master_seed: world.read_resource::<MasterSeed>().0,
        entities: snapshot_entities,
    };

    omoba_sim::snapshot::serialize(&snapshot).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use omoba_sim::{Angle, Fixed64, Vec2 as SimVec2};
    use specs::{Builder, World, WorldExt};

    fn make_world() -> World {
        let mut w = World::new();
        w.register::<Pos>();
        w.register::<Vel>();
        w.register::<Facing>();
        w.register::<CProperty>();
        w.register::<Hero>();
        w.register::<Tower>();
        w.register::<Projectile>();
        w.insert(Tick(0));
        w.insert(MasterSeed::default());
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
    fn empty_world_round_trips() {
        let w = make_world();
        let bytes = serialize_snapshot(&w);
        assert!(!bytes.is_empty(), "even an empty world should serialize the WorldSnapshot envelope");
        let snap: WorldSnapshot = omoba_sim::snapshot::deserialize(&bytes)
            .expect("empty world snapshot must deserialize");
        assert_eq!(snap.schema_version, SCHEMA_VERSION);
        assert_eq!(snap.tick, 0);
        assert_eq!(snap.master_seed, MasterSeed::default().0);
        assert_eq!(snap.entities.len(), 0);

        // 重新序列化反序列化的快照必須產生相同的結果
        // 位元組（bincode 是此模式的規格）。
        let bytes2 = omoba_sim::snapshot::serialize(&snap).expect("re-serialize");
        assert_eq!(bytes, bytes2, "snapshot bytes must round-trip identically");
    }

    #[test]
    fn three_entities_preserve_count_and_seed() {
        let mut w = make_world();
        // 插入一個自訂 MasterSeed，以便我們可以斷言它往返。
        w.insert(MasterSeed(0x1234_5678_9ABC_DEF0));
        w.insert(Tick(123));

        // 普通實體（種類 = 其他，因為沒有 CProperty）
        w.create_entity().with(pos_xy(1, 1)).build();
        // 類似蠕變（存在 C 屬性，無英雄/塔樓/彈）
        w.create_entity()
            .with(pos_xy(2, 2))
            .with(Vel(SimVec2 { x: Fixed64::from_i32(1), y: Fixed64::ZERO }))
            .with(Facing(Angle::from_ticks(1024)))
            .with(cprop(50, 100))
            .build();
        // 普通實體2
        w.create_entity().with(pos_xy(3, 3)).build();

        let bytes = serialize_snapshot(&w);
        let snap: WorldSnapshot = omoba_sim::snapshot::deserialize(&bytes)
            .expect("three-entity snapshot must deserialize");

        assert_eq!(snap.schema_version, SCHEMA_VERSION);
        assert_eq!(snap.tick, 123);
        assert_eq!(snap.master_seed, 0x1234_5678_9ABC_DEF0);
        assert_eq!(snap.entities.len(), 3, "three Pos-bearing entities must survive round-trip");

        // 確認至少一個蠕變分類實體具有匹配
        // 馬力/速度/面值（依種類找出）。
        let creep = snap
            .entities
            .iter()
            .find(|e| e.kind == EntityKindTag::Creep)
            .expect("one Creep-tagged entity expected");
        assert_eq!(creep.hp_raw, Fixed64::from_i32(50).raw());
        assert_eq!(creep.mhp_raw, Fixed64::from_i32(100).raw());
        assert_eq!(creep.facing_ticks, 1024);
    }

    #[test]
    fn schema_version_pinned() {
        // Tripwire：任何未來影響線上模式的變更都必須
        // 要有意識——客戶將他們的預期版本與此相對應
        // 持續的。如果您修改了它，也要更新 omfx LockstepClient
        // lockstep_client.rs 中的觀察者重新加入處理程序。
        assert_eq!(SCHEMA_VERSION, 1);
    }
}
