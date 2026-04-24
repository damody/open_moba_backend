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
    let raw = sig.to_token_stream().to_string();
    // quote 用 space 分隔每個 token；塞回接近手寫的樣子
    raw.replace(" < ", "<")
        .replace(" > ", ">")
        .replace(" <", "<")
        .replace("> ", ">")
        .replace(" , ", ", ")
        .replace(" ,", ",")
        .replace(" ( ", "(")
        .replace(" (", "(")
        .replace(" ) ", ") ")
        .replace(" )", ")")
        .replace(" :: ", "::")
        .replace(" : ", ": ")
        .replace("& ", "&")
        .replace("' ", "'")
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
    let file: File = syn::parse_str(src).context("parse world.rs")?;
    let headers: Vec<(usize, String)> = {
        let dash_re = regex::Regex::new(r"^\s*//\s*----\s*(.+?)\s*----").unwrap();
        let eq_start = regex::Regex::new(r"^\s*//\s*={6,}\s*$").unwrap();
        let eq_title = regex::Regex::new(r"^\s*//\s*(.+?)\s*$").unwrap();
        let lines: Vec<&str> = src.lines().collect();
        let mut out: Vec<(usize, String)> = Vec::new();
        let mut i = 0;
        while i < lines.len() {
            let l = lines[i];
            if let Some(c) = dash_re.captures(l) {
                out.push((i + 1, c[1].trim().to_string()));
            } else if eq_start.is_match(l) {
                // 往下找 title 行 + 收尾 `====`；title 是 `// <text>`（非純 `=`）
                if i + 2 < lines.len() {
                    let next = lines[i + 1];
                    let close = lines[i + 2];
                    if eq_start.is_match(close) {
                        if let Some(t) = eq_title.captures(next) {
                            let title = t[1].trim().to_string();
                            // 排除 title 行本身是 `====...`
                            if !title.chars().all(|c| c == '=' || c.is_whitespace()) {
                                out.push((i + 3, title));  // header 結束於第 3 行
                                i += 2;
                            }
                        }
                    }
                }
            }
            i += 1;
        }
        out
    };
    let pick_header = |line: usize| -> Option<String> {
        headers.iter().rev().find(|(h, _)| *h <= line).map(|(_, n)| n.clone())
    };
    let group_of = |hdr: &str| -> ApiGroup {
        let l = hdr.to_ascii_lowercase();
        if l.contains("query") || hdr.contains("查詢") { ApiGroup::WorldQuery }
        else if l.contains("mutate") { ApiGroup::WorldMutate }
        else if l.contains("tower") || hdr.contains("單位屬性") { ApiGroup::WorldTower }
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

pub fn scan_stat_keys(_src: &str) -> Result<Vec<StatKey>> {
    use omb_script_abi::stat_keys::{
        self as sk_mod, Aggregation, StatKey as StatKeyEnum, StatSection as StatSectionEnum,
    };

    let out = sk_mod::ALL
        .iter()
        .map(|&v| StatKey {
            const_name: format!("{:?}", v),
            string_value: v.as_str().to_string(),
            doc: String::new(),
            section: match v.section() {
                StatSectionEnum::All => StatSection::All,
                StatSectionEnum::NonBuilding => StatSection::NonBuilding,
                StatSectionEnum::Visual => StatSection::Visual,
            },
            sub_group: None,
            aggregation: match v.aggregation() {
                Aggregation::SumAdd => "SumAdd".to_string(),
                Aggregation::SumAddThenMul1Plus => "SumAddThenMul1Plus".to_string(),
                Aggregation::ProductMult => "ProductMult".to_string(),
                Aggregation::Chance => "Chance".to_string(),
                Aggregation::PassThrough => "PassThrough".to_string(),
            },
        })
        .collect();

    // Silence unused-import warnings on StatKeyEnum (kept for future per-variant doc hooks).
    let _ = std::marker::PhantomData::<StatKeyEnum>;
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

    #[test]
    fn scans_real_script_abi() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../scripts/script-abi/src");
        let spec = scan(&dir).unwrap();
        assert!(spec.unit_hooks.len() >= 15, "got only {} unit hooks", spec.unit_hooks.len());
        assert!(spec.ability_hooks.iter().any(|m| m.name == "execute"));
        assert!(spec.world_methods.iter().any(|m| m.name == "get_final_armor"));
        assert!(spec.stat_keys.iter().any(|k| k.const_name == "PreattackBonusDamage"));

        // Regression guard for I1: un-`----` sections must not end up as WorldLog
        let armor = spec.world_methods.iter().find(|m| m.name == "get_final_armor")
            .expect("get_final_armor should be present");
        assert!(
            !matches!(armor.group, crate::lib::model::ApiGroup::WorldLog),
            "get_final_armor got grouped as WorldLog (I1 regression); actual: {:?}", armor.group
        );

        let sum_stat = spec.world_methods.iter().find(|m| m.name == "sum_stat")
            .expect("sum_stat should be present");
        assert!(
            !matches!(sum_stat.group, crate::lib::model::ApiGroup::WorldLog),
            "sum_stat got grouped as WorldLog (I1 regression); actual: {:?}", sum_stat.group
        );
    }
}
