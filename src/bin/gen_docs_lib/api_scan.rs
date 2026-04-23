//! Parse script-abi source files using syn to extract ApiMethod lists and
//! StatKey tables for the reference section.

use crate::lib::model::{ApiGroup, ApiMethod, ApiSpec, StatKey, StatSection};
use anyhow::{Context, Result};
use std::path::Path;
use syn::{File, Item, TraitItem, TraitItemFn};

pub fn scan(abi_src_dir: &Path) -> Result<ApiSpec> {
    let script_src = std::fs::read_to_string(abi_src_dir.join("script.rs"))
        .with_context(|| format!("reading {}/script.rs", abi_src_dir.display()))?;
    let ability_src = std::fs::read_to_string(abi_src_dir.join("ability.rs"))
        .with_context(|| format!("reading {}/ability.rs", abi_src_dir.display()))?;
    let world_src = std::fs::read_to_string(abi_src_dir.join("world.rs"))
        .with_context(|| format!("reading {}/world.rs", abi_src_dir.display()))?;
    let stat_src = std::fs::read_to_string(abi_src_dir.join("stat_keys.rs"))
        .with_context(|| format!("reading {}/stat_keys.rs", abi_src_dir.display()))?;

    let unit_hooks = scan_trait(&script_src, "UnitScript", ApiGroup::UnitHook)?;
    let ability_hooks = scan_trait(&ability_src, "AbilityScript", ApiGroup::AbilityHook)?;
    let world_methods = scan_world(&world_src)?;
    let stat_keys = scan_stat_keys(&stat_src)?;

    Ok(ApiSpec { unit_hooks, ability_hooks, world_methods, stat_keys })
}

pub fn scan_trait(src: &str, trait_name: &str, group: ApiGroup) -> Result<Vec<ApiMethod>> {
    let file: File = syn::parse_str(src).context("parse trait file")?;
    let mut out = Vec::new();
    for item in &file.items {
        if let Item::Trait(t) = item {
            if t.ident == trait_name {
                for ti in &t.items {
                    if let TraitItem::Fn(f) = ti {
                        out.push(method_from_trait_item(f, group, None));
                    }
                }
            }
        }
    }
    Ok(out)
}

fn method_from_trait_item(f: &TraitItemFn, group: ApiGroup, sub: Option<String>) -> ApiMethod {
    ApiMethod {
        name: f.sig.ident.to_string(),
        signature: render_sig(&f.sig),
        doc: extract_doc(&f.attrs),
        group,
        sub_group: sub,
    }
}

fn render_sig(sig: &syn::Signature) -> String {
    use quote::ToTokens;
    sig.to_token_stream().to_string()
}

fn extract_doc(attrs: &[syn::Attribute]) -> String {
    let mut lines = Vec::new();
    for a in attrs {
        if a.path().is_ident("doc") {
            if let syn::Meta::NameValue(nv) = &a.meta {
                if let syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. }) = &nv.value {
                    let v = s.value();
                    lines.push(v.trim_start().to_string());
                }
            }
        }
    }
    lines.join("\n")
}

