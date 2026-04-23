//! Merge DllData + EntityData + ApiSpec + ImplEntry list into final Catalog.

use crate::lib::coverage::ImplEntry;
use crate::lib::dll::DllData;
use crate::lib::entity::EntityData;
use crate::lib::model::{
    ApiSpec, BuildMeta, Catalog, UnitEntry, UnitKind, Warning,
};
use std::collections::HashMap;

pub fn merge(
    dll: DllData,
    entity: EntityData,
    api: ApiSpec,
    impls: Vec<ImplEntry>,
    warnings: Vec<Warning>,
    meta: BuildMeta,
) -> Catalog {
    // Build impl lookup keyed by resolved id.
    let mut by_id: HashMap<String, ImplEntry> = HashMap::new();
    for i in impls {
        let key = i.id.clone().unwrap_or_else(|| snake(&i.self_ty));
        by_id.insert(key, i);
    }

    let mut units: Vec<UnitEntry> = Vec::new();

    // 1. DLL-provided units (towers + Unknown)
    for u in dll.units {
        let imp = by_id.remove(&u.id);
        let (overrides, world_calls, src) = match imp {
            Some(i) => (i.overrides, i.world_calls, Some(i.source_file)),
            None => (Vec::new(), Default::default(), None),
        };
        let label = u.tower.as_ref().map(|t| t.label.clone());
        units.push(UnitEntry {
            id: u.id,
            kind: u.kind,
            label,
            tower: u.tower,
            hero: None,
            creep: None,
            abilities: Vec::new(),
            overrides,
            world_calls,
            source_file: src,
        });
    }

    // 2. Heroes from entity.json
    for (id, h) in entity.heroes {
        let imp = by_id.remove(&id);
        let (overrides, world_calls, src) = match imp {
            Some(i) => (i.overrides, i.world_calls, Some(i.source_file)),
            None => (Vec::new(), Default::default(), None),
        };
        units.push(UnitEntry {
            id: id.clone(),
            kind: UnitKind::Hero,
            label: Some(h.name.clone()),
            tower: None,
            abilities: h.abilities.clone(),
            hero: Some(h),
            creep: None,
            overrides,
            world_calls,
            source_file: src,
        });
    }

    // 3. Creeps from entity.json
    for (id, c) in entity.creeps {
        let imp = by_id.remove(&id);
        let (overrides, world_calls, src) = match imp {
            Some(i) => (i.overrides, i.world_calls, Some(i.source_file)),
            None => (Vec::new(), Default::default(), None),
        };
        units.push(UnitEntry {
            id: id.clone(),
            kind: UnitKind::Creep,
            label: Some(c.name.clone()),
            tower: None,
            abilities: c.abilities.clone(),
            hero: None,
            creep: Some(c),
            overrides,
            world_calls,
            source_file: src,
        });
    }

    // 4. Remaining impls → warnings (orphan UnitScript; AbilityScript expected to
    //    attach to abilities list, not units — don't warn for those).
    let mut warnings = warnings;
    for (k, i) in by_id {
        if i.trait_name == "UnitScript" {
            warnings.push(Warning {
                source: i.source_file.clone(),
                message: format!(
                    "orphan UnitScript impl for {} (id={}) not referenced by DLL manifest or entity.json",
                    i.self_ty, k),
            });
        }
    }

    Catalog {
        units,
        abilities: dll.abilities,
        api,
        warnings,
        meta,
    }
}

fn snake(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 { out.push('_'); }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}
