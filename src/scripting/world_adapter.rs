//! `WorldAdapter` — 在 `&mut specs::World` 上實作 `omb_script_abi::GameWorld`。
//!
//! 僅在串行腳本分發階段（E1）期間存活，因此它成立
//! 獨家「&mut World」；不需要鎖。
//!
//! 當腳本需要更多表面積時在此處新增組件/方法。保留
//! 表面與 PoC-1（以及後續 PoC）實際需要的一樣小。

use abi_stable::std_types::{RNone, ROption, RSome, RStr, RVec};
use crossbeam_channel::Sender;
use omb_script_abi::{
    stat_keys::StatKey,
    types::{Angle, DamageKind, EntityHandle, Fixed64, PathSpec, ProjectileSpec, Target, Vec2},
    world::GameWorld,
};
use rand::{RngCore, SeedableRng};
use rand_pcg::Pcg64Mcg;
use serde_json::json;
use specs::world::Generation;
use specs::{
    Builder, Entities, Entity, Join, LazyUpdate, Read, ReadStorage, World, WorldExt, Write,
    WriteStorage,
};

use crate::ability_runtime::{BuffStore, UnitStats};
use crate::comp::*;
use crate::scripting::event::{ScriptEvent, ScriptEventQueue};
use crate::scripting::tag::ScriptUnitTag;
use crate::transport::OutboundMsg;

/// 預先 fetch 好的 storage / resource 集合。
/// 在整個 `run_script_dispatch` 期間共用，每個 `GameWorld` API 不再重複 borrow。
///
/// 同時 read+write 的 component 統一用 `WriteStorage`（提供 `.get()` / `.get_mut()`）。
/// 同時 read+write 的 resource 統一用 `Write<'_>`（deref 給 read，DerefMut 給 write）。
pub struct AdapterCache<'a> {
    pub entities: Entities<'a>,
    pub lazy: Read<'a, LazyUpdate>,

    // Component storages（read+write 混合，全用 WriteStorage）
    pub tattack: WriteStorage<'a, TAttack>,
    pub pos: WriteStorage<'a, Pos>,
    pub facing: WriteStorage<'a, Facing>,
    pub cprop: WriteStorage<'a, CProperty>,
    pub unit: WriteStorage<'a, Unit>,
    pub hero: WriteStorage<'a, Hero>,

    // 唯讀存儲
    pub faction: ReadStorage<'a, Faction>,
    pub creep: ReadStorage<'a, Creep>,
    pub tower: ReadStorage<'a, Tower>,
    pub is_building: ReadStorage<'a, IsBuilding>,
    pub collision: ReadStorage<'a, CollisionRadius>,
    pub tags: ReadStorage<'a, ScriptUnitTag>,

    // 資源
    pub buffs: Write<'a, BuffStore>,
    pub events: Write<'a, ScriptEventQueue>,
    pub searcher: Read<'a, Searcher>,
    pub blocked: Read<'a, BlockedRegions>,
    pub time: Read<'a, Time>,
    /// 階段 4.2：explosion-FX 佇列（腳本端 `emit_explosion` 推送
    /// 這裡而不是通過“mqtx”； sim_runner 提取器排水管）。
    pub explosion_fx: Write<'a, ExplosionFxQueue>,
    pub tower_fire_fx: Write<'a, TowerFireFxQueue>,
    pub attack_phase_fx: Write<'a, AttackPhaseFxQueue>,
    /// 階段 4.2：目前刻度 — 印在每個 ExplosionFx 上，以便
    /// 渲染側可依 omfx 掛鐘開始老化戒指
    /// 它到達的快照。
    pub tick: Read<'a, Tick>,
    /// 階段 1.6：結果隊列 — 腳本端 `despawn` 推播
    /// 此處為「結果::EntityRemoved」；process_outcomes 在之後執行
    /// 腳本調度並處理entities().delete()+RemovedEntitiesQueue
    /// 均勻推。
    pub outcomes: Write<'a, Vec<crate::comp::Outcome>>,
}

impl<'a> AdapterCache<'a> {
    pub fn new(world: &'a World) -> Self {
        Self {
            entities: world.entities(),
            lazy: world.read_resource::<LazyUpdate>().into(),

            tattack: world.write_storage::<TAttack>(),
            pos: world.write_storage::<Pos>(),
            facing: world.write_storage::<Facing>(),
            cprop: world.write_storage::<CProperty>(),
            unit: world.write_storage::<Unit>(),
            hero: world.write_storage::<Hero>(),

            faction: world.read_storage::<Faction>(),
            creep: world.read_storage::<Creep>(),
            tower: world.read_storage::<Tower>(),
            is_building: world.read_storage::<IsBuilding>(),
            collision: world.read_storage::<CollisionRadius>(),
            tags: world.read_storage::<ScriptUnitTag>(),

            buffs: world.write_resource::<BuffStore>().into(),
            events: world.write_resource::<ScriptEventQueue>().into(),
            searcher: world.read_resource::<Searcher>().into(),
            blocked: world.read_resource::<BlockedRegions>().into(),
            time: world.read_resource::<Time>().into(),
            explosion_fx: world.write_resource::<ExplosionFxQueue>().into(),
            tower_fire_fx: world.write_resource::<TowerFireFxQueue>().into(),
            attack_phase_fx: world.write_resource::<AttackPhaseFxQueue>().into(),
            tick: world.read_resource::<Tick>().into(),
            outcomes: world.write_resource::<Vec<crate::comp::Outcome>>().into(),
        }
    }
}

/// 主機端適配器。為每個“run_script_dispatch”呼叫建立新的。
pub struct WorldAdapter<'a> {
    pub cache: AdapterCache<'a>,
    pub rng: Pcg64Mcg,
    /// 廣播給前端的 sender；spawn_projectile_ex、emit_explosion 會用到
    pub mqtx: Sender<OutboundMsg>,
}

