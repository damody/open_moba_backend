//! Walk base_content source files to detect `impl UnitScript for X` /
//! `impl AbilityScript for X` blocks and collect overridden methods + the
//! GameWorld method names called inside each impl.

use anyhow::{Context, Result};
use std::collections::{BTreeSet, HashSet};
use std::path::Path;
use syn::visit::Visit;
use syn::{File, Item, ImplItem, ImplItemFn};

#[derive(Debug, Clone)]
pub struct ImplEntry {
    pub self_ty: String,
    pub trait_name: String,
    pub overrides: Vec<String>,
    pub world_calls: BTreeSet<String>,
    pub unit_id: Option<String>,
    pub source_file: String,
}

pub fn scan_dir(dir: &Path, world_methods: &HashSet<String>) -> Result<Vec<ImplEntry>> {
    let mut out = Vec::new();
    for entry in walkdir(dir)? {
        let src = std::fs::read_to_string(&entry)
            .with_context(|| format!("reading {}", entry.display()))?;
        let rel = entry.strip_prefix(dir).unwrap_or(&entry).display().to_string();
        let more = scan_source(&src, &rel, world_methods)?;
        out.extend(more);
    }
    Ok(out)
}

fn walkdir(dir: &Path) -> Result<Vec<std::path::PathBuf>> {
    let mut out = Vec::new();
    fn inner(p: &Path, out: &mut Vec<std::path::PathBuf>) -> std::io::Result<()> {
        for e in std::fs::read_dir(p)? {
            let e = e?;
            let path = e.path();
            if path.is_dir() { inner(&path, out)?; }
            else if path.extension().and_then(|s| s.to_str()) == Some("rs") { out.push(path); }
        }
        Ok(())
    }
    inner(dir, &mut out)?;
    Ok(out)
}

pub fn scan_source(src: &str, rel: &str, world_methods: &HashSet<String>) -> Result<Vec<ImplEntry>> {
    let file: File = syn::parse_str(src).context("parse source")?;
    let mut out = Vec::new();
    for item in &file.items {
        if let Item::Impl(imp) = item {
            if let Some((_, path, _)) = &imp.trait_ {
                let last = match path.segments.last() {
                    Some(s) => s.ident.to_string(),
                    None => continue,
                };
                if last != "UnitScript" && last != "AbilityScript" {
                    continue;
                }
                let self_ty = quote_ty(&imp.self_ty);
                let mut entry = ImplEntry {
                    self_ty,
                    trait_name: last,
                    overrides: Vec::new(),
                    world_calls: BTreeSet::new(),
                    unit_id: None,
                    source_file: rel.to_string(),
                };
                for it in &imp.items {
                    if let ImplItem::Fn(f) = it {
                        let name = f.sig.ident.to_string();
                        if name == "unit_id" {
                            entry.unit_id = extract_string_return(f);
                        } else {
                            entry.overrides.push(name);
                        }
                        let mut v = CallVisitor {
                            receivers: &["world", "w", "_w"],
                            methods: world_methods,
                            found: &mut entry.world_calls,
                        };
                        v.visit_impl_item_fn(f);
                    }
                }
                out.push(entry);
            }
        }
    }
    Ok(out)
}

fn quote_ty(ty: &syn::Type) -> String {
    use quote::ToTokens;
    ty.to_token_stream().to_string().replace(' ', "")
}

fn extract_string_return(f: &ImplItemFn) -> Option<String> {
    struct Find(Option<String>);
    impl<'ast> Visit<'ast> for Find {
        fn visit_lit_str(&mut self, l: &'ast syn::LitStr) {
            if self.0.is_none() { self.0 = Some(l.value()); }
        }
    }
    let mut f2 = Find(None);
    f2.visit_block(&f.block);
    f2.0
}

struct CallVisitor<'a> {
    receivers: &'a [&'a str],
    methods: &'a HashSet<String>,
    found: &'a mut BTreeSet<String>,
}

impl<'a, 'ast> Visit<'ast> for CallVisitor<'a> {
    fn visit_expr_method_call(&mut self, m: &'ast syn::ExprMethodCall) {
        if let syn::Expr::Path(p) = &*m.receiver {
            if let Some(seg) = p.path.segments.last() {
                let name = seg.ident.to_string();
                if self.receivers.contains(&name.as_str()) {
                    let method = m.method.to_string();
                    if self.methods.contains(&method) {
                        self.found.insert(method);
                    }
                }
            }
        }
        syn::visit::visit_expr_method_call(self, m);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAKE: &str = r#"
        struct DartTower;
        impl UnitScript for DartTower {
            fn unit_id(&self) -> RStr<'_> { "dart".into() }
            fn on_spawn(&self, e: EntityHandle, w: &mut GameWorldDyn<'_>) {
                w.set_tower_atk(e, 10.0);
            }
            fn on_tick(&self, e: EntityHandle, _dt: f32, w: &mut GameWorldDyn<'_>) {
                let enemies = w.query_enemies_in_range(v2, 100.0, e);
                for t in enemies {
                    w.deal_damage(t, 5.0, DamageKind::Physical, RSome(e));
                }
            }
        }
    "#;

    #[test]
    fn detects_impl_and_world_calls() {
        let world_methods: HashSet<String> = ["set_tower_atk","query_enemies_in_range","deal_damage"]
            .iter().map(|s| s.to_string()).collect();
        let result = scan_source(FAKE, "fake.rs", &world_methods).unwrap();
        assert_eq!(result.len(), 1);
        let e = &result[0];
        assert_eq!(e.self_ty, "DartTower");
        assert_eq!(e.trait_name, "UnitScript");
        assert_eq!(e.unit_id.as_deref(), Some("dart"));
        assert!(e.overrides.contains(&"on_spawn".to_string()));
        assert!(e.overrides.contains(&"on_tick".to_string()));
        assert!(!e.overrides.contains(&"unit_id".to_string()));
        assert!(e.world_calls.contains("set_tower_atk"));
        assert!(e.world_calls.contains("query_enemies_in_range"));
        assert!(e.world_calls.contains("deal_damage"));
    }
}
