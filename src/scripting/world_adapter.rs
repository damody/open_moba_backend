//! `WorldAdapter` — implements `omb_script_abi::GameWorld` over `&mut specs::World`.
//!
//! Only alive during the serial script-dispatch stage (E1), so it holds
//! `&mut World` exclusively; no locks needed.
//!
//! Adds components/methods here when scripts need more surface area. Keep the
//! surface as small as the PoC-1 (and subsequent PoCs) actually needs.

use abi_stable::std_types::{RNone, ROption, RSome, RStr, RVec};
use crossbeam_channel::Sender;
use omb_script_abi::{
    stat_keys::StatKey,
    types::{Angle, DamageKind, EntityHandle, Fixed32, PathSpec, ProjectileSpec, Vec2},
    world::GameWorld,
};
use rand::{Rng, SeedableRng};
use rand_pcg::Pcg64Mcg;
use serde_json::json;
use specs::{
    Builder, Entities, Entity, Join, LazyUpdate, Read, ReadStorage, World, WorldExt, Write,
    WriteStorage,
};
use specs::world::Generation;

use crate::ability_runtime::{BuffStore, UnitStats};
use crate::comp::*;
use crate::scripting::event::{ScriptEvent, ScriptEventQueue};
use crate::scripting::tag::ScriptUnitTag;
use crate::transport::OutboundMsg;

// P2 typed payload helpers (local copies to avoid cross-module pub leakage).
// P7: `damage` carries pre-declared single-target damage for latency hiding.
#[inline]
fn make_projectile_create_script(
    id: u32, target_id: u32,
    start_x: f32, start_y: f32, end_x: f32, end_y: f32,
    move_speed: f32, flight_time_ms: u64,
    directional: bool, splash_radius: f32, hit_radius: f32, kind_id: u16,
    damage: f32,
) -> OutboundMsg {
    // Backward-compat JSON payload: still emit `kind` as a string so legacy
    // non-kcp transports and omfx's debug inspector see the familiar tag.
    let kind_str = omoba_template_ids::projectile_id_str(
        omoba_template_ids::ProjectileKindId(kind_id),
    );
    #[cfg(feature = "kcp")]
    {
        use crate::state::resource_management::proto_build;
        use crate::transport::TypedOutbound;
        OutboundMsg::new_typed_at(
            "td/all/res", "projectile", "C",
            TypedOutbound::ProjectileCreate(proto_build::projectile_create(
                id, target_id, start_x, start_y, end_x, end_y,
                flight_time_ms, directional, splash_radius, hit_radius, kind_id,
                damage,
            )),
            json!({
                "id": id, "target_id": target_id,
                "start_pos": { "x": start_x, "y": start_y },
                "end_pos":   { "x": end_x,   "y": end_y },
                "move_speed": move_speed, "flight_time_ms": flight_time_ms,
                "kind": kind_str, "directional": directional,
                "hit_radius": hit_radius, "splash_radius": splash_radius,
                "damage": damage,
            }),
            start_x, start_y,
        )
    }
    #[cfg(not(feature = "kcp"))]
    {
        OutboundMsg::new_s_at(
            "td/all/res", "projectile", "C",
            json!({
                "id": id, "target_id": target_id,
                "start_pos": { "x": start_x, "y": start_y },
                "end_pos":   { "x": end_x,   "y": end_y },
                "move_speed": move_speed, "flight_time_ms": flight_time_ms,
                "kind": kind_str, "directional": directional,
                "hit_radius": hit_radius, "splash_radius": splash_radius,
                "damage": damage,
            }),
            start_x, start_y,
        )
    }
}

#[inline]
fn make_game_explosion_script(x: f32, y: f32, radius: f32, duration: f32) -> OutboundMsg {
    #[cfg(feature = "kcp")]
    {
        use crate::state::resource_management::proto_build;
        use crate::transport::TypedOutbound;
        OutboundMsg::new_typed_at(
            "td/all/res", "game", "explosion",
            TypedOutbound::GameExplosion(proto_build::game_explosion(x, y, radius, duration)),
            json!({ "x": x, "y": y, "radius": radius, "duration": duration }),
            x, y,
        )
    }
    #[cfg(not(feature = "kcp"))]
    {
        OutboundMsg::new_s_at("td/all/res", "game", "explosion",
            json!({ "x": x, "y": y, "radius": radius, "duration": duration }), x, y)
    }
}

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

    // Read-only storages
    pub faction: ReadStorage<'a, Faction>,
    pub creep: ReadStorage<'a, Creep>,
    pub tower: ReadStorage<'a, Tower>,
    pub is_building: ReadStorage<'a, IsBuilding>,
    pub collision: ReadStorage<'a, CollisionRadius>,
    pub tags: ReadStorage<'a, ScriptUnitTag>,

    // Resources
    pub buffs: Write<'a, BuffStore>,
    pub events: Write<'a, ScriptEventQueue>,
    pub searcher: Read<'a, Searcher>,
    pub blocked: Read<'a, BlockedRegions>,
    pub time: Read<'a, Time>,
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
        }
    }
}