impl<'a> WorldAdapter<'a> {
    pub fn new(world: &'a World, seed: u64, mqtx: Sender<OutboundMsg>) -> Self {
        Self {
            cache: AdapterCache::new(world),
            rng: Pcg64Mcg::seed_from_u64(seed),
            mqtx,
        }
    }

    #[inline]
    pub fn entity_to_handle(e: Entity) -> EntityHandle {
        EntityHandle {
            id: e.id(),
            gen: e.gen().id() as u32,
        }
    }

    #[inline]
    pub fn handle_to_entity(h: EntityHandle) -> Option<Entity> {
        if !h.is_valid() {
            return None;
        }
        let gen_i = h.gen as i32;
        if gen_i == 0 {
            return None;
        }
        Some(Entity::new(h.id, Generation::new(gen_i)))
    }

    fn angle_to_rad_f32(angle: Angle) -> f32 {
        angle.ticks() as f32 / omoba_sim::trig::TAU_TICKS as f32 * std::f32::consts::TAU
    }

    fn push_tower_fire_fx(&mut self, owner: Entity, dir_rad: f32) {
        if self.cache.tower.get(owner).is_none() {
            return;
        }
        let spawn_tick = self.cache.tick.0 as u32;
        let entity_id = owner.id();
        if self
            .cache
            .tower_fire_fx
            .pending
            .iter()
            .any(|fx| fx.entity_id == entity_id && fx.spawn_tick == spawn_tick)
        {
            return;
        }
        self.cache.tower_fire_fx.pending.push(TowerFireFx {
            entity_id,
            entity_gen: owner.gen().id() as u32,
            spawn_tick,
            dir_rad,
        });
    }
}

// ============================================================
// 第 1d/1e 階段最終清理：戰鬥/能力層現在已完全固定64/Vec2/角度。
// 剩餘的 f32 邊界是非戰鬥的：單位 i32 馬力（有意）、VFX 總線、
// 日誌格式化程式和 omb-mcp 查詢有線格式（第 5 階段+遷移）。
// ============================================================

impl<'a> GameWorld for WorldAdapter<'a> {
    // - - - - - - - - 問 - - - - - - - -

    fn get_pos(&self, e: EntityHandle) -> ROption<Vec2> {
        let Some(ent) = Self::handle_to_entity(e) else {
            return RNone;
        };
        match self.cache.pos.get(ent) {
            Some(p) => RSome(p.0),
            None => RNone,
        }
    }

    fn get_hp(&self, e: EntityHandle) -> ROption<Fixed64> {
        let Some(ent) = Self::handle_to_entity(e) else {
            return RNone;
        };
        // 偏好 CProperty（TD 模式的小兵/塔使用）；回落到單位。
        if let Some(p) = self.cache.cprop.get(ent) {
            // 階段 1c.3：CProperty.hp 現在固定64（階段 1c.2）- 直接回傳。
            return RSome(p.hp);
        }
        if let Some(u) = self.cache.unit.get(ent) {
            // 注意：Unit.current_hp 設計為 i32（整數遊戲值）；在此邊界轉換為固定64。
            return RSome(Fixed64::from_i32(u.current_hp));
        }
        RNone
    }

    fn get_max_hp(&self, e: EntityHandle) -> ROption<Fixed64> {
        let Some(ent) = Self::handle_to_entity(e) else {
            return RNone;
        };
        if let Some(p) = self.cache.cprop.get(ent) {
            // 階段 1c.3：CProperty.mhp 現在固定64（階段 1c.2）— 直接回傳。
            return RSome(p.mhp);
        }
        if let Some(u) = self.cache.unit.get(ent) {
            // 注意：Unit.max_hp 設計為 i32（整數遊戲值）；在此邊界轉換為固定64。
            return RSome(Fixed64::from_i32(u.max_hp));
        }
        RNone
    }

    fn is_alive(&self, e: EntityHandle) -> bool {
        let Some(ent) = Self::handle_to_entity(e) else {
            return false;
        };
        self.cache.entities.is_alive(ent)
    }

