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
    types::{DamageKind, EntityHandle, PathSpec, ProjectileSpec, Vec2f},
    world::GameWorld,
};
use rand::{Rng, SeedableRng};
use rand_pcg::Pcg64Mcg;
use serde_json::json;
use specs::{Builder, Entity, Join, World, WorldExt};
use specs::world::Generation;

use crate::ability_runtime::{BuffStore, UnitStats};
use crate::comp::*;
use crate::scripting::event::{ScriptEvent, ScriptEventQueue};
use crate::transport::OutboundMsg;

/// Host-side adapter. Created fresh for each `run_script_dispatch` call.
///
/// `log_str_scratch` is needed because `log_info(&self, ...)` is `&self`
/// but we format into `&mut String`; a `RefCell` avoids that.
pub struct WorldAdapter<'a> {
    pub world: &'a mut World,
    pub rng: Pcg64Mcg,
    /// 廣播給前端的 sender；supply_projectile_ex、emit_explosion 會用到
    pub mqtx: Sender<OutboundMsg>,
}

impl<'a> WorldAdapter<'a> {
    pub fn new(world: &'a mut World, seed: u64, mqtx: Sender<OutboundMsg>) -> Self {
        Self {
            world,
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

impl<'a> GameWorld for WorldAdapter<'a> {
    // ---------------- Query ----------------

    fn get_pos(&self, e: EntityHandle) -> ROption<Vec2f> {
        let Some(ent) = Self::handle_to_entity(e) else { return RNone };
        let store = self.world.read_storage::<Pos>();
        match store.get(ent) {
            Some(p) => RSome(Vec2f { x: p.0.x, y: p.0.y }),
            None => RNone,
        }
    }

    fn get_hp(&self, e: EntityHandle) -> ROption<f32> {
        let Some(ent) = Self::handle_to_entity(e) else { return RNone };
        // Prefer CProperty (used by creeps/towers in TD mode); fall back to Unit.
        if let Some(p) = self.world.read_storage::<CProperty>().get(ent) {
            return RSome(p.hp);
        }
        if let Some(u) = self.world.read_storage::<Unit>().get(ent) {
            return RSome(u.current_hp as f32);
        }
        RNone
    }

    fn get_max_hp(&self, e: EntityHandle) -> ROption<f32> {
        let Some(ent) = Self::handle_to_entity(e) else { return RNone };
        if let Some(p) = self.world.read_storage::<CProperty>().get(ent) {
            return RSome(p.mhp);
        }
        if let Some(u) = self.world.read_storage::<Unit>().get(ent) {
            return RSome(u.max_hp as f32);
        }
        RNone
    }

    fn is_alive(&self, e: EntityHandle) -> bool {
        let Some(ent) = Self::handle_to_entity(e) else { return false };
        self.world.entities().is_alive(ent)
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
        center: Vec2f,
        radius: f32,
        of: EntityHandle,
    ) -> RVec<EntityHandle> {
        let Some(of_ent) = Self::handle_to_entity(of) else { return RVec::new() };
        let entities = self.world.entities();
        let positions = self.world.read_storage::<Pos>();
        let factions = self.world.read_storage::<Faction>();

        let my_team = match factions.get(of_ent) {
            Some(f) => f.team_id,
            None => return RVec::new(),
        };

        let r2 = radius * radius;
        let cx = center.x;
        let cy = center.y;
        let mut out: RVec<EntityHandle> = RVec::new();

        for (ent, pos, fac) in (&entities, &positions, &factions).join() {
            if fac.team_id == my_team { continue; }
            let dx = pos.0.x - cx;
            let dy = pos.0.y - cy;
            if dx * dx + dy * dy <= r2 {
                out.push(Self::entity_to_handle(ent));
            }
        }
        out
    }

    // ---------------- Mutate ----------------

    fn set_pos(&mut self, e: EntityHandle, p: Vec2f) {
        let Some(ent) = Self::handle_to_entity(e) else { return };
        let mut store = self.world.write_storage::<Pos>();
        if let Some(pos) = store.get_mut(ent) {
            pos.0.x = p.x;
            pos.0.y = p.y;
        }
    }

    fn advance_with_collision(
        &mut self,
        e: EntityHandle,
        target: Vec2f,
        step: f32,
    ) -> Vec2f {
        let Some(ent) = Self::handle_to_entity(e) else {
            return target;
        };
        let pos = match self.world.read_storage::<Pos>().get(ent) {
            Some(p) => p.0,
            None => return target,
        };
        let radius = self
            .world
            .read_storage::<CollisionRadius>()
            .get(ent)
            .map(|r| r.0)
            .unwrap_or(30.0);
        let target_vek = vek::Vec2::new(target.x, target.y);
        let searcher = self.world.read_resource::<Searcher>();
        let radii = self.world.read_storage::<CollisionRadius>();
        let regions = self.world.read_resource::<BlockedRegions>();
        let (new_pos, _reached) = crate::tick::hero_move_tick::advance_with_collision(
            pos, target_vek, step, radius, &searcher, &radii, ent, &regions,
        );
        Vec2f::new(new_pos.x, new_pos.y)
    }

    fn deal_damage(
        &mut self,
        target: EntityHandle,
        amount: f32,
        _kind: DamageKind,
        _source: ROption<EntityHandle>,
    ) {
        let Some(ent) = Self::handle_to_entity(target) else { return };
        // Prefer CProperty (TD mode).
        {
            let mut store = self.world.write_storage::<CProperty>();
            if let Some(p) = store.get_mut(ent) {
                p.hp = (p.hp - amount).max(0.0);
                return;
            }
        }
        let mut store = self.world.write_storage::<Unit>();
        if let Some(u) = store.get_mut(ent) {
            u.current_hp = (u.current_hp - amount as i32).max(0);
        }
    }

    fn heal(&mut self, target: EntityHandle, amount: f32) {
        let Some(ent) = Self::handle_to_entity(target) else { return };
        {
            let mut store = self.world.write_storage::<CProperty>();
            if let Some(p) = store.get_mut(ent) {
                p.hp = (p.hp + amount).min(p.mhp);
                return;
            }
        }
        let mut store = self.world.write_storage::<Unit>();
        if let Some(u) = store.get_mut(ent) {
            u.current_hp = (u.current_hp + amount as i32).min(u.max_hp);
        }
    }

    fn add_buff(&mut self, target: EntityHandle, buff_id: RStr<'_>, duration: f32) {
        let Some(ent) = Self::handle_to_entity(target) else { return };
        let id_owned = buff_id.as_str().to_string();
        {
            let mut store = self.world.write_resource::<BuffStore>();
            store.add(ent, &id_owned, duration, serde_json::Value::Null);
        }
        self.world.write_resource::<ScriptEventQueue>()
            .push(ScriptEvent::ModifierAdded { e: ent, modifier_id: id_owned });
    }

    fn remove_buff(&mut self, target: EntityHandle, buff_id: RStr<'_>) {
        let Some(ent) = Self::handle_to_entity(target) else { return };
        let id_owned = buff_id.as_str().to_string();
        {
            let mut store = self.world.write_resource::<BuffStore>();
            store.remove(ent, &id_owned);
        }
        self.world.write_resource::<ScriptEventQueue>()
            .push(ScriptEvent::ModifierRemoved { e: ent, modifier_id: id_owned });
    }

    fn has_buff(&self, target: EntityHandle, buff_id: RStr<'_>) -> bool {
        let Some(ent) = Self::handle_to_entity(target) else { return false };
        let store = self.world.read_resource::<BuffStore>();
        store.has(ent, buff_id.as_str())
    }

    fn add_stat_buff(
        &mut self,
        target: EntityHandle,
        buff_id: RStr<'_>,
        duration: f32,
        modifiers_json: RStr<'_>,
    ) {
        let Some(ent) = Self::handle_to_entity(target) else { return };
        let payload: serde_json::Value =
            serde_json::from_str(modifiers_json.as_str()).unwrap_or(serde_json::Value::Null);
        let id_owned = buff_id.as_str().to_string();
        {
            let mut store = self.world.write_resource::<BuffStore>();
            store.add(ent, &id_owned, duration, payload);
        }
        self.world.write_resource::<ScriptEventQueue>()
            .push(ScriptEvent::ModifierAdded { e: ent, modifier_id: id_owned });
    }

    fn spawn_summoned_unit(
        &mut self,
        pos: Vec2f,
        unit_type: RStr<'_>,
        owner: EntityHandle,
        duration: f32,
    ) -> EntityHandle {
        let Some(owner_ent) = Self::handle_to_entity(owner) else {
            return EntityHandle::INVALID;
        };
        let unit_type_str = unit_type.as_str();

        // 繼承 owner 的 faction（陣營 + team_id）
        let faction = {
            let factions = self.world.read_storage::<Faction>();
            factions.get(owner_ent).cloned().unwrap_or_else(|| Faction::new(FactionType::Player, 0))
        };

        let Some(unit) = Unit::create_summon_unit(unit_type_str, (pos.x, pos.y), faction.team_id) else {
            return EntityHandle::INVALID;
        };

        // Unit 內嵌數值映射出 CProperty + TAttack（hero/creep 的基礎屬性 component）
        let cprop = CProperty {
            hp: unit.current_hp as f32,
            mhp: unit.max_hp as f32,
            msd: unit.move_speed,
            def_physic: unit.base_armor,
            def_magic: unit.magic_resistance,
        };
        let tatk = TAttack {
            atk_physic: Vf32::new(unit.base_damage as f32),
            asd: Vf32::new(1.0 / unit.attack_speed.max(0.01)),
            range: Vf32::new(unit.attack_range),
            asd_count: 0.0,
            bullet_speed: 1000.0,
        };

        let summon_time = self.world.read_resource::<Time>().0 as f32;
        let summoned = SummonedUnit::new(
            owner_ent,
            if duration > 0.0 { Some(duration) } else { None },
            summon_time,
        );

        let e = self.world.create_entity()
            .with(Pos(vek::Vec2::new(pos.x, pos.y)))
            .with(Vel(vek::Vec2::new(0.0, 0.0)))
            .with(unit.clone())
            .with(faction)
            .with(cprop)
            .with(tatk)
            .with(Facing(0.0))
            .with(TurnSpeed(std::f32::consts::PI))
            .with(CollisionRadius(30.0))
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
                "position": { "x": pos.x, "y": pos.y },
                "hp": unit.current_hp,
                "max_hp": unit.max_hp,
                "move_speed": unit.move_speed,
                "duration": duration,
            }),
            pos.x, pos.y,
        ));