pub fn scan_world(src: &str) -> Result<Vec<ApiMethod>> {
    use regex::Regex;
    let file: File = syn::parse_str(src).context("parse world.rs")?;
    let headers: Vec<(usize, String)> = {
        let re = Regex::new(r"^\s*//\s*----\s*(.+?)\s*----").unwrap();
        src.lines().enumerate()
            .filter_map(|(i, l)| re.captures(l).map(|c| (i + 1, c[1].trim().to_string())))
            .collect()
    };
    let pick_header = |line: usize| -> Option<String> {
        headers.iter().rev().find(|(h, _)| *h <= line).map(|(_, n)| n.clone())
    };
    let group_of = |hdr: &str| -> ApiGroup {
        let l = hdr.to_ascii_lowercase();
        if l.contains("query") { ApiGroup::WorldQuery }
        else if l.contains("mutate") { ApiGroup::WorldMutate }
        else if l.contains("tower") || l.contains("單位屬性") { ApiGroup::WorldTower }
        else if l.contains("rng") || l.contains("deterministic") { ApiGroup::WorldRng }
        else if l.contains("log") { ApiGroup::WorldLog }
        else if l.contains("vfx") || l.contains("side effect") { ApiGroup::WorldVfx }
        else { ApiGroup::WorldStats }
    };

    let mut out = Vec::new();
    for item in &file.items {
        if let Item::Trait(t) = item {
            if t.ident == "GameWorld" {
                for ti in &t.items {
                    if let TraitItem::Fn(f) = ti {
                        use syn::spanned::Spanned;
                        let line = f.span().start().line;
                        let hdr = pick_header(line);
                        let grp = hdr.as_deref().map(group_of).unwrap_or(ApiGroup::WorldStats);
                        out.push(method_from_trait_item(f, grp, hdr));
                    }
                }
            }
        }
    }
    Ok(out)
}

pub fn scan_stat_keys(src: &str) -> Result<Vec<StatKey>> {
    use regex::Regex;
    let file: File = syn::parse_str(src).context("parse stat_keys.rs")?;

    let sec_re = Regex::new(r"^//\s*SECTION\s+(\d)").unwrap();
    let sec_lines: Vec<(usize, StatSection)> = src.lines().enumerate()
        .filter_map(|(i, l)| sec_re.captures(l).and_then(|c| match &c[1] {
            "1" => Some((i + 1, StatSection::All)),
            "2" => Some((i + 1, StatSection::NonBuilding)),
            "3" => Some((i + 1, StatSection::Visual)),
            _ => None,
        }))
        .collect();
    let sub_re = Regex::new(r"^\s*//\s*----\s*(.+?)\s*----").unwrap();
    let sub_headers: Vec<(usize, String)> = src.lines().enumerate()
        .filter_map(|(i, l)| sub_re.captures(l).map(|c| (i + 1, c[1].trim().to_string())))
        .collect();

    let pick_section = |line: usize| -> StatSection {
        sec_lines.iter().rev().find(|(h, _)| *h <= line).map(|(_, s)| *s).unwrap_or(StatSection::All)
    };
    let pick_sub = |line: usize| -> Option<String> {
        sub_headers.iter().rev().find(|(h, _)| *h <= line).map(|(_, n)| n.clone())
    };

    let mut out = Vec::new();
    for item in &file.items {
        if let Item::Const(c) = item {
            use syn::spanned::Spanned;
            let line = c.span().start().line;
            let name = c.ident.to_string();
            let value = match &*c.expr {
                syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. }) => s.value(),
                syn::Expr::Path(_) => continue,
                _ => continue,
            };
            out.push(StatKey {
                const_name: name,
                string_value: value,
                doc: extract_doc(&c.attrs),
                section: pick_section(line),
                sub_group: pick_sub(line),
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAKE: &str = r#"
        pub trait UnitScript: Send + Sync {
            /// Called once when the entity is spawned.
            fn on_spawn(&self, _e: EntityHandle, _w: &mut GameWorldDyn<'_>) {}
            /// Called every tick.
            /// `dt` is the tick delta in seconds.
            fn on_tick(&self, _e: EntityHandle, _dt: f32, _w: &mut GameWorldDyn<'_>) {}
        }
    "#;

    #[test]
    fn extracts_unit_hooks_with_docs() {
        let hooks = scan_trait(FAKE, "UnitScript", ApiGroup::UnitHook).unwrap();
        assert_eq!(hooks.len(), 2);
        assert_eq!(hooks[0].name, "on_spawn");
        assert!(hooks[0].doc.contains("spawned"));
        assert_eq!(hooks[1].name, "on_tick");
        assert!(hooks[1].doc.contains("tick delta"));
        assert!(hooks[0].signature.contains("on_spawn"));
    }
}
