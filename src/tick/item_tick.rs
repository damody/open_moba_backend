use specs::{shred, Entities, Join, Read, ReadExpect, ReadStorage, SystemData, Write, WriteStorage};
use crate::comp::*;

#[derive(SystemData)]
pub struct ItemRead<'a> {
    entities: Entities<'a>,
    dt: Read<'a, DeltaTime>,
    item_reg: ReadExpect<'a, crate::item::ItemRegistry>,
}

#[derive(SystemData)]
pub struct ItemWrite<'a> {
    inventories: WriteStorage<'a, Inventory>,
    effects: WriteStorage<'a, ItemEffects>,
    properties: WriteStorage<'a, CProperty>,
    attacks: WriteStorage<'a, TAttack>,
}

#[derive(Default)]
pub struct Sys;

impl<'a> System<'a> for Sys {
    type SystemData = (ItemRead<'a>, ItemWrite<'a>);

    const NAME: &'static str = "item";

    fn run(_job: &mut Job<Self>, (tr, mut tw): Self::SystemData) {
        // TODO Phase 1[c]: drop conversion when item cooldowns go Fixed32-native.
        let dt = tr.dt.0.to_f32_for_render();

        // 1) 遞減所有 inventory CD
        for inv in (&mut tw.inventories).join() {
            for slot in inv.slots.iter_mut() {
                if let Some(inst) = slot {
                    if inst.cooldown_remaining > 0.0 {
                        inst.cooldown_remaining = (inst.cooldown_remaining - dt).max(0.0);
                    }
                }
            }
        }

        // 2) 若 ItemEffects.dirty → 重算 bonus 並同步到 CProperty/TAttack
        // 先收集需要重算的 entities
        let dirty_ents: Vec<specs::Entity> = (&tr.entities, &tw.effects)
            .join()
            .filter(|(_, eff)| eff.dirty)
            .map(|(e, _)| e)
            .collect();

        for e in dirty_ents {
            // 聚合裝備屬性
            let mut sum_atk = 0.0f32;
            let mut sum_hp = 0.0f32;
            let mut sum_mp = 0.0f32;
            let mut sum_ms = 0.0f32;
            let mut sum_armor = 0.0f32;
            let mut sum_mp_regen = 0.0f32;
            if let Some(inv) = tw.inventories.get(e) {
                for slot in inv.slots.iter() {
                    if let Some(inst) = slot {
                        if let Some(cfg) = tr.item_reg.get(&inst.item_id) {
                            sum_atk += cfg.bonus.atk;
                            sum_hp += cfg.bonus.hp;
                            sum_mp += cfg.bonus.mp;
                            sum_ms += cfg.bonus.ms;
                            sum_armor += cfg.bonus.armor;
                            sum_mp_regen += cfg.bonus.mp_regen;
                        }
                    }
                }
            }

            let (applied_atk, applied_hp, applied_ms, applied_armor) = {
                if let Some(eff) = tw.effects.get(e) {
                    (eff.applied_atk, eff.applied_hp, eff.applied_ms, eff.applied_armor)
                } else {
                    (0.0, 0.0, 0.0, 0.0)
                }
            };

            if let Some(prop) = tw.properties.get_mut(e) {
                prop.mhp = prop.mhp - applied_hp + sum_hp;
                prop.hp = prop.hp.min(prop.mhp).max(1.0);
                prop.msd = prop.msd - applied_ms + sum_ms;
                prop.def_physic = prop.def_physic - applied_armor + sum_armor;
            }
            if let Some(atk) = tw.attacks.get_mut(e) {
                let cur = atk.atk_physic.val();
                atk.atk_physic = Vf32::new(cur - applied_atk + sum_atk);
            }

            if let Some(eff) = tw.effects.get_mut(e) {
                eff.bonus_atk = sum_atk;
                eff.bonus_hp = sum_hp;
                eff.bonus_mp = sum_mp;
                eff.bonus_ms = sum_ms;
                eff.bonus_armor = sum_armor;
                eff.bonus_mp_regen = sum_mp_regen;
                eff.applied_atk = sum_atk;
                eff.applied_hp = sum_hp;
                eff.applied_ms = sum_ms;
                eff.applied_armor = sum_armor;
                eff.dirty = false;
            }

            log::info!(
                "ItemEffects 重算 entity={:?}: atk+{} hp+{} ms+{} armor+{} mp+{}",
                e, sum_atk, sum_hp, sum_ms, sum_armor, sum_mp
            );
        }
    }
}
