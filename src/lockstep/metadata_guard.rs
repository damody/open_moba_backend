#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::{Path, PathBuf};
    use syn::visit::{self, Visit};

    const TRACE_METADATA_TOKENS: &[&str] = &[
        "server_receive_tick",
        "server_drain_tick",
        "server_queue_us",
        "submit_start_us",
        "submit_done_us",
        "client_receive_us",
        "origin_kind",
        "origin_us",
        "send_lockstep_input_us",
        "client_receive_tickbatch_us",
        "game_forward_to_sim_us",
        "extract_data_for_render_us",
        "InputOriginKind",
        "LockstepInputMsg",
        "LockstepTickInput",
        "TickBatchInput",
        "AppliedInputMeta",
        "applied_input_meta",
    ];

    const TRACE_METADATA_ALLOWED_FILES: &[&str] = &[
        "omb/src/lockstep/input_buffer.rs",
        "omb/src/lockstep/tick_broadcaster.rs",
        "omb/src/lockstep/metadata_guard.rs",
        "omb/src/transport/kcp_transport.rs",
        "omoba-core/src/generated/game.rs",
        "omoba-core/src/runtime/native/snapshot.rs",
        "proto/game.proto",
    ];

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("omb lives under the repo root")
            .to_path_buf()
    }

    fn normalized_relative_path(root: &Path, path: &Path) -> String {
        path.strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    }

    fn collect_files(dir: &Path, extensions: &[&str], out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_files(&path, extensions, out);
                continue;
            }

            let ext = path
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or_default();
            if extensions.iter().any(|wanted| *wanted == ext) {
                out.push(path);
            }
        }
    }

    // Evaluate only what is known in a non-test build. Unknown production
    // features stay conservative: any(test, feature="x") is not test-only.
    fn production_cfg(meta: &syn::Meta) -> Option<bool> {
        match meta {
            syn::Meta::Path(path) if path.is_ident("test") => Some(false),
            syn::Meta::List(list) => {
                let args = list.parse_args_with(
                    syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
                ).ok()?;
                let values: Vec<_> = args.iter().map(production_cfg).collect();
                if list.path.is_ident("all") {
                    if values.contains(&Some(false)) { Some(false) }
                    else if values.iter().all(|value| *value == Some(true)) { Some(true) }
                    else { None }
                } else if list.path.is_ident("any") {
                    if values.contains(&Some(true)) { Some(true) }
                    else if values.iter().all(|value| *value == Some(false)) { Some(false) }
                    else { None }
                } else if list.path.is_ident("not") && values.len() == 1 {
                    values[0].map(|value| !value)
                } else { None }
            }
            _ => None,
        }
    }

    fn test_only(attrs: &[syn::Attribute]) -> bool {
        attrs.iter().any(|attr| attr.path().is_ident("test") ||
            (attr.path().is_ident("cfg") && attr.parse_args::<syn::Meta>()
                .ok().is_some_and(|meta| production_cfg(&meta) == Some(false))))
    }

    fn item_attrs(item: &syn::Item) -> &[syn::Attribute] {
        match item {
            syn::Item::Const(item) => &item.attrs, syn::Item::Enum(item) => &item.attrs,
            syn::Item::ExternCrate(item) => &item.attrs, syn::Item::Fn(item) => &item.attrs,
            syn::Item::ForeignMod(item) => &item.attrs, syn::Item::Impl(item) => &item.attrs,
            syn::Item::Macro(item) => &item.attrs, syn::Item::Mod(item) => &item.attrs,
            syn::Item::Static(item) => &item.attrs, syn::Item::Struct(item) => &item.attrs,
            syn::Item::Trait(item) => &item.attrs, syn::Item::TraitAlias(item) => &item.attrs,
            syn::Item::Type(item) => &item.attrs, syn::Item::Union(item) => &item.attrs,
            syn::Item::Use(item) => &item.attrs, _ => &[],
        }
    }

    #[derive(Default)]
    struct ProductionTokens(Vec<String>);
    impl<'ast> Visit<'ast> for ProductionTokens {
        fn visit_item(&mut self, item: &'ast syn::Item) {
            if test_only(item_attrs(item)) { return; }
            if let syn::Item::Verbatim(tokens) = item { self.0.push(tokens.to_string()); }
            visit::visit_item(self, item);
        }
        fn visit_local(&mut self, local: &'ast syn::Local) {
            if !test_only(&local.attrs) { visit::visit_local(self, local); }
        }
        fn visit_ident(&mut self, ident: &'ast syn::Ident) { self.0.push(ident.to_string()); }
        fn visit_lit_str(&mut self, value: &'ast syn::LitStr) { self.0.push(value.value()); }
        fn visit_macro(&mut self, value: &'ast syn::Macro) { self.0.push(value.tokens.to_string()); }
    }

    fn module_references(items: &[syn::Item], attribute_base: &Path, module_dir: &Path,
        inherited_test: bool, known: &BTreeSet<PathBuf>, tests: &mut BTreeSet<PathBuf>,
        production: &mut BTreeSet<PathBuf>) {
        for item in items {
            let syn::Item::Mod(module) = item else { continue; };
            let is_test = inherited_test || test_only(&module.attrs);
            if let Some((_, children)) = &module.content {
                let child_dir = module_dir.join(module.ident.to_string());
                module_references(children, &child_dir, &child_dir, is_test, known, tests, production);
                continue;
            }
            let explicit = module.attrs.iter().find_map(|attr| {
                if !attr.path().is_ident("path") { return None; }
                let syn::Meta::NameValue(meta) = &attr.meta else { return None; };
                let syn::Expr::Lit(value) = &meta.value else { return None; };
                if let syn::Lit::Str(value) = &value.lit { Some(value.value()) } else { None }
            });
            let candidates = if let Some(path) = explicit { vec![attribute_base.join(path)] }
                else { vec![module_dir.join(format!("{}.rs", module.ident)),
                    module_dir.join(module.ident.to_string()).join("mod.rs")] };
            for candidate in candidates {
                let path = fs::canonicalize(&candidate).unwrap_or(candidate);
                if known.contains(&path) {
                    if is_test { tests.insert(path); } else { production.insert(path); }
                    break;
                }
            }
        }
    }

    fn test_module_files(docs: &[(PathBuf, syn::File)]) -> BTreeSet<PathBuf> {
        let known: BTreeSet<_> = docs.iter().map(|(path, _)| path.clone()).collect();
        let intrinsic: BTreeSet<_> = docs.iter().filter(|(_, file)| test_only(&file.attrs))
            .map(|(path, _)| path.clone()).collect();
        let mut excluded = intrinsic.clone();
        for _ in 0..=docs.len() {
            let (mut tests, mut production) = (BTreeSet::new(), BTreeSet::new());
            for (path, file) in docs {
                let parent = path.parent().expect("source parent");
                let stem = path.file_stem().unwrap().to_string_lossy();
                let module_dir = if matches!(stem.as_ref(), "lib" | "main" | "mod") { parent.to_path_buf() }
                    else { parent.join(stem.as_ref()) };
                module_references(&file.items, parent, &module_dir, excluded.contains(path),
                    &known, &mut tests, &mut production);
            }
            let next: BTreeSet<_> = intrinsic.union(&tests.difference(&production).cloned().collect())
                .cloned().collect();
            if next == excluded { return next; }
            excluded = next;
        }
        panic!("test module provenance did not converge");
    }

    fn production_contents(files: Vec<PathBuf>) -> Vec<(PathBuf, String)> {
        let mut docs = Vec::new();
        for path in &files {
            if path.extension().is_some_and(|ext| ext == "rs") {
                let contents = fs::read_to_string(path).expect("read metadata guard source");
                let parsed = syn::parse_file(&contents).unwrap_or_else(|error|
                    panic!("metadata guard cannot parse {}: {error}", path.display()));
                docs.push((fs::canonicalize(path).expect("canonical source"), parsed));
            }
        }
        let excluded = test_module_files(&docs);
        files.into_iter().filter_map(|path| {
            if path.extension().is_some_and(|ext| ext == "rs") {
                let canonical = fs::canonicalize(&path).expect("canonical source");
                if excluded.contains(&canonical) { return None; }
                let file = &docs.iter().find(|(known, _)| *known == canonical).unwrap().1;
                let mut tokens = ProductionTokens::default();tokens.visit_file(file);
                Some((path, tokens.0.join("\n")))
            } else {
                let contents = fs::read_to_string(&path).expect("read metadata guard source");
                Some((path, contents))
            }
        }).collect()
    }

    #[test]
    fn phase_trace_metadata_stays_on_wire_edge_or_client_trace_path() {
        let root = repo_root();
        let mut files = Vec::new();
        for (relative, extensions) in [
            ("omb/src", &["rs"][..]),
            ("omoba-core/src", &["rs"][..]),
            ("omoba-sim/src", &["rs"][..]),
            ("scripts/base_content/src", &["rs"][..]),
            ("proto", &["proto"][..]),
        ] {
            collect_files(&root.join(relative), extensions, &mut files);
        }

        let mut violations = Vec::new();
        for (path, contents) in production_contents(files) {
            let relative = normalized_relative_path(&root, &path);
            if TRACE_METADATA_ALLOWED_FILES.contains(&relative.as_str()) {
                continue;
            }

            for token in TRACE_METADATA_TOKENS {
                if contents.contains(token) {
                    violations.push(format!("{relative}: contains {token}"));
                }
            }
        }

        assert!(
            violations.is_empty(),
            "lockstep phase trace metadata must not enter gameplay ECS/state paths:\n{}",
            violations.join("\n")
        );
    }

    #[test]
    fn input_id_does_not_enter_gameplay_state_or_scripts() {
        let root = repo_root();
        let mut files = Vec::new();
        for relative in [
            "omb/src/ability_runtime",
            "omb/src/comp",
            "omb/src/item",
            "omb/src/scripting",
            "omb/src/state",
            "omb/src/tick",
            "omb/src/vision",
            "omoba-sim/src",
            "scripts/base_content/src",
        ] {
            collect_files(&root.join(relative), &["rs"], &mut files);
        }

        let mut violations = Vec::new();
        for (path, contents) in production_contents(files) {
            if contents.contains("input_id") {
                violations.push(normalized_relative_path(&root, &path));
            }
        }

        assert!(
            violations.is_empty(),
            "input_id is wire/trace metadata and must not enter gameplay state or scripts:\n{}",
            violations.join("\n")
        );
    }

    #[test]
    fn production_scanner_keeps_identifiers_strings_and_macros_but_excludes_test_items() {
        let file = syn::parse_file(r#"
            fn real() { let input_id = 1; log!("server_queue_us"); }
            #[cfg(test)] mod tests { struct OnlyTest { client_receive_us: u64 } }
            #[test] fn fixture() { let origin_us = 1; }
        "#).unwrap();
        let mut tokens = ProductionTokens::default();tokens.visit_file(&file);
        let text = tokens.0.join("\n");
        assert!(text.contains("input_id") && text.contains("server_queue_us"));
        assert!(!text.contains("client_receive_us") && !text.contains("origin_us"));
    }

    #[test]
    fn production_cfg_keeps_unknown_feature_alternatives() {
        let evaluate = |text: &str| production_cfg(&syn::parse_str(text).unwrap());
        assert_eq!(evaluate("test"), Some(false));
        assert_eq!(evaluate("all(test, feature=\"x\")"), Some(false));
        assert_eq!(evaluate("any(test, feature=\"x\")"), None);
        assert_eq!(evaluate("not(test)"), Some(true));
    }

    #[test]
    fn external_test_provenance_follows_paths_and_preserves_shared_production_modules() {
        let doc = |path: &str, text: &str| (PathBuf::from(path), syn::parse_file(text).unwrap());
        let docs = vec![doc("fixture/lib.rs", r#"
            #[cfg(test)] #[path="suite.rs"] mod tests;
            #[path="shared.rs"] mod production;
            #[cfg(test)] #[path="shared.rs"] mod shared_fixture;
        "#), doc("fixture/suite.rs", "mod nested;"),
            doc("fixture/suite/nested.rs", "struct Receipt { input_id: u64 }"),
            doc("fixture/shared.rs", "struct Gameplay { input_id: u64 }")];
        let excluded = test_module_files(&docs);
        assert!(excluded.contains(Path::new("fixture/suite.rs")));
        assert!(excluded.contains(Path::new("fixture/suite/nested.rs")));
        assert!(!excluded.contains(Path::new("fixture/shared.rs")));
    }
}
