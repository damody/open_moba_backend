//! `WorldAdapter` — implements `omb_script_abi::GameWorld` over `&mut specs::World`.
//!
//! Only alive during the serial script-dispatch stage (E1), so it holds
//! `&mut World` exclusively; no locks needed.
//!
//! Adds components/methods here when scripts need more surface area. Keep the
//! surface as small as the PoC-1 (and subsequent PoCs) actually needs.

use abi_stable::std_types::{RNone, ROption, RSome, RStr, RVec};
use omb_script_abi::{
    types::{DamageKind, EntityHandle, Vec2f},
    world::GameWorld,
};
use rand::{Rng, SeedableRng};
use rand_pcg::Pcg64Mcg;
use specs::{Entity, Join, World, WorldExt};
use specs::world::Generation;

use crate::comp::{CProperty, Faction, Pos, Unit};

/// Host-side adapter. Created fresh for each `run_script_dispatch` call.
///
/// `log_str_scratch` is needed because `log_info(&self, ...)` is `&self`
/// but we format into `&mut String`; a `RefCell` avoids that.
pub struct WorldAdapter<'a> {
    pub world: &'a mut World,
    pub rng: Pcg64Mcg,
}

impl<'a> WorldAdapter<'a> {
    pub fn new(world: &'a mut World, seed: u64) -> Self {
        Self {
            world,
            rng: Pcg64Mcg::seed_from_u64(seed),
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
        log::debug!("[scripting] spawn_projectile (stub)");
        EntityHandle::INVALID
    }

    fn despawn(&mut self, e: EntityHandle) {
        let Some(ent) = Self::handle_to_entity(e) else { return };
        let _ = self.world.entities().delete(ent);
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