/// Host-side adapter. Created fresh for each `run_script_dispatch` call.
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
}

// ============================================================
// Phase 1c.5 cleanup: battle / ability layer is fully Fixed32 / Vec2 / Angle now.
// Remaining f32 boundaries are non-battle: Unit i32 hp, Projectile struct, VFX
// bus, log formatters, RNG. Each is tagged `// TODO Phase 1[d]` for next phase.
// ============================================================

impl<'a> GameWorld for WorldAdapter<'a> {
    // ---------------- Query ----------------

    fn get_pos(&self, e: EntityHandle) -> ROption<Vec2> {
        let Some(ent) = Self::handle_to_entity(e) else { return RNone };
        match self.cache.pos.get(ent) {
            Some(p) => RSome(p.0),
            None => RNone,
        }
    }

    fn get_hp(&self, e: EntityHandle) -> ROption<Fixed32> {
        let Some(ent) = Self::handle_to_entity(e) else { return RNone };
        // Prefer CProperty (used by creeps/towers in TD mode); fall back to Unit.
        if let Some(p) = self.cache.cprop.get(ent) {
            // Phase 1c.3: CProperty.hp now Fixed32 (Phase 1c.2) — direct return.
            return RSome(p.hp);
        }
        if let Some(u) = self.cache.unit.get(ent) {
            // TODO Phase 1[d]: Unit.current_hp still i32 — boundary at conversion.
            return RSome(Fixed32::from_i32(u.current_hp));
        }
        RNone
    }

    fn get_max_hp(&self, e: EntityHandle) -> ROption<Fixed32> {
        let Some(ent) = Self::handle_to_entity(e) else { return RNone };
        if let Some(p) = self.cache.cprop.get(ent) {
            // Phase 1c.3: CProperty.mhp now Fixed32 (Phase 1c.2) — direct return.
            return RSome(p.mhp);
        }
        if let Some(u) = self.cache.unit.get(ent) {
            // TODO Phase 1[d]: Unit.max_hp still i32 — boundary at conversion.
            return RSome(Fixed32::from_i32(u.max_hp));
        }
        RNone
    }

    fn is_alive(&self, e: EntityHandle) -> bool {
        let Some(ent) = Self::handle_to_entity(e) else { return false };
        self.cache.entities.is_alive(ent)
    }