    fn faction_of(&self, _e: EntityHandle) -> ROption<RStr<'_>> {
        // 注意：傳回從元件儲存借用的 RStr
        // FFI 邊界很尷尬（儲存被鎖定在讀取後面）。
        // PoC 存根：返回 None 直到出現真正的需求；切換到
        // RString（擁有）如果/當腳本實際查詢它時。
        RNone
    }

    fn unit_id_of(&self, _e: EntityHandle) -> ROption<RStr<'_>> {
        // 與“faction_of”相同的終身關注。腳本通常不需要
        // 查詢他們自己的unit_id——他們靜態地知道它。
        RNone
    }

    fn query_enemies_in_range(
        &self,
        center: Vec2,
        radius: Fixed64,
        of: EntityHandle,
    ) -> RVec<EntityHandle> {
        let Some(of_ent) = Self::handle_to_entity(of) else {
            return RVec::new();
        };
        let my_team = match self.cache.faction.get(of_ent) {
            Some(f) => f.team_id,
            None => return RVec::new(),
        };

        let r2 = radius * radius;
        let mut out: RVec<EntityHandle> = RVec::new();

        for (ent, pos, fac) in (&self.cache.entities, &self.cache.pos, &self.cache.faction).join() {
            if fac.team_id == my_team {
                continue;
            }
            if pos.0.distance_squared(center) <= r2 {
                out.push(Self::entity_to_handle(ent));
            }
        }
        out
    }

    // ---------------- 變異 ----------------

    fn set_pos(&mut self, e: EntityHandle, p: Vec2) {
        let Some(ent) = Self::handle_to_entity(e) else {
            return;
        };
        if let Some(pos) = self.cache.pos.get_mut(ent) {
            pos.0 = p;
        }
    }

    fn advance_with_collision(&mut self, e: EntityHandle, target: Vec2, step: Fixed64) -> Vec2 {
        let Some(ent) = Self::handle_to_entity(e) else {
            return target;
        };
        let pos = match self.cache.pos.get(ent) {
            Some(p) => p.0,
            None => return target,
        };
        let radius = self
            .cache
            .collision
            .get(ent)
            .map(|r| r.0)
            .unwrap_or(Fixed64::from_i32(30));
        // 階段 1b.3：碰撞標記現在直接採用 Fix64 / Vec2；無邊界
        // 需要轉換（ABI 類型和 omoba_sim 類型一致）。
        let (new_pos, _reached) = crate::tick::hero_move_tick::advance_with_collision(
            pos,
            target,
            step,
            radius,
            &self.cache.searcher,
            &self.cache.collision,
            ent,
            &self.cache.blocked,
        );
        new_pos
    }

    fn deal_damage(
        &mut self,
        target: EntityHandle,
        amount: Fixed64,
        _kind: DamageKind,
        _source: ROption<EntityHandle>,
    ) {
        let Some(ent) = Self::handle_to_entity(target) else {
            return;
        };
        // 階段 1c.3：CProperty.hp 是 Fix64 — 直接算術。
        if let Some(p) = self.cache.cprop.get_mut(ent) {
            let new_hp = p.hp - amount;
            p.hp = if new_hp < Fixed64::ZERO {
                Fixed64::ZERO
            } else {
                new_hp
            };
            return;
        }
        if let Some(u) = self.cache.unit.get_mut(ent) {
            // 注意：Unit.current_hp 設計為 i32；在此邊界量化固定 64 點傷害。
            let amount_i = amount.to_f32_for_render() as i32;
            u.current_hp = (u.current_hp - amount_i).max(0);
        }
    }

    fn heal(&mut self, target: EntityHandle, amount: Fixed64) {
        let Some(ent) = Self::handle_to_entity(target) else {
            return;
        };
        // 階段 1c.3：CProperty.hp / mhp 現在是 Fix64 — 直接算術。
        if let Some(p) = self.cache.cprop.get_mut(ent) {
            let new_hp = p.hp + amount;
            p.hp = if new_hp > p.mhp { p.mhp } else { new_hp };
            return;
        }
        if let Some(u) = self.cache.unit.get_mut(ent) {
            // 注意：Unit.current_hp 設計為 i32；量化固定 64 在此邊界處治癒。
            let amount_i = amount.to_f32_for_render() as i32;
            u.current_hp = (u.current_hp + amount_i).min(u.max_hp);
        }
    }

    fn add_buff(&mut self, target: EntityHandle, buff_id: RStr<'_>, duration: Fixed64) {
        let Some(ent) = Self::handle_to_entity(target) else {
            return;
        };
        let id_owned = buff_id.as_str().to_string();
        // 階段 1c.3：BuffStore::add 現在採用 Fix64 — 直接呼叫。
        self.cache
            .buffs
            .add(ent, &id_owned, duration, serde_json::Value::Null);
        self.cache.events.push(ScriptEvent::ModifierAdded {
            e: ent,
            modifier_id: id_owned,
        });
    }

    fn remove_buff(&mut self, target: EntityHandle, buff_id: RStr<'_>) {
        let Some(ent) = Self::handle_to_entity(target) else {
            return;
        };
        let id_owned = buff_id.as_str().to_string();
        self.cache.buffs.remove(ent, &id_owned);
        self.cache.events.push(ScriptEvent::ModifierRemoved {
            e: ent,
            modifier_id: id_owned,
        });
    }

    fn has_buff(&self, target: EntityHandle, buff_id: RStr<'_>) -> bool {
        let Some(ent) = Self::handle_to_entity(target) else {
            return false;
        };
        self.cache.buffs.has(ent, buff_id.as_str())
    }

    fn add_stat_buff(
        &mut self,
        target: EntityHandle,
        buff_id: RStr<'_>,
        duration: Fixed64,
        modifiers_json: RStr<'_>,
    ) {
        let Some(ent) = Self::handle_to_entity(target) else {
            return;
        };
        let payload: serde_json::Value =
            serde_json::from_str(modifiers_json.as_str()).unwrap_or(serde_json::Value::Null);
        let id_owned = buff_id.as_str().to_string();
        // 階段 1c.3：BuffStore::add 現在採用 Fix64 — 直接呼叫。
        self.cache.buffs.add(ent, &id_owned, duration, payload);
        self.cache.events.push(ScriptEvent::ModifierAdded {
            e: ent,
            modifier_id: id_owned,
        });
    }

    fn spawn_summoned_unit(
        &mut self,
        pos: Vec2,
        unit_type: RStr<'_>,
        owner: EntityHandle,
        duration: Fixed64,
    ) -> EntityHandle {
        let Some(owner_ent) = Self::handle_to_entity(owner) else {
            return EntityHandle::INVALID;
        };
        let unit_type_str = unit_type.as_str();

        // 繼承 owner 的 faction（陣營 + team_id）
        let faction = self
            .cache
            .faction
            .get(owner_ent)
            .cloned()
            .unwrap_or_else(|| Faction::new(FactionType::Player, 0));

        let pos_x_f = pos.x.to_f32_for_render();
        let pos_y_f = pos.y.to_f32_for_render();
        let duration_f = duration.to_f32_for_render();
        let Some(unit) =
            Unit::create_summon_unit(unit_type_str, (pos_x_f, pos_y_f), faction.team_id)
        else {
            return EntityHandle::INVALID;
        };

        // Unit 內嵌數值映射出 CProperty + TAttack（hero/creep 的基礎屬性 component）
        // 階段 1c.3：單位的 move_speed/attack_speed/attack_range 已經固定64
        // (Phase 1c.2). hp / max_hp 仍為 i32。
        let one_hundredth = Fixed64::from_raw(10); // 0.01 in Q22.10 (10/1024 ≈ 0.00977)
        let attack_speed_min = if unit.attack_speed < one_hundredth {
            one_hundredth
        } else {
            unit.attack_speed
        };
        let cprop = CProperty {
            hp: Fixed64::from_i32(unit.current_hp),
            mhp: Fixed64::from_i32(unit.max_hp),
            msd: unit.move_speed,
            def_physic: unit.base_armor,
            def_magic: unit.magic_resistance,
        };
        let tatk = TAttack {
            atk_physic: Vf32::new(Fixed64::from_i32(unit.base_damage)),
            asd: Vf32::new(Fixed64::ONE / attack_speed_min),
            range: Vf32::new(unit.attack_range),
            asd_count: Fixed64::ZERO,
            bullet_speed: Fixed64::from_i32(1000),
            attack_seq: 0,
            attack_phase: AttackSequencePhase::Idle,
        };

        let summon_time = self.cache.time.0 as f32;
        let summoned = SummonedUnit::new(
            owner_ent,
            if duration_f > 0.0 {
                Some(duration_f)
            } else {
                None
            },
            summon_time,
        );

        // LazyUpdate：entity id 立刻分配，components 在下次 maintain (core.rs:364) 真正附上。
        // 本 frame 後續 system 已跑完，不會看到這個半成品。
        let e = self
            .cache
            .lazy
            .create_entity(&self.cache.entities)
            .with(Pos(pos))
            .with(Vel(omoba_sim::Vec2::ZERO))
            .with(unit.clone())
            .with(faction)
            .with(cprop)
            .with(tatk)
            .with(Facing(omoba_sim::Angle::ZERO))
            .with(crate::comp::FacingBroadcast(None))
            // π rad/s ≈ 3.14159 → 固定 64 raw = round(π * 1024) = 3217
            .with(TurnSpeed(omoba_sim::Fixed64::from_raw(3217)))
            .with(CollisionRadius(omoba_sim::Fixed64::from_i32(30)))
            .with(summoned)
            // 綁 ScriptUnitTag 讓 dispatch tick 呼叫 UnitScript::on_tick 驅動 AI
            .with(crate::scripting::ScriptUnitTag {
                unit_id: unit_type_str.to_string(),
            })
            .build();

        Self::entity_to_handle(e)
    }

    fn spawn_projectile_ex(&mut self, spec: ProjectileSpec) -> EntityHandle {
        use omoba_sim::Fixed64;
        let Some(owner_ent) = Self::handle_to_entity(spec.owner) else {
            return EntityHandle::INVALID;
        };
        let from = spec.from;

        // 依 PathSpec 算 tpos + target option（end_pos 不再需要 — wire-emit 已砍）
        let (target_opt, tpos) = match spec.path {
            PathSpec::Homing { target } => {
                let Some(target_ent) = Self::handle_to_entity(target) else {
                    return EntityHandle::INVALID;
                };
                let tp = self.cache.pos.get(target_ent).map(|p| p.0).unwrap_or(from);
                (Some(target_ent), tp)
            }
            PathSpec::Straight { end_pos } => (None, end_pos),
        };

        // 飛行時間數學透過 f32 計算 .max(0.01) 箝位行為
        // （Fixed64 沒有 sqrt）。 Projectile 中的 Sim 端狀態保持固定 64。
        let speed_f = spec.speed.to_f32_for_render();
        let initial_dist = {
            let dx = (tpos.x - from.x).to_f32_for_render();
            let dy = (tpos.y - from.y).to_f32_for_render();
            (dx * dx + dy * dy).sqrt()
        };
        let flight_time_s: f32 = if speed_f > 0.0 {
            (initial_dist / speed_f).max(0.01)
        } else {
            0.01
        };
        let safety: Fixed64 = Fixed64::from_raw(
            ((flight_time_s * 3.0 + 1.5) * omoba_sim::fixed::SCALE as f32) as i64,
        );
        let fire_angle = omoba_sim::trig::atan2(tpos.y - from.y, tpos.x - from.x);
        if let Some(facing) = self.cache.facing.get_mut(owner_ent) {
            facing.0 = fire_angle;
        }
        self.push_tower_fire_fx(owner_ent, Self::angle_to_rad_f32(fire_angle));

        // LazyUpdate spawn — 同 frame 後續系統已跑完，maintain 在 core.rs tick 結尾。
        let e = self
            .cache
            .lazy
            .create_entity(&self.cache.entities)
            .with(Pos(from))
            .with(Projectile {
                time_left: safety,
                owner: owner_ent,
                tpos,
                target: target_opt,
                radius: spec.splash_radius,
                msd: spec.speed,
                damage_phys: spec.damage,
                damage_magi: Fixed64::ZERO,
                damage_real: Fixed64::ZERO,
                slow_factor: spec.slow_factor,
                slow_duration: spec.slow_duration,
                hit_radius: spec.hit_radius,
                stun_duration: spec.stun_duration,
            })
            .build();

        Self::entity_to_handle(e)
    }

    fn emit_explosion(&mut self, pos: Vec2, radius: Fixed64, duration: Fixed64) {
        // 階段 4.2：是 `mqtx.try_send(make_game_explosion_script(...))`；
        // 現在透過推入來完成鎖定快照管道
        // `ExplosionFxQueue`。 sim_runner 提取器會耗盡每個
        // 勾選，渲染線程會產生帶有 omfx-wall-clock 的圓環
        // 生命週期。 ExplosionFxQueue 是一種無狀態資源（sim 從不
        // 讀回來），所以決定論不受影響。
        let duration_ms =
            (duration.to_f32_for_render() * 1000.0).clamp(0.0, u32::MAX as f32) as u32;
        let current_tick = self.cache.tick.0 as u32;
        self.cache.explosion_fx.pending.push(ExplosionFx {
            pos_x: pos.x.to_f32_for_render(),
            pos_y: pos.y.to_f32_for_render(),
            radius: radius.to_f32_for_render(),
            duration_ms,
            spawn_tick: current_tick,
        });
    }

    fn despawn(&mut self, e: EntityHandle) {
        let Some(ent) = Self::handle_to_entity(e) else {
            return;
        };
        // 走 Outcome::EntityRemoved 通道 — script-side 跟 omb-side 統一
        // entry。process_outcomes 跑時處理 entities().delete() +
        // 刪除了Entities隊列推送。
        self.cache
            .outcomes
            .push(crate::comp::Outcome::EntityRemoved { entity: ent });
    }

    // ---------------- 塔 / 單位屬性 ----------------

    fn get_tower_range(&self, e: EntityHandle) -> Fixed64 {
        let Some(ent) = Self::handle_to_entity(e) else {
            return Fixed64::ZERO;
        };
        // 階段 1c.3：TAtack.range.v 為 Fix64 — 直接回傳。
        self.cache
            .tattack
            .get(ent)
            .map(|t| t.range.v)
            .unwrap_or(Fixed64::ZERO)
    }

    fn get_tower_atk(&self, e: EntityHandle) -> Fixed64 {
        let Some(ent) = Self::handle_to_entity(e) else {
            return Fixed64::ZERO;
        };
        // 階段 1c.3：TAtack.atk_physical.v 是 Fix64 — 直接回傳。
        self.cache
            .tattack
            .get(ent)
            .map(|t| t.atk_physic.v)
            .unwrap_or(Fixed64::ZERO)
    }

    fn get_asd_interval(&self, e: EntityHandle) -> Fixed64 {
        let Some(ent) = Self::handle_to_entity(e) else {
            return Fixed64::ZERO;
        };
        // 階段 1c.3：TAtack.asd.v 是 Fix64 — 直接回傳。
        self.cache
            .tattack
            .get(ent)
            .map(|t| t.asd.v)
            .unwrap_or(Fixed64::ZERO)
    }

    fn get_asd_count(&self, e: EntityHandle) -> Fixed64 {
        let Some(ent) = Self::handle_to_entity(e) else {
            return Fixed64::ZERO;
        };
        // 階段 1c.3：TAtack.asd_count 為 Fix64 — 直接回傳。
        self.cache
            .tattack
            .get(ent)
            .map(|t| t.asd_count)
            .unwrap_or(Fixed64::ZERO)
    }

    fn set_asd_count(&mut self, e: EntityHandle, v: Fixed64) {
        let Some(ent) = Self::handle_to_entity(e) else {
            return;
        };
        if let Some(t) = self.cache.tattack.get_mut(ent) {
            // 階段 1c.3：TAtack.asd_count 是固定 64 — 直接寫入。
            t.asd_count = v;
        }
    }

    fn set_tower_atk(&mut self, e: EntityHandle, v: Fixed64) {
        let Some(ent) = Self::handle_to_entity(e) else {
            return;
        };
        if let Some(t) = self.cache.tattack.get_mut(ent) {
            // 階段 1c.3：Vf32 保留 Fix64 — 直接寫入。
            t.atk_physic.bv = v;
            t.atk_physic.v = v;
        }
    }

    fn set_tower_range(&mut self, e: EntityHandle, v: Fixed64) {
        let Some(ent) = Self::handle_to_entity(e) else {
            return;
        };
        if let Some(t) = self.cache.tattack.get_mut(ent) {
            // 階段 1c.3：Vf32 保留 Fix64 — 直接寫入。
            t.range.bv = v;
            t.range.v = v;
        }
    }

    fn set_asd_interval(&mut self, e: EntityHandle, v: Fixed64) {
        let Some(ent) = Self::handle_to_entity(e) else {
            return;
        };
        if let Some(t) = self.cache.tattack.get_mut(ent) {
            // 階段 1c.3：Vf32 保留 Fix64 — 直接寫入。
            t.asd.bv = v;
            t.asd.v = v;
        }
    }

    fn set_facing(&mut self, e: EntityHandle, angle: Angle) {
        let Some(ent) = Self::handle_to_entity(e) else {
            return;
        };
        if let Some(f) = self.cache.facing.get_mut(ent) {
            f.0 = angle;
        }
    }

    fn get_facing(&self, e: EntityHandle) -> Angle {
        let Some(ent) = Self::handle_to_entity(e) else {
            return Angle::ZERO;
        };
        self.cache
            .facing
            .get(ent)
            .map(|f| f.0)
            .unwrap_or(Angle::ZERO)
    }

    fn query_nearest_enemy(
        &self,
        center: Vec2,
        radius: Fixed64,
        of: EntityHandle,
    ) -> ROption<EntityHandle> {
        let Some(of_ent) = Self::handle_to_entity(of) else {
            return RNone;
        };
        let my_team = match self.cache.faction.get(of_ent) {
            Some(f) => f.team_id,
            None => return RNone,
        };
        let r2 = radius * radius;
        let mut best: Option<(Entity, Fixed64)> = None;
        // 只選 creep（氣球）為目標；不要誤選隊友/其他塔
        for (ent, pos, fac, _c) in (
            &self.cache.entities,
            &self.cache.pos,
            &self.cache.faction,
            &self.cache.creep,
        )
            .join()
        {
            if fac.team_id == my_team {
                continue;
            }
            let d2 = pos.0.distance_squared(center);
            if d2 <= r2 {
                if best.map(|(_, b)| d2 < b).unwrap_or(true) {
                    best = Some((ent, d2));
                }
            }
        }
        match best {
            Some((ent, _)) => RSome(Self::entity_to_handle(ent)),
            None => RNone,
        }
    }

    // ---------------- 副作用 ----------------

    fn play_vfx(&mut self, id: RStr<'_>, at: Vec2) {
        // 注意：log 使用 f32 邊界 — Fix64 沒有顯示。
        log::debug!(
            "[scripting] play_vfx id={} at=({},{})",
            id.as_str(),
            at.x.to_f32_for_render(),
            at.y.to_f32_for_render()
        );
    }

    fn play_sfx(&mut self, id: RStr<'_>, at: Vec2) {
        // 注意：log 使用 f32 邊界 — Fix64 沒有顯示。
        log::debug!(
            "[scripting] play_sfx id={} at=({},{})",
            id.as_str(),
            at.x.to_f32_for_render(),
            at.y.to_f32_for_render()
        );
    }

    // ---------------- RNG ----------------

    fn rand_unit(&mut self) -> Fixed64 {
        // 階段 1de.2：確定性 Pcg64Mcg → Fix64 [0,1)，無 f32 量化。
        // 匹配 omoba_sim::SimRng::gen_fixed64_unit （相同的 Pcg 變體，相同的模 1024）。
        // 這裡的 Pcg64Mcg 透過 `WorldAdapter::new(world, Seed, ..)` 在每次調度時播種，
        // 因此，只要調度種子存在，確定性就會在重播中保留。
        let raw = (self.rng.next_u32() % 1024) as i64;
        Fixed64::from_raw(raw)
    }

    // ---------------- Log ----------------

    fn log_info(&self, msg: RStr<'_>) {
        log::debug!("[script] {}", msg.as_str());
    }
    fn log_warn(&self, msg: RStr<'_>) {
        log::warn!("[script] {}", msg.as_str());
    }
    fn log_error(&self, msg: RStr<'_>) {
        log::error!("[script] {}", msg.as_str());
    }

    // ---------------- Dota 2 modifier 風格聚合 ----------------

    fn sum_stat(&self, e: EntityHandle, stat_key: StatKey) -> Fixed64 {
        let Some(ent) = Self::handle_to_entity(e) else {
            return Fixed64::ZERO;
        };
        // 階段 1c.3：BuffStore::sum_add 現在回到 Fix64 — 直接回傳。
        self.cache.buffs.sum_add(ent, stat_key)
    }

    fn product_stat(&self, e: EntityHandle, stat_key: StatKey) -> Fixed64 {
        let Some(ent) = Self::handle_to_entity(e) else {
            return Fixed64::ONE;
        };
        // 階段 1c.3：BuffStore::product_mult 現在回到 Fix64 — 直接回傳。
        self.cache.buffs.product_mult(ent, stat_key)
    }

    fn get_final_move_speed(&self, e: EntityHandle) -> Fixed64 {
        let Some(ent) = Self::handle_to_entity(e) else {
            return Fixed64::ZERO;
        };
        // 階段 1c.3：CProperty.msd 是 Fix64（階段 1c.2）— 直接讀取。
        let base = self
            .cache
            .cprop
            .get(ent)
            .map(|p| p.msd)
            .unwrap_or(Fixed64::ZERO);
        let is_b = self.cache.is_building.get(ent).is_some();
        // 階段 1c.3：UnitStats::final_move_speed 現在回傳 Fix64 — 直接回傳。
        UnitStats::from_refs(&*self.cache.buffs, is_b).final_move_speed(base, ent)
    }

    fn get_final_atk(&self, e: EntityHandle) -> Fixed64 {
        let Some(ent) = Self::handle_to_entity(e) else {
            return Fixed64::ZERO;
        };
        // TAttack.atk_physical.v 是Fixed64（階段1c.2 — Vf32 內部保留Fixed64）。
        let base = self
            .cache
            .tattack
            .get(ent)
            .map(|t| t.atk_physic.v)
            .unwrap_or(Fixed64::ZERO);
        let is_b = self.cache.is_building.get(ent).is_some();
        // 階段 1c.3：UnitStats::final_atk 現在回傳 Fix64 — 直接回傳。
        UnitStats::from_refs(&*self.cache.buffs, is_b).final_atk(base, ent)
    }

    fn get_tower_upgrade(&self, e: EntityHandle, path: u8) -> u8 {
        let Some(ent) = Self::handle_to_entity(e) else {
            return 0;
        };
        self.cache
            .tower
            .get(ent)
            .and_then(|t| t.upgrade_levels.get(path as usize))
            .copied()
            .unwrap_or(0)
    }

    fn has_tower_flag(&self, e: EntityHandle, flag: RStr<'_>) -> bool {
        let Some(ent) = Self::handle_to_entity(e) else {
            return false;
        };
        self.cache
            .tower
            .get(ent)
            .map(|t| t.upgrade_flags.iter().any(|f| f == flag.as_str()))
            .unwrap_or(false)
    }

    fn apply_tower_permanent_buff(
        &mut self,
        e: EntityHandle,
        buff_id: RStr<'_>,
        modifiers_json: RStr<'_>,
    ) {
        // 注意：Fixed64::from_raw(i64::MAX) 是「永久 buff」哨兵 — 夠大，buff_tick 不會
        // 在任何合理的會話中減少到零。可以在第 2 階段中用顯式的 None/permanent 標誌替換。
        self.add_stat_buff(e, buff_id, Fixed64::from_raw(i64::MAX), modifiers_json);
    }

    fn get_final_attack_range(&self, e: EntityHandle) -> Fixed64 {
        let Some(ent) = Self::handle_to_entity(e) else {
            return Fixed64::ZERO;
        };
        let base = self
            .cache
            .tattack
            .get(ent)
            .map(|t| t.range.v)
            .unwrap_or(Fixed64::ZERO);
        let is_b = self.cache.is_building.get(ent).is_some();
        // 階段 1c.3：UnitStats::final_attack_range 現在回傳 Fix64 — 直接回傳。
        UnitStats::from_refs(&*self.cache.buffs, is_b).final_attack_range(base, ent)
    }

    fn get_buff_remaining(&self, e: EntityHandle, buff_id: RStr<'_>) -> Fixed64 {
        let Some(ent) = Self::handle_to_entity(e) else {
            return Fixed64::ZERO;
        };
        // 階段 1c.3：BuffEntry.remaining 現在是 Fix64 — 直接回傳。
        self.cache
            .buffs
            .get(ent, buff_id.as_str())
            .map(|b| b.remaining)
            .unwrap_or(Fixed64::ZERO)
    }

    fn current_mana(&self, e: EntityHandle) -> Fixed64 {
        // 沒有 current_mana component — 目前回 max（視為永遠滿）。
        // 如果之後加 `ManaPool` component，這裡要改成讀 current。
        let Some(ent) = Self::handle_to_entity(e) else {
            return Fixed64::ZERO;
        };
        // 階段 1c.3：Hero.get_max_mana 現在回傳 Fix64 — 直接回傳。
        self.cache
            .hero
            .get(ent)
            .map(|h| h.get_max_mana())
            .unwrap_or(Fixed64::ZERO)
    }

    fn spend_mana(&mut self, e: EntityHandle, amount: Fixed64, ability_id: RStr<'_>) -> bool {
        let Some(ent) = Self::handle_to_entity(e) else {
            return false;
        };
        // 目前沒有 mana storage，永遠視為成功；push 事件讓腳本 hook。
        self.cache.events.push(ScriptEvent::SpentMana {
            caster: ent,
            // 階段 1c.3：ScriptEvent::SpentMana.cost 現在固定64 — 直接推播。
            cost: amount,
            ability_id: ability_id.as_str().to_string(),
        });
        true
    }

    fn restore_mana(&mut self, e: EntityHandle, amount: Fixed64) {
        let Some(ent) = Self::handle_to_entity(e) else {
            return;
        };
        // 階段 1c.3：ScriptEvent::ManaGained.amount 現在固定64 — 直接推送。
        self.cache
            .events
            .push(ScriptEvent::ManaGained { e: ent, amount });
    }

    fn trigger_state_changed(&mut self, e: EntityHandle, state_id: RStr<'_>, active: bool) {
        let Some(ent) = Self::handle_to_entity(e) else {
            return;
        };
        self.cache.events.push(ScriptEvent::StateChanged {
            e: ent,
            state_id: state_id.as_str().to_string(),
            active,
        });
    }

    // ---------------- Dota 2 property 完整查詢 ----------------

    fn get_final_armor(&self, e: EntityHandle) -> Fixed64 {
        let Some(ent) = Self::handle_to_entity(e) else {
            return Fixed64::ZERO;
        };
        let is_bldg = self.cache.is_building.get(ent).is_some();
        // 階段 1c.3：CProperty.def_physical 是 Fix64（階段 1c.2）— 直接讀取。
        let base = self
            .cache
            .cprop
            .get(ent)
            .map(|c| c.def_physic)
            .unwrap_or(Fixed64::ZERO);
        // 階段 1c.3：UnitStats::final_armor 現在回傳 Fix64 — 直接回傳。
        UnitStats::from_refs(&*self.cache.buffs, is_bldg).final_armor(base, ent)
    }

    fn get_final_magic_resist(&self, e: EntityHandle) -> Fixed64 {
        let Some(ent) = Self::handle_to_entity(e) else {
            return Fixed64::ZERO;
        };
        let is_bldg = self.cache.is_building.get(ent).is_some();
        // 階段 1c.3：CProperty.def_magic 是 Fix64（階段 1c.2）— 直接讀取。
        let base = self
            .cache
            .cprop
            .get(ent)
            .map(|c| c.def_magic)
            .unwrap_or(Fixed64::ZERO);
        // 階段 1c.3：UnitStats::final_magic_resist 現在回傳 Fix64 — 直接回傳。
        UnitStats::from_refs(&*self.cache.buffs, is_bldg).final_magic_resist(base, ent)
    }

    fn get_evasion_chance(&self, e: EntityHandle) -> Fixed64 {
        let Some(ent) = Self::handle_to_entity(e) else {
            return Fixed64::ZERO;
        };
        let is_bldg = self.cache.is_building.get(ent).is_some();
        // 階段 1c.3：UnitStats::evasion_chance 現在回傳 Fix64 — 直接回傳。
        UnitStats::from_refs(&*self.cache.buffs, is_bldg).evasion_chance(ent)
    }

    fn get_miss_chance(&self, e: EntityHandle) -> Fixed64 {
        let Some(ent) = Self::handle_to_entity(e) else {
            return Fixed64::ZERO;
        };
        let is_bldg = self.cache.is_building.get(ent).is_some();
        // 階段 1c.3：UnitStats::miss_chance 現在回傳 Fix64 — 直接回傳。
        UnitStats::from_refs(&*self.cache.buffs, is_bldg).miss_chance(ent)
    }

    fn get_crit_chance(&self, e: EntityHandle) -> Fixed64 {
        let Some(ent) = Self::handle_to_entity(e) else {
            return Fixed64::ZERO;
        };
        let is_bldg = self.cache.is_building.get(ent).is_some();
        // 階段 1c.3：UnitStats::crit 現在回傳 (Fixed64, Fix64) — 直接回傳。
        UnitStats::from_refs(&*self.cache.buffs, is_bldg)
            .crit(ent)
            .0
    }

    fn get_crit_multiplier(&self, e: EntityHandle) -> Fixed64 {
        let Some(ent) = Self::handle_to_entity(e) else {
            return Fixed64::ONE;
        };
        let is_bldg = self.cache.is_building.get(ent).is_some();
        // 階段 1c.3：UnitStats::crit 現在回傳 (Fixed64, Fix64) — 直接回傳。
        UnitStats::from_refs(&*self.cache.buffs, is_bldg)
            .crit(ent)
            .1
    }

    fn get_cooldown_mult(&self, e: EntityHandle) -> Fixed64 {
        let Some(ent) = Self::handle_to_entity(e) else {
            return Fixed64::ONE;
        };
        let is_bldg = self.cache.is_building.get(ent).is_some();
        // 階段 1c.3：UnitStats::cooldown_mult 現在回傳 Fix64 — 直接回傳。
        UnitStats::from_refs(&*self.cache.buffs, is_bldg).cooldown_mult(ent)
    }

    fn is_building(&self, e: EntityHandle) -> bool {
        let Some(ent) = Self::handle_to_entity(e) else {
            return false;
        };
        self.cache.is_building.get(ent).is_some()
    }

    fn get_max_hp_bonus(&self, e: EntityHandle) -> Fixed64 {
        let Some(ent) = Self::handle_to_entity(e) else {
            return Fixed64::ZERO;
        };
        let is_bldg = self.cache.is_building.get(ent).is_some();
        // 階段 1c.3：UnitStats::max_hp_bonus 現在回傳 Fix64 — 直接回傳。
        UnitStats::from_refs(&*self.cache.buffs, is_bldg).max_hp_bonus(ent)
    }

    fn get_hp_regen(&self, e: EntityHandle) -> Fixed64 {
        let Some(ent) = Self::handle_to_entity(e) else {
            return Fixed64::ZERO;
        };
        let is_bldg = self.cache.is_building.get(ent).is_some();
        // 階段 1c.3：UnitStats::hp_regen 現在回傳 Fix64 — 直接回傳。
        UnitStats::from_refs(&*self.cache.buffs, is_bldg).hp_regen(Fixed64::ZERO, ent)
    }

    fn get_stat_bonus(&self, e: EntityHandle, key: StatKey) -> Fixed64 {
        let Some(ent) = Self::handle_to_entity(e) else {
            return Fixed64::ZERO;
        };
        // 階段 1c.3：BuffStore::sum_add 現在回到 Fix64 — 直接回傳。
        self.cache.buffs.sum_add(ent, key)
    }

    fn deal_damage_splash(
        &mut self,
        at: Vec2,
        radius: Fixed64,
        damage: Fixed64,
        kind: DamageKind,
        source: ROption<EntityHandle>,
    ) {
        // 以 source 為 "of" 參考；若 source 無則以 at 對 "空" 做查詢（query_enemies_in_range 需要一個 of）。
        let of = match source {
            ROption::RSome(h) => h,
            ROption::RNone => return,
        };
        if let Some(source_ent) = Self::handle_to_entity(of) {
            let dir = self
                .cache
                .facing
                .get(source_ent)
                .map(|f| Self::angle_to_rad_f32(f.0))
                .unwrap_or(0.0);
            self.push_tower_fire_fx(source_ent, dir);
        }
        let targets = self.query_enemies_in_range(at, radius, of);
        for th in targets.iter() {
            self.deal_damage(*th, damage, kind, source);
        }
    }

    fn emit_attack_phase_fx(
        &mut self,
        entity: EntityHandle,
        target: Target,
        windup_ms: u32,
        backswing_ms: u32,
    ) {
        let Some(ent) = Self::handle_to_entity(entity) else {
            return;
        };
        let Some(pos) = self.cache.pos.get(ent).map(|p| p.0) else {
            return;
        };
        let (target_entity_id, target_pos) = match target {
            Target::Entity(handle) => {
                let target_ent = Self::handle_to_entity(handle);
                let tpos = target_ent.and_then(|te| self.cache.pos.get(te).map(|p| p.0));
                (target_ent.map(|te| te.id()), tpos)
            }
            Target::Point(point) => (None, Some(point)),
            Target::None => (None, None),
        };
        let dir_angle = if let Some(tpos) = target_pos {
            omoba_sim::trig::atan2(tpos.y - pos.y, tpos.x - pos.x)
        } else {
            self.cache
                .facing
                .get(ent)
                .map(|f| f.0)
                .unwrap_or(Angle::ZERO)
        };
        if let Some(facing) = self.cache.facing.get_mut(ent) {
            facing.0 = dir_angle;
        }
        let q = &mut self.cache.attack_phase_fx;
        let attack_seq = q.next_seq;
        q.next_seq = q.next_seq.wrapping_add(1);
        q.pending.push(AttackPhaseFx {
            entity_id: ent.id(),
            entity_gen: ent.gen().id() as u32,
            spawn_tick: self.cache.tick.0 as u32,
            attack_seq,
            is_critical: false,
            windup_ms,
            impact_at_ms: windup_ms,
            backswing_ms,
            dir_rad: Self::angle_to_rad_f32(dir_angle),
            target_entity_id,
            target_pos_x: target_pos.map(|p| p.x.to_f32_for_render()),
            target_pos_y: target_pos.map(|p| p.y.to_f32_for_render()),
        });
    }
}
