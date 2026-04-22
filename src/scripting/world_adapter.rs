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
    types::{DamageKind, EntityHandle, PathSpec, ProjectileSpec, Vec2f},
    world::GameWorld,
};
use rand::{Rng, SeedableRng};
use rand_pcg::Pcg64Mcg;
use serde_json::json;
use specs::{Builder, Entity, Join, World, WorldExt};
use specs::world::Generation;

use crate::comp::*;
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

    fn add_buff(&mut self, _target: EntityHandle, buff_id: RStr<'_>, _duration: f32) {
        log::debug!("[scripting] add_buff (stub) id={}", buff_id.as_str());
    }

    fn remove_buff(&mut self, _target: EntityHandle, buff_id: RStr<'_>) {
        log::debug!("[scripting] remove_buff (stub) id={}", buff_id.as_str());
    }

    fn spawn_projectile(
        &mut self,
        _from: Vec2f,
        _to: EntityHandle,
        _speed: f32,
        _dmg: f32,
        _owner: EntityHandle,
    ) -> EntityHandle {
        log::debug!("[scripting] spawn_projectile (stub; use spawn_projectile_ex)");
        EntityHandle::INVALID
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
            })
            .build();

        let flight_time_ms: u64 = (flight_time_s * 1000.0).max(1.0) as u64;
        let kind_str = spec.kind_tag.as_str();
        let pjs = json!({
            "id": e.id(),
            "source_id": owner_ent.id(),
            "target_id": target_id_out,
            "start_pos": { "x": from_vek.x, "y": from_vek.y },
            "end_pos":   { "x": end_pos_vek.x, "y": end_pos_vek.y },
            "move_speed": spec.speed,
            "flight_time_ms": flight_time_ms,
            "damage": spec.damage,
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

    fn add_slow_buff(&mut self, target: EntityHandle, factor: f32, duration: f32) {
        let Some(ent) = Self::handle_to_entity(target) else { return };
        let mut buffs = self.world.write_storage::<SlowBuff>();
        let existing = buffs.get(ent).copied();
        let (f, r) = match existing {
            Some(b) => (b.factor.min(factor), b.remaining.max(duration)),
            None => (factor, duration),
        };
        let _ = buffs.insert(ent, SlowBuff { factor: f, remaining: r });
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
        log::info!("[script] {}", msg.as_str());
    }
    fn log_warn(&self, msg: RStr<'_>) {
        log::warn!("[script] {}", msg.as_str());
    }
    fn log_error(&self, msg: RStr<'_>) {
        log::error!("[script] {}", msg.as_str());
    }
}