    fn faction_of(&self, _e: EntityHandle) -> ROption<RStr<'_>> {
        // NOTE: returning an RStr borrowed from component storage across
        // the FFI boundary is awkward (storage is locked behind Read).
        // PoC stub: return None until a real need pops up; switch to
        // RString (owned) if/when scripts actually query this.
        RNone
    }

    fn unit_id_of(&self, _e: EntityHandle) -> ROption<RStr<'_>> {
        // Same lifetime concern as `faction_of`. Scripts typically don't need
        // to query their own unit_id — they know it statically.
        RNone
    }

    fn query_enemies_in_range(
        &self,
        center: Vec2,
        radius: Fixed32,
        of: EntityHandle,
    ) -> RVec<EntityHandle> {
        let Some(of_ent) = Self::handle_to_entity(of) else { return RVec::new() };
        let my_team = match self.cache.faction.get(of_ent) {
            Some(f) => f.team_id,
            None => return RVec::new(),
        };

        let r2 = radius * radius;
        let mut out: RVec<EntityHandle> = RVec::new();

        for (ent, pos, fac) in (&self.cache.entities, &self.cache.pos, &self.cache.faction).join() {
            if fac.team_id == my_team { continue; }
            if pos.0.distance_squared(center) <= r2 {
                out.push(Self::entity_to_handle(ent));
            }
        }
        out
    }

    // ---------------- Mutate ----------------

    fn set_pos(&mut self, e: EntityHandle, p: Vec2) {
        let Some(ent) = Self::handle_to_entity(e) else { return };
        if let Some(pos) = self.cache.pos.get_mut(ent) {
            pos.0 = p;
        }
    }

    fn advance_with_collision(
        &mut self,
        e: EntityHandle,
        target: Vec2,
        step: Fixed32,
    ) -> Vec2 {
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
            .unwrap_or(Fixed32::from_i32(30));
        // Phase 1b.3: collision tick now takes Fixed32 / Vec2 directly; no boundary
        // conversion needed (ABI types and omoba_sim types coincide).
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
        amount: Fixed32,
        _kind: DamageKind,
        _source: ROption<EntityHandle>,
    ) {
        let Some(ent) = Self::handle_to_entity(target) else { return };
        // Phase 1c.3: CProperty.hp is Fixed32 — direct arithmetic.
        if let Some(p) = self.cache.cprop.get_mut(ent) {
            let new_hp = p.hp - amount;
            p.hp = if new_hp < Fixed32::ZERO { Fixed32::ZERO } else { new_hp };
            return;
        }
        if let Some(u) = self.cache.unit.get_mut(ent) {
            // TODO Phase 1[d]: Unit.current_hp still i32 — Fixed32 boundary on damage application.
            let amount_i = amount.to_f32_for_render() as i32;
            u.current_hp = (u.current_hp - amount_i).max(0);
        }
    }

    fn heal(&mut self, target: EntityHandle, amount: Fixed32) {
        let Some(ent) = Self::handle_to_entity(target) else { return };
        // Phase 1c.3: CProperty.hp / mhp now Fixed32 — direct arithmetic.
        if let Some(p) = self.cache.cprop.get_mut(ent) {
            let new_hp = p.hp + amount;
            p.hp = if new_hp > p.mhp { p.mhp } else { new_hp };
            return;
        }
        if let Some(u) = self.cache.unit.get_mut(ent) {
            // TODO Phase 1[d]: Unit.current_hp still i32 — Fixed32 boundary on heal.
            let amount_i = amount.to_f32_for_render() as i32;
            u.current_hp = (u.current_hp + amount_i).min(u.max_hp);
        }
    }

    fn add_buff(&mut self, target: EntityHandle, buff_id: RStr<'_>, duration: Fixed32) {
        let Some(ent) = Self::handle_to_entity(target) else { return };
        let id_owned = buff_id.as_str().to_string();
        // Phase 1c.3: BuffStore::add now takes Fixed32 — direct call.
        self.cache.buffs.add(ent, &id_owned, duration, serde_json::Value::Null);
        self.cache.events
            .push(ScriptEvent::ModifierAdded { e: ent, modifier_id: id_owned });
    }

    fn remove_buff(&mut self, target: EntityHandle, buff_id: RStr<'_>) {
        let Some(ent) = Self::handle_to_entity(target) else { return };
        let id_owned = buff_id.as_str().to_string();
        self.cache.buffs.remove(ent, &id_owned);
        self.cache.events
            .push(ScriptEvent::ModifierRemoved { e: ent, modifier_id: id_owned });
    }

    fn has_buff(&self, target: EntityHandle, buff_id: RStr<'_>) -> bool {
        let Some(ent) = Self::handle_to_entity(target) else { return false };
        self.cache.buffs.has(ent, buff_id.as_str())
    }

    fn add_stat_buff(
        &mut self,
        target: EntityHandle,
        buff_id: RStr<'_>,
        duration: Fixed32,
        modifiers_json: RStr<'_>,
    ) {
        let Some(ent) = Self::handle_to_entity(target) else { return };
        let payload: serde_json::Value =
            serde_json::from_str(modifiers_json.as_str()).unwrap_or(serde_json::Value::Null);
        let id_owned = buff_id.as_str().to_string();
        // Phase 1c.3: BuffStore::add now takes Fixed32 — direct call.
        self.cache.buffs.add(ent, &id_owned, duration, payload);
        self.cache.events
            .push(ScriptEvent::ModifierAdded { e: ent, modifier_id: id_owned });
    }

    fn spawn_summoned_unit(
        &mut self,
        pos: Vec2,
        unit_type: RStr<'_>,
        owner: EntityHandle,
        duration: Fixed32,
    ) -> EntityHandle {
        let Some(owner_ent) = Self::handle_to_entity(owner) else {
            return EntityHandle::INVALID;
        };
        let unit_type_str = unit_type.as_str();

        // 繼承 owner 的 faction（陣營 + team_id）
        let faction = self.cache.faction
            .get(owner_ent)
            .cloned()
            .unwrap_or_else(|| Faction::new(FactionType::Player, 0));

        // TODO Phase 1[d]: drop conversions when Unit / spawn helpers migrate to Fixed32.
        let pos_x_f = pos.x.to_f32_for_render();
        let pos_y_f = pos.y.to_f32_for_render();
        let duration_f = duration.to_f32_for_render();
        let Some(unit) = Unit::create_summon_unit(unit_type_str, (pos_x_f, pos_y_f), faction.team_id) else {
            return EntityHandle::INVALID;
        };

        // Unit 內嵌數值映射出 CProperty + TAttack（hero/creep 的基礎屬性 component）
        // Phase 1c.3: Unit's move_speed / attack_speed / attack_range are already Fixed32
        // (Phase 1c.2). hp / max_hp 仍為 i32。
        let one_hundredth = Fixed32::from_raw(10); // 0.01 in Q22.10 (10/1024 ≈ 0.00977)
        let attack_speed_min = if unit.attack_speed < one_hundredth { one_hundredth } else { unit.attack_speed };
        let cprop = CProperty {
            hp: Fixed32::from_i32(unit.current_hp),
            mhp: Fixed32::from_i32(unit.max_hp),
            msd: unit.move_speed,
            def_physic: unit.base_armor,
            def_magic: unit.magic_resistance,
        };
        let tatk = TAttack {
            atk_physic: Vf32::new(Fixed32::from_i32(unit.base_damage)),
            asd: Vf32::new(Fixed32::ONE / attack_speed_min),
            range: Vf32::new(unit.attack_range),
            asd_count: Fixed32::ZERO,
            bullet_speed: Fixed32::from_i32(1000),
        };

        let summon_time = self.cache.time.0 as f32;
        let summoned = SummonedUnit::new(
            owner_ent,
            if duration_f > 0.0 { Some(duration_f) } else { None },
            summon_time,
        );

        // LazyUpdate：entity id 立刻分配，components 在下次 maintain (core.rs:364) 真正附上。
        // 本 frame 後續 system 已跑完，不會看到這個半成品。
        let e = self.cache.lazy.create_entity(&self.cache.entities)
            .with(Pos(pos))
            .with(Vel(omoba_sim::Vec2::ZERO))
            .with(unit.clone())
            .with(faction)
            .with(cprop)
            .with(tatk)
            .with(Facing(omoba_sim::Angle::ZERO))
            .with(crate::comp::FacingBroadcast(None))
            // π rad/s ≈ 3.14159 → Fixed32 raw = round(π * 1024) = 3217
            .with(TurnSpeed(omoba_sim::Fixed32::from_raw(3217)))
            .with(CollisionRadius(omoba_sim::Fixed32::from_i32(30)))
            .with(summoned)
            // 綁 ScriptUnitTag 讓 dispatch tick 呼叫 UnitScript::on_tick 驅動 AI
            .with(crate::scripting::ScriptUnitTag {
                unit_id: unit_type_str.to_string(),
            })
            .build();

        // 廣播給前端
        let _ = self.mqtx.try_send(OutboundMsg::new_s_at(
            "td/all/res", "unit", "C",
            json!({
                "id": e.id(),
                "unit_id": unit_type_str,
                "name": unit.name,
                "position": { "x": pos_x_f, "y": pos_y_f },
                "hp": unit.current_hp,
                "max_hp": unit.max_hp,
                "move_speed": unit.move_speed,
                "duration": duration_f,
            }),
            pos_x_f, pos_y_f,
        ));

        Self::entity_to_handle(e)
    }

    fn spawn_projectile_ex(&mut self, spec: ProjectileSpec) -> EntityHandle {
        let Some(owner_ent) = Self::handle_to_entity(spec.owner) else {
            return EntityHandle::INVALID;
        };
        let from = spec.from;
        // TODO Phase 1[d]: drop these conversions when Projectile struct migrates to Fixed32.
        let speed_f = spec.speed.to_f32_for_render();
        let damage_f = spec.damage.to_f32_for_render();
        let splash_radius_f = spec.splash_radius.to_f32_for_render();
        let hit_radius_f = spec.hit_radius.to_f32_for_render();
        let slow_factor_f = spec.slow_factor.to_f32_for_render();
        let slow_duration_f = spec.slow_duration.to_f32_for_render();
        let stun_duration_f = spec.stun_duration.to_f32_for_render();

        // 依 PathSpec 算 tpos + target option + end_pos（供前端直線渲染）
        let (target_opt, tpos, end_pos, is_directional, target_id_out) = match spec.path {
            PathSpec::Homing { target } => {
                let Some(target_ent) = Self::handle_to_entity(target) else {
                    return EntityHandle::INVALID;
                };
                let tp = self.cache.pos
                    .get(target_ent).map(|p| p.0).unwrap_or(from);
                (Some(target_ent), tp, tp, false, target.id)
            }
            PathSpec::Straight { end_pos } => {
                (None, end_pos, end_pos, true, 0u32)
            }
        };

        // TODO Phase 1[d]: Projectile struct fields are still f32; render-side conversion at boundary.
        let from_x_f = from.x.to_f32_for_render();
        let from_y_f = from.y.to_f32_for_render();
        let tpos_x_f = tpos.x.to_f32_for_render();
        let tpos_y_f = tpos.y.to_f32_for_render();
        let end_x_f = end_pos.x.to_f32_for_render();
        let end_y_f = end_pos.y.to_f32_for_render();

        let dx = tpos_x_f - from_x_f;
        let dy = tpos_y_f - from_y_f;
        let initial_dist = (dx * dx + dy * dy).sqrt();
        let flight_time_s: f32 = if speed_f > 0.0 {
            (initial_dist / speed_f).max(0.01)
        } else { 0.01 };
        let safety = flight_time_s * 3.0 + 1.5;

        // LazyUpdate spawn — 同 frame 後續系統已跑完，maintain 在 core.rs tick 結尾。
        let e = self.cache.lazy.create_entity(&self.cache.entities)
            .with(Pos(from))
            .with(Projectile {
                time_left: safety,
                owner: owner_ent,
                tpos: vek::Vec2::new(tpos_x_f, tpos_y_f),
                target: target_opt,
                radius: splash_radius_f,
                msd: speed_f,
                damage_phys: damage_f,
                damage_magi: 0.0,
                damage_real: 0.0,
                slow_factor: slow_factor_f,
                slow_duration: slow_duration_f,
                hit_radius: hit_radius_f,
                stun_duration: stun_duration_f,
            })
            .build();

        let flight_time_ms: u64 = (flight_time_s * 1000.0).max(1.0) as u64;
        // P7 layered (re-enabled): pre-declared single-target damage. AOE
        // (splash > 0), directional, or untargeted shots still carry 0 —
        // those don't fit the in_flight reconciliation model and stay
        // server-broadcast (creep/H per impact).
        let predeclared_dmg = if splash_radius_f > 0.0 || is_directional || target_id_out == 0 {
            0.0
        } else {
            damage_f
        };
        let _ = self.mqtx.try_send(make_projectile_create_script(
            e.id(), target_id_out,
            from_x_f, from_y_f, end_x_f, end_y_f,
            speed_f, flight_time_ms,
            is_directional, splash_radius_f, hit_radius_f, spec.kind_id,
            predeclared_dmg,
        ));

        Self::entity_to_handle(e)
    }

    fn emit_explosion(&mut self, pos: Vec2, radius: Fixed32, duration: Fixed32) {
        // TODO Phase 1[d]: drop conversions when explosion VFX takes Fixed32.
        let _ = self.mqtx.try_send(make_game_explosion_script(
            pos.x.to_f32_for_render(),
            pos.y.to_f32_for_render(),
            radius.to_f32_for_render(),
            duration.to_f32_for_render(),
        ));
    }

    fn despawn(&mut self, e: EntityHandle) {
        let Some(ent) = Self::handle_to_entity(e) else { return };
        // EntitiesRes::delete 內部走 atomic flag，&Entities 即可。
        let _ = self.cache.entities.delete(ent);
    }

    // ---------------- 塔 / 單位屬性 ----------------

    fn get_tower_range(&self, e: EntityHandle) -> Fixed32 {
        let Some(ent) = Self::handle_to_entity(e) else { return Fixed32::ZERO };
        // Phase 1c.3: TAttack.range.v is Fixed32 — direct return.
        self.cache.tattack.get(ent).map(|t| t.range.v).unwrap_or(Fixed32::ZERO)
    }

    fn get_tower_atk(&self, e: EntityHandle) -> Fixed32 {
        let Some(ent) = Self::handle_to_entity(e) else { return Fixed32::ZERO };
        // Phase 1c.3: TAttack.atk_physic.v is Fixed32 — direct return.
        self.cache.tattack.get(ent).map(|t| t.atk_physic.v).unwrap_or(Fixed32::ZERO)
    }

    fn get_asd_interval(&self, e: EntityHandle) -> Fixed32 {
        let Some(ent) = Self::handle_to_entity(e) else { return Fixed32::ZERO };
        // Phase 1c.3: TAttack.asd.v is Fixed32 — direct return.
        self.cache.tattack.get(ent).map(|t| t.asd.v).unwrap_or(Fixed32::ZERO)
    }

    fn get_asd_count(&self, e: EntityHandle) -> Fixed32 {
        let Some(ent) = Self::handle_to_entity(e) else { return Fixed32::ZERO };
        // Phase 1c.3: TAttack.asd_count is Fixed32 — direct return.
        self.cache.tattack.get(ent).map(|t| t.asd_count).unwrap_or(Fixed32::ZERO)
    }

    fn set_asd_count(&mut self, e: EntityHandle, v: Fixed32) {
        let Some(ent) = Self::handle_to_entity(e) else { return };
        if let Some(t) = self.cache.tattack.get_mut(ent) {
            // Phase 1c.3: TAttack.asd_count is Fixed32 — direct write.
            t.asd_count = v;
        }
    }

    fn set_tower_atk(&mut self, e: EntityHandle, v: Fixed32) {
        let Some(ent) = Self::handle_to_entity(e) else { return };
        if let Some(t) = self.cache.tattack.get_mut(ent) {
            // Phase 1c.3: Vf32 holds Fixed32 — direct write.
            t.atk_physic.bv = v;
            t.atk_physic.v = v;
        }
    }

    fn set_tower_range(&mut self, e: EntityHandle, v: Fixed32) {
        let Some(ent) = Self::handle_to_entity(e) else { return };
        if let Some(t) = self.cache.tattack.get_mut(ent) {
            // Phase 1c.3: Vf32 holds Fixed32 — direct write.
            t.range.bv = v;
            t.range.v = v;
        }
    }

    fn set_asd_interval(&mut self, e: EntityHandle, v: Fixed32) {
        let Some(ent) = Self::handle_to_entity(e) else { return };
        if let Some(t) = self.cache.tattack.get_mut(ent) {
            // Phase 1c.3: Vf32 holds Fixed32 — direct write.
            t.asd.bv = v;
            t.asd.v = v;
        }
    }

    fn set_facing(&mut self, e: EntityHandle, angle: Angle) {
        let Some(ent) = Self::handle_to_entity(e) else { return };
        if let Some(f) = self.cache.facing.get_mut(ent) {
            f.0 = angle;
        }
    }

    fn get_facing(&self, e: EntityHandle) -> Angle {
        let Some(ent) = Self::handle_to_entity(e) else { return Angle::ZERO };
        self.cache.facing.get(ent).map(|f| f.0).unwrap_or(Angle::ZERO)
    }

    fn query_nearest_enemy(
        &self,
        center: Vec2,
        radius: Fixed32,
        of: EntityHandle,
    ) -> ROption<EntityHandle> {
        let Some(of_ent) = Self::handle_to_entity(of) else { return RNone };
        let my_team = match self.cache.faction.get(of_ent) {
            Some(f) => f.team_id,
            None => return RNone,
        };
        let r2 = radius * radius;
        let mut best: Option<(Entity, Fixed32)> = None;
        // 只選 creep（氣球）為目標；不要誤選隊友/其他塔
        for (ent, pos, fac, _c) in (&self.cache.entities, &self.cache.pos, &self.cache.faction, &self.cache.creep).join() {
            if fac.team_id == my_team { continue; }
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

    // ---------------- Side effects ----------------

    fn play_vfx(&mut self, id: RStr<'_>, at: Vec2) {
        // TODO Phase 1[d]: drop conversion when VFX bus takes Fixed32.
        log::debug!("[scripting] play_vfx id={} at=({},{})", id.as_str(),
            at.x.to_f32_for_render(), at.y.to_f32_for_render());
    }

    fn play_sfx(&mut self, id: RStr<'_>, at: Vec2) {
        // TODO Phase 1[d]: drop conversion when VFX bus takes Fixed32.
        log::debug!("[scripting] play_sfx id={} at=({},{})", id.as_str(),
            at.x.to_f32_for_render(), at.y.to_f32_for_render());
    }

    // ---------------- RNG ----------------

    fn rand_unit(&mut self) -> Fixed32 {
        // TODO Phase 1[d]: replace with omoba_sim::SimRng for full deterministic
        // bit-exact replay. Today we're piggy-backing on Pcg64Mcg seeded from tick
        // counter (deterministic across replays, but f32 sample loses 6 bits at
        // the SCALE=1024 quantization).
        let r = self.rng.gen_range(0.0_f32..1.0_f32);
        Fixed32::from_raw((r * 1024.0) as i32)
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

    fn sum_stat(&self, e: EntityHandle, stat_key: StatKey) -> Fixed32 {
        let Some(ent) = Self::handle_to_entity(e) else { return Fixed32::ZERO };
        // Phase 1c.3: BuffStore::sum_add now returns Fixed32 — direct return.
        self.cache.buffs.sum_add(ent, stat_key)
    }

    fn product_stat(&self, e: EntityHandle, stat_key: StatKey) -> Fixed32 {
        let Some(ent) = Self::handle_to_entity(e) else { return Fixed32::ONE };
        // Phase 1c.3: BuffStore::product_mult now returns Fixed32 — direct return.
        self.cache.buffs.product_mult(ent, stat_key)
    }

    fn get_final_move_speed(&self, e: EntityHandle) -> Fixed32 {
        let Some(ent) = Self::handle_to_entity(e) else { return Fixed32::ZERO };
        // Phase 1c.3: CProperty.msd is Fixed32 (Phase 1c.2) — direct read.
        let base = self.cache.cprop.get(ent).map(|p| p.msd).unwrap_or(Fixed32::ZERO);
        let is_b = self.cache.is_building.get(ent).is_some();
        // Phase 1c.3: UnitStats::final_move_speed now returns Fixed32 — direct return.
        UnitStats::from_refs(&*self.cache.buffs, is_b).final_move_speed(base, ent)
    }

    fn get_final_atk(&self, e: EntityHandle) -> Fixed32 {
        let Some(ent) = Self::handle_to_entity(e) else { return Fixed32::ZERO };
        // TAttack.atk_physic.v is Fixed32 (Phase 1c.2 — Vf32 holds Fixed32 internally).
        let base = self.cache.tattack.get(ent).map(|t| t.atk_physic.v).unwrap_or(Fixed32::ZERO);
        let is_b = self.cache.is_building.get(ent).is_some();
        // Phase 1c.3: UnitStats::final_atk now returns Fixed32 — direct return.
        UnitStats::from_refs(&*self.cache.buffs, is_b).final_atk(base, ent)
    }

    fn get_tower_upgrade(&self, e: EntityHandle, path: u8) -> u8 {
        let Some(ent) = Self::handle_to_entity(e) else { return 0 };
        self.cache.tower.get(ent)
            .and_then(|t| t.upgrade_levels.get(path as usize))
            .copied()
            .unwrap_or(0)
    }

    fn has_tower_flag(&self, e: EntityHandle, flag: RStr<'_>) -> bool {
        let Some(ent) = Self::handle_to_entity(e) else { return false };
        self.cache.tower.get(ent)
            .map(|t| t.upgrade_flags.iter().any(|f| f == flag.as_str()))
            .unwrap_or(false)
    }

    fn apply_tower_permanent_buff(&mut self, e: EntityHandle, buff_id: RStr<'_>, modifiers_json: RStr<'_>) {
        // TODO Phase 1[d]: replace with a sentinel "permanent" Fixed32 instead of MAX-clamp magic.
        // Use Fixed32::from_raw(i32::MAX) — large enough that buff_tick won't decrement to zero in any reasonable session.
        self.add_stat_buff(e, buff_id, Fixed32::from_raw(i32::MAX), modifiers_json);
    }

    fn get_final_attack_range(&self, e: EntityHandle) -> Fixed32 {
        let Some(ent) = Self::handle_to_entity(e) else { return Fixed32::ZERO };
        let base = self.cache.tattack.get(ent).map(|t| t.range.v).unwrap_or(Fixed32::ZERO);
        let is_b = self.cache.is_building.get(ent).is_some();
        // Phase 1c.3: UnitStats::final_attack_range now returns Fixed32 — direct return.
        UnitStats::from_refs(&*self.cache.buffs, is_b).final_attack_range(base, ent)
    }

    fn get_buff_remaining(&self, e: EntityHandle, buff_id: RStr<'_>) -> Fixed32 {
        let Some(ent) = Self::handle_to_entity(e) else { return Fixed32::ZERO };
        // Phase 1c.3: BuffEntry.remaining now Fixed32 — direct return.
        self.cache.buffs
            .get(ent, buff_id.as_str())
            .map(|b| b.remaining)
            .unwrap_or(Fixed32::ZERO)
    }

    fn current_mana(&self, e: EntityHandle) -> Fixed32 {
        // 沒有 current_mana component — 目前回 max（視為永遠滿）。
        // 如果之後加 `ManaPool` component，這裡要改成讀 current。
        let Some(ent) = Self::handle_to_entity(e) else { return Fixed32::ZERO };
        // Phase 1c.3: Hero.get_max_mana now returns Fixed32 — direct return.
        self.cache.hero.get(ent).map(|h| h.get_max_mana()).unwrap_or(Fixed32::ZERO)
    }

    fn spend_mana(&mut self, e: EntityHandle, amount: Fixed32, ability_id: RStr<'_>) -> bool {
        let Some(ent) = Self::handle_to_entity(e) else { return false };
        // 目前沒有 mana storage，永遠視為成功；push 事件讓腳本 hook。
        self.cache.events
            .push(ScriptEvent::SpentMana {
                caster: ent,
                // Phase 1c.3: ScriptEvent::SpentMana.cost now Fixed32 — direct push.
                cost: amount,
                ability_id: ability_id.as_str().to_string(),
            });
        true
    }

    fn restore_mana(&mut self, e: EntityHandle, amount: Fixed32) {
        let Some(ent) = Self::handle_to_entity(e) else { return };
        // Phase 1c.3: ScriptEvent::ManaGained.amount now Fixed32 — direct push.
        self.cache.events
            .push(ScriptEvent::ManaGained { e: ent, amount });
    }

    fn trigger_modifier_added(&mut self, e: EntityHandle, modifier_id: RStr<'_>) {
        let Some(ent) = Self::handle_to_entity(e) else { return };
        self.cache.events
            .push(ScriptEvent::ModifierAdded {
                e: ent,
                modifier_id: modifier_id.as_str().to_string(),
            });
    }

    fn trigger_state_changed(&mut self, e: EntityHandle, state_id: RStr<'_>, active: bool) {
        let Some(ent) = Self::handle_to_entity(e) else { return };
        self.cache.events
            .push(ScriptEvent::StateChanged {
                e: ent,
                state_id: state_id.as_str().to_string(),
                active,
            });
    }

    // ---------------- Dota 2 property 完整查詢 ----------------

    fn get_final_armor(&self, e: EntityHandle) -> Fixed32 {
        let Some(ent) = Self::handle_to_entity(e) else { return Fixed32::ZERO };
        let is_bldg = self.cache.is_building.get(ent).is_some();
        // Phase 1c.3: CProperty.def_physic is Fixed32 (Phase 1c.2) — direct read.
        let base = self.cache.cprop.get(ent).map(|c| c.def_physic).unwrap_or(Fixed32::ZERO);
        // Phase 1c.3: UnitStats::final_armor now returns Fixed32 — direct return.
        UnitStats::from_refs(&*self.cache.buffs, is_bldg).final_armor(base, ent)
    }

    fn get_final_magic_resist(&self, e: EntityHandle) -> Fixed32 {
        let Some(ent) = Self::handle_to_entity(e) else { return Fixed32::ZERO };
        let is_bldg = self.cache.is_building.get(ent).is_some();
        // Phase 1c.3: CProperty.def_magic is Fixed32 (Phase 1c.2) — direct read.
        let base = self.cache.cprop.get(ent).map(|c| c.def_magic).unwrap_or(Fixed32::ZERO);
        // Phase 1c.3: UnitStats::final_magic_resist now returns Fixed32 — direct return.
        UnitStats::from_refs(&*self.cache.buffs, is_bldg).final_magic_resist(base, ent)
    }

    fn get_evasion_chance(&self, e: EntityHandle) -> Fixed32 {
        let Some(ent) = Self::handle_to_entity(e) else { return Fixed32::ZERO };
        let is_bldg = self.cache.is_building.get(ent).is_some();
        // Phase 1c.3: UnitStats::evasion_chance now returns Fixed32 — direct return.
        UnitStats::from_refs(&*self.cache.buffs, is_bldg).evasion_chance(ent)
    }

    fn get_miss_chance(&self, e: EntityHandle) -> Fixed32 {
        let Some(ent) = Self::handle_to_entity(e) else { return Fixed32::ZERO };
        let is_bldg = self.cache.is_building.get(ent).is_some();
        // Phase 1c.3: UnitStats::miss_chance now returns Fixed32 — direct return.
        UnitStats::from_refs(&*self.cache.buffs, is_bldg).miss_chance(ent)
    }

    fn get_crit_chance(&self, e: EntityHandle) -> Fixed32 {
        let Some(ent) = Self::handle_to_entity(e) else { return Fixed32::ZERO };
        let is_bldg = self.cache.is_building.get(ent).is_some();
        // Phase 1c.3: UnitStats::crit now returns (Fixed32, Fixed32) — direct return.
        UnitStats::from_refs(&*self.cache.buffs, is_bldg).crit(ent).0
    }

    fn get_crit_multiplier(&self, e: EntityHandle) -> Fixed32 {
        let Some(ent) = Self::handle_to_entity(e) else { return Fixed32::ONE };
        let is_bldg = self.cache.is_building.get(ent).is_some();
        // Phase 1c.3: UnitStats::crit now returns (Fixed32, Fixed32) — direct return.
        UnitStats::from_refs(&*self.cache.buffs, is_bldg).crit(ent).1
    }

    fn get_cooldown_mult(&self, e: EntityHandle) -> Fixed32 {
        let Some(ent) = Self::handle_to_entity(e) else { return Fixed32::ONE };
        let is_bldg = self.cache.is_building.get(ent).is_some();
        // Phase 1c.3: UnitStats::cooldown_mult now returns Fixed32 — direct return.
        UnitStats::from_refs(&*self.cache.buffs, is_bldg).cooldown_mult(ent)
    }

    fn is_building(&self, e: EntityHandle) -> bool {
        let Some(ent) = Self::handle_to_entity(e) else { return false };
        self.cache.is_building.get(ent).is_some()
    }

    fn get_max_hp_bonus(&self, e: EntityHandle) -> Fixed32 {
        let Some(ent) = Self::handle_to_entity(e) else { return Fixed32::ZERO };
        let is_bldg = self.cache.is_building.get(ent).is_some();
        // Phase 1c.3: UnitStats::max_hp_bonus now returns Fixed32 — direct return.
        UnitStats::from_refs(&*self.cache.buffs, is_bldg).max_hp_bonus(ent)
    }

    fn get_hp_regen(&self, e: EntityHandle) -> Fixed32 {
        let Some(ent) = Self::handle_to_entity(e) else { return Fixed32::ZERO };
        let is_bldg = self.cache.is_building.get(ent).is_some();
        // Phase 1c.3: UnitStats::hp_regen now returns Fixed32 — direct return.
        UnitStats::from_refs(&*self.cache.buffs, is_bldg).hp_regen(Fixed32::ZERO, ent)
    }

    fn get_stat_bonus(&self, e: EntityHandle, key: StatKey) -> Fixed32 {
        let Some(ent) = Self::handle_to_entity(e) else { return Fixed32::ZERO };
        // Phase 1c.3: BuffStore::sum_add now returns Fixed32 — direct return.
        self.cache.buffs.sum_add(ent, key)
    }

    fn deal_damage_splash(
        &mut self,
        at: Vec2,
        radius: Fixed32,
        damage: Fixed32,
        kind: DamageKind,
        source: ROption<EntityHandle>,
    ) {
        // 以 source 為 "of" 參考；若 source 無則以 at 對 "空" 做查詢（query_enemies_in_range 需要一個 of）。
        let of = match source {
            ROption::RSome(h) => h,
            ROption::RNone => return,
        };
        let targets = self.query_enemies_in_range(at, radius, of);
        for th in targets.iter() {
            self.deal_damage(*th, damage, kind, source);
        }
    }
}