        Self::entity_to_handle(e)
    }

    fn spawn_projectile_ex(&mut self, spec: ProjectileSpec) -> EntityHandle {
        let Some(owner_ent) = Self::handle_to_entity(spec.owner) else {
            return EntityHandle::INVALID;
        };
        let from_vek = vek::Vec2::new(spec.from.x, spec.from.y);

        // 依 PathSpec 算 tpos + target option + end_pos（供前端直線渲染）
        let (target_opt, tpos_vek, end_pos_vek, is_directional, target_id_out) = match spec.path {
            PathSpec::Homing { target } => {
                let Some(target_ent) = Self::handle_to_entity(target) else {
                    return EntityHandle::INVALID;
                };
                let tpos = self.world.read_storage::<Pos>()
                    .get(target_ent).map(|p| p.0).unwrap_or(from_vek);
                (Some(target_ent), tpos, tpos, false, target.id)
            }
            PathSpec::Straight { end_pos } => {
                let end = vek::Vec2::new(end_pos.x, end_pos.y);
                (None, end, end, true, 0u32)
            }
        };

        let initial_dist = (tpos_vek - from_vek).magnitude();
        let flight_time_s: f32 = if spec.speed > 0.0 {
            (initial_dist / spec.speed).max(0.01)
        } else { 0.01 };
        let safety = flight_time_s * 3.0 + 1.5;

        let e = self.world.create_entity()
            .with(Pos(from_vek))
            .with(Projectile {
                time_left: safety,
                owner: owner_ent,
                tpos: tpos_vek,
                target: target_opt,
                radius: spec.splash_radius,
                msd: spec.speed,
                damage_phys: spec.damage,
                damage_magi: 0.0,
                damage_real: 0.0,
                slow_factor: spec.slow_factor,
                slow_duration: spec.slow_duration,
                hit_radius: spec.hit_radius,
                stun_duration: spec.stun_duration,
            })
            .build();

        let flight_time_ms: u64 = (flight_time_s * 1000.0).max(1.0) as u64;
        let kind_str = spec.kind_tag.as_str();
        let pjs = json!({
            "id": e.id(),
            "target_id": target_id_out,
            "start_pos": { "x": from_vek.x, "y": from_vek.y },
            "end_pos":   { "x": end_pos_vek.x, "y": end_pos_vek.y },
            "move_speed": spec.speed,
            "flight_time_ms": flight_time_ms,
            "kind": kind_str,
            "directional": is_directional,
            "hit_radius": spec.hit_radius,
            "splash_radius": spec.splash_radius,
        });
        let _ = self.mqtx.try_send(OutboundMsg::new_s_at(
            "td/all/res", "projectile", "C", pjs, from_vek.x, from_vek.y,
        ));

        Self::entity_to_handle(e)
    }

    fn emit_explosion(&mut self, pos: Vec2f, radius: f32, duration: f32) {
        let _ = self.mqtx.try_send(OutboundMsg::new_s_at(
            "td/all/res", "game", "explosion",
            json!({
                "x": pos.x,
                "y": pos.y,
                "radius": radius,
                "duration": duration,
            }),
            pos.x, pos.y,
        ));
    }

    fn despawn(&mut self, e: EntityHandle) {
        let Some(ent) = Self::handle_to_entity(e) else { return };
        let _ = self.world.entities().delete(ent);
    }

    // ---------------- 塔 / 單位屬性 ----------------

    fn get_tower_range(&self, e: EntityHandle) -> f32 {
        let Some(ent) = Self::handle_to_entity(e) else { return 0.0 };
        self.world.read_storage::<TAttack>()
            .get(ent).map(|t| t.range.v).unwrap_or(0.0)
    }

    fn get_tower_atk(&self, e: EntityHandle) -> f32 {
        let Some(ent) = Self::handle_to_entity(e) else { return 0.0 };
        self.world.read_storage::<TAttack>()
            .get(ent).map(|t| t.atk_physic.v).unwrap_or(0.0)
    }

    fn get_asd_interval(&self, e: EntityHandle) -> f32 {
        let Some(ent) = Self::handle_to_entity(e) else { return 0.0 };
        self.world.read_storage::<TAttack>()
            .get(ent).map(|t| t.asd.v).unwrap_or(0.0)
    }

    fn get_asd_count(&self, e: EntityHandle) -> f32 {
        let Some(ent) = Self::handle_to_entity(e) else { return 0.0 };
        self.world.read_storage::<TAttack>()
            .get(ent).map(|t| t.asd_count).unwrap_or(0.0)
    }

    fn set_asd_count(&mut self, e: EntityHandle, v: f32) {
        let Some(ent) = Self::handle_to_entity(e) else { return };
        let mut store = self.world.write_storage::<TAttack>();
        if let Some(t) = store.get_mut(ent) {
            t.asd_count = v;
        }
    }

    fn set_tower_atk(&mut self, e: EntityHandle, v: f32) {
        let Some(ent) = Self::handle_to_entity(e) else { return };
        let mut store = self.world.write_storage::<TAttack>();
        if let Some(t) = store.get_mut(ent) {
            t.atk_physic.bv = v;
            t.atk_physic.v = v;
        }
    }

    fn set_tower_range(&mut self, e: EntityHandle, v: f32) {
        let Some(ent) = Self::handle_to_entity(e) else { return };
        let mut store = self.world.write_storage::<TAttack>();
        if let Some(t) = store.get_mut(ent) {
            t.range.bv = v;
            t.range.v = v;
        }
    }

    fn set_asd_interval(&mut self, e: EntityHandle, v: f32) {
        let Some(ent) = Self::handle_to_entity(e) else { return };
        let mut store = self.world.write_storage::<TAttack>();
        if let Some(t) = store.get_mut(ent) {
            t.asd.bv = v;
            t.asd.v = v;
        }
    }

    fn set_facing(&mut self, e: EntityHandle, angle_rad: f32) {
        let Some(ent) = Self::handle_to_entity(e) else { return };
        let mut store = self.world.write_storage::<Facing>();
        if let Some(f) = store.get_mut(ent) {
            f.0 = angle_rad;
        }
    }

    fn get_facing(&self, e: EntityHandle) -> f32 {
        let Some(ent) = Self::handle_to_entity(e) else { return 0.0 };
        self.world.read_storage::<Facing>().get(ent).map(|f| f.0).unwrap_or(0.0)
    }

    fn query_nearest_enemy(
        &self,
        center: Vec2f,
        radius: f32,
        of: EntityHandle,
    ) -> ROption<EntityHandle> {
        let Some(of_ent) = Self::handle_to_entity(of) else { return RNone };
        let entities = self.world.entities();
        let positions = self.world.read_storage::<Pos>();
        let factions = self.world.read_storage::<Faction>();
        let creeps = self.world.read_storage::<Creep>();

        let my_team = match factions.get(of_ent) {
            Some(f) => f.team_id,
            None => return RNone,
        };
        let r2 = radius * radius;
        let mut best: Option<(Entity, f32)> = None;
        // 只選 creep（氣球）為目標；不要誤選隊友/其他塔
        for (ent, pos, fac, _c) in (&entities, &positions, &factions, &creeps).join() {
            if fac.team_id == my_team { continue; }
            let dx = pos.0.x - center.x;
            let dy = pos.0.y - center.y;
            let d2 = dx * dx + dy * dy;
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

    fn play_vfx(&mut self, id: RStr<'_>, at: Vec2f) {
        log::debug!("[scripting] play_vfx id={} at=({},{})", id.as_str(), at.x, at.y);
    }

    fn play_sfx(&mut self, id: RStr<'_>, at: Vec2f) {
        log::debug!("[scripting] play_sfx id={} at=({},{})", id.as_str(), at.x, at.y);
    }

    // ---------------- RNG ----------------

    fn rand_f32(&mut self) -> f32 {
        // rand_pcg 0.9: gen_range for f32 is inclusive of low, exclusive of high
        self.rng.gen_range(0.0_f32..1.0_f32)
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

    fn sum_stat(&self, e: EntityHandle, stat_key: StatKey) -> f32 {
        let Some(ent) = Self::handle_to_entity(e) else { return 0.0 };
        self.world.read_resource::<BuffStore>().sum_add(ent, stat_key)
    }

    fn product_stat(&self, e: EntityHandle, stat_key: StatKey) -> f32 {
        let Some(ent) = Self::handle_to_entity(e) else { return 1.0 };
        self.world.read_resource::<BuffStore>().product_mult(ent, stat_key)
    }

    fn get_final_move_speed(&self, e: EntityHandle) -> f32 {
        let Some(ent) = Self::handle_to_entity(e) else { return 0.0 };
        let base = self.world.read_storage::<CProperty>().get(ent).map(|p| p.msd).unwrap_or(0.0);
        let store = self.world.read_resource::<BuffStore>();
        let is_b = self.world.read_storage::<IsBuilding>().get(ent).is_some();
        UnitStats::from_refs(&*store, is_b).final_move_speed(base, ent)
    }

    fn get_final_atk(&self, e: EntityHandle) -> f32 {
        let Some(ent) = Self::handle_to_entity(e) else { return 0.0 };
        let base = self.world.read_storage::<TAttack>()
            .get(ent).map(|t| t.atk_physic.v).unwrap_or(0.0);
        let store = self.world.read_resource::<BuffStore>();
        let is_b = self.world.read_storage::<IsBuilding>().get(ent).is_some();
        UnitStats::from_refs(&*store, is_b).final_atk(base, ent)
    }

    fn get_tower_upgrade(&self, e: EntityHandle, path: u8) -> u8 {
        let Some(ent) = Self::handle_to_entity(e) else { return 0 };
        let towers = self.world.read_storage::<Tower>();
        towers.get(ent)
            .and_then(|t| t.upgrade_levels.get(path as usize))
            .copied()
            .unwrap_or(0)
    }

    fn has_tower_flag(&self, e: EntityHandle, flag: RStr<'_>) -> bool {
        let Some(ent) = Self::handle_to_entity(e) else { return false };
        let towers = self.world.read_storage::<Tower>();
        towers.get(ent)
            .map(|t| t.upgrade_flags.iter().any(|f| f == flag.as_str()))
            .unwrap_or(false)
    }

    fn apply_tower_permanent_buff(&mut self, e: EntityHandle, buff_id: RStr<'_>, modifiers_json: RStr<'_>) {
        self.add_stat_buff(e, buff_id, f32::MAX, modifiers_json);
    }

    fn get_final_attack_range(&self, e: EntityHandle) -> f32 {
        let Some(ent) = Self::handle_to_entity(e) else { return 0.0 };
        let base = self.world.read_storage::<TAttack>()
            .get(ent).map(|t| t.range.v).unwrap_or(0.0);
        let store = self.world.read_resource::<BuffStore>();
        let is_b = self.world.read_storage::<IsBuilding>().get(ent).is_some();
        UnitStats::from_refs(&*store, is_b).final_attack_range(base, ent)
    }

    fn get_buff_remaining(&self, e: EntityHandle, buff_id: RStr<'_>) -> f32 {
        let Some(ent) = Self::handle_to_entity(e) else { return 0.0 };
        self.world.read_resource::<BuffStore>()
            .get(ent, buff_id.as_str())
            .map(|b| b.remaining)
            .unwrap_or(0.0)
    }

    fn current_mana(&self, e: EntityHandle) -> f32 {
        // 沒有 current_mana component — 目前回 max（視為永遠滿）。
        // 如果之後加 `ManaPool` component，這裡要改成讀 current。
        let Some(ent) = Self::handle_to_entity(e) else { return 0.0 };
        self.world.read_storage::<Hero>().get(ent).map(|h| h.get_max_mana()).unwrap_or(0.0)
    }

    fn spend_mana(&mut self, e: EntityHandle, amount: f32, ability_id: RStr<'_>) -> bool {
        let Some(ent) = Self::handle_to_entity(e) else { return false };
        // 目前沒有 mana storage，永遠視為成功；push 事件讓腳本 hook。
        self.world.write_resource::<ScriptEventQueue>()
            .push(ScriptEvent::SpentMana {
                caster: ent,
                cost: amount,
                ability_id: ability_id.as_str().to_string(),
            });
        true
    }

    fn restore_mana(&mut self, e: EntityHandle, amount: f32) {
        let Some(ent) = Self::handle_to_entity(e) else { return };
        self.world.write_resource::<ScriptEventQueue>()
            .push(ScriptEvent::ManaGained { e: ent, amount });
    }

    fn trigger_modifier_added(&mut self, e: EntityHandle, modifier_id: RStr<'_>) {
        let Some(ent) = Self::handle_to_entity(e) else { return };
        self.world.write_resource::<ScriptEventQueue>()
            .push(ScriptEvent::ModifierAdded {
                e: ent,
                modifier_id: modifier_id.as_str().to_string(),
            });
    }

    fn trigger_state_changed(&mut self, e: EntityHandle, state_id: RStr<'_>, active: bool) {
        let Some(ent) = Self::handle_to_entity(e) else { return };
        self.world.write_resource::<ScriptEventQueue>()
            .push(ScriptEvent::StateChanged {
                e: ent,
                state_id: state_id.as_str().to_string(),
                active,
            });
    }

    // ---------------- Dota 2 property 完整查詢 ----------------

    fn get_final_armor(&self, e: EntityHandle) -> f32 {
        let Some(ent) = Self::handle_to_entity(e) else { return 0.0 };
        let buffs = self.world.read_resource::<crate::ability_runtime::BuffStore>();
        let is_bldg = self.world.read_storage::<IsBuilding>().get(ent).is_some();
        let base = self.world.read_storage::<CProperty>()
            .get(ent).map(|c| c.def_physic).unwrap_or(0.0);
        crate::ability_runtime::UnitStats::from_refs(&*buffs, is_bldg).final_armor(base, ent)
    }

    fn get_final_magic_resist(&self, e: EntityHandle) -> f32 {
        let Some(ent) = Self::handle_to_entity(e) else { return 0.0 };
        let buffs = self.world.read_resource::<crate::ability_runtime::BuffStore>();
        let is_bldg = self.world.read_storage::<IsBuilding>().get(ent).is_some();
        let base = self.world.read_storage::<CProperty>()
            .get(ent).map(|c| c.def_magic).unwrap_or(0.0);
        crate::ability_runtime::UnitStats::from_refs(&*buffs, is_bldg).final_magic_resist(base, ent)
    }

    fn get_evasion_chance(&self, e: EntityHandle) -> f32 {
        let Some(ent) = Self::handle_to_entity(e) else { return 0.0 };
        let buffs = self.world.read_resource::<crate::ability_runtime::BuffStore>();
        let is_bldg = self.world.read_storage::<IsBuilding>().get(ent).is_some();
        crate::ability_runtime::UnitStats::from_refs(&*buffs, is_bldg).evasion_chance(ent)
    }

    fn get_miss_chance(&self, e: EntityHandle) -> f32 {
        let Some(ent) = Self::handle_to_entity(e) else { return 0.0 };
        let buffs = self.world.read_resource::<crate::ability_runtime::BuffStore>();
        let is_bldg = self.world.read_storage::<IsBuilding>().get(ent).is_some();
        crate::ability_runtime::UnitStats::from_refs(&*buffs, is_bldg).miss_chance(ent)
    }

    fn get_crit_chance(&self, e: EntityHandle) -> f32 {
        let Some(ent) = Self::handle_to_entity(e) else { return 0.0 };
        let buffs = self.world.read_resource::<crate::ability_runtime::BuffStore>();
        let is_bldg = self.world.read_storage::<IsBuilding>().get(ent).is_some();
        crate::ability_runtime::UnitStats::from_refs(&*buffs, is_bldg).crit(ent).0
    }

    fn get_crit_multiplier(&self, e: EntityHandle) -> f32 {
        let Some(ent) = Self::handle_to_entity(e) else { return 1.0 };
        let buffs = self.world.read_resource::<crate::ability_runtime::BuffStore>();
        let is_bldg = self.world.read_storage::<IsBuilding>().get(ent).is_some();
        crate::ability_runtime::UnitStats::from_refs(&*buffs, is_bldg).crit(ent).1
    }

    fn get_cooldown_mult(&self, e: EntityHandle) -> f32 {
        let Some(ent) = Self::handle_to_entity(e) else { return 1.0 };
        let buffs = self.world.read_resource::<crate::ability_runtime::BuffStore>();
        let is_bldg = self.world.read_storage::<IsBuilding>().get(ent).is_some();
        crate::ability_runtime::UnitStats::from_refs(&*buffs, is_bldg).cooldown_mult(ent)
    }

    fn is_building(&self, e: EntityHandle) -> bool {
        let Some(ent) = Self::handle_to_entity(e) else { return false };
        self.world.read_storage::<IsBuilding>().get(ent).is_some()
    }

    fn get_max_hp_bonus(&self, e: EntityHandle) -> f32 {
        let Some(ent) = Self::handle_to_entity(e) else { return 0.0 };
        let buffs = self.world.read_resource::<crate::ability_runtime::BuffStore>();
        let is_bldg = self.world.read_storage::<IsBuilding>().get(ent).is_some();
        crate::ability_runtime::UnitStats::from_refs(&*buffs, is_bldg).max_hp_bonus(ent)
    }

    fn get_hp_regen(&self, e: EntityHandle) -> f32 {
        let Some(ent) = Self::handle_to_entity(e) else { return 0.0 };
        let buffs = self.world.read_resource::<crate::ability_runtime::BuffStore>();
        let is_bldg = self.world.read_storage::<IsBuilding>().get(ent).is_some();
        crate::ability_runtime::UnitStats::from_refs(&*buffs, is_bldg).hp_regen(0.0, ent)
    }

    fn get_stat_bonus(&self, e: EntityHandle, key: StatKey) -> f32 {
        let Some(ent) = Self::handle_to_entity(e) else { return 0.0 };
        self.world.read_resource::<crate::ability_runtime::BuffStore>()
            .sum_add(ent, key)
    }

    fn deal_damage_splash(
        &mut self,
        at: Vec2f,
        radius: f32,
        damage: f32,
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

