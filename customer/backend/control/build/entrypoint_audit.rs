use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use proc_macro2::{TokenStream, TokenTree};
use syn::visit::{self, Visit};

const REGISTRATION_METHODS: &[&str] = &[
    "route",
    "route_service",
    "nest",
    "nest_service",
    "merge",
    "fallback",
    "fallback_service",
    "method_not_allowed_fallback",
];

fn is_test(attributes: &[syn::Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        attribute.path().is_ident("cfg")
            && attribute
                .parse_args::<syn::Path>()
                .is_ok_and(|path| path.is_ident("test"))
    })
}

struct Audit<'a> {
    registry: bool,
    registry_path: &'a Path,
    module_directory: PathBuf,
    path_directory: PathBuf,
    seen: &'a mut BTreeSet<(PathBuf, bool)>,
    errors: Vec<String>,
}

impl Audit<'_> {
    fn tokens(&mut self, tokens: TokenStream) {
        let tokens: Vec<_> = tokens.into_iter().collect();
        for (index, token) in tokens.iter().enumerate() {
            if matches!(token, TokenTree::Punct(punctuation) if matches!(punctuation.as_char(), '.' | ':'))
                && matches!(tokens.get(index + 1), Some(TokenTree::Punct(punctuation)) if punctuation.as_char() == '$')
            {
                self.errors.push(
                    "parameterized method or path macros cannot construct audited endpoints".into(),
                );
            }
            if let TokenTree::Ident(name) = token {
                if name == "include"
                    && matches!(tokens.get(index + 1), Some(TokenTree::Punct(punctuation)) if punctuation.as_char() == '!')
                {
                    self.errors
                        .push("executable source includes inside macros are not auditable".into());
                }
                if REGISTRATION_METHODS.contains(&name.to_string().as_str())
                    && (matches!(tokens.get(index + 1), Some(TokenTree::Group(group)) if group.delimiter() == proc_macro2::Delimiter::Parenthesis)
                        || matches!(index.checked_sub(1).and_then(|previous| tokens.get(previous)), Some(TokenTree::Punct(punctuation)) if matches!(punctuation.as_char(), '.' | ':')))
                {
                    self.errors
                        .push(format!("unclassified route registration in macro: {name}"));
                }
                if name == "mod"
                    && matches!(tokens.get(index + 1), Some(TokenTree::Ident(_)))
                    && matches!(tokens.get(index + 2), Some(TokenTree::Punct(punctuation)) if punctuation.as_char() == ';')
                {
                    self.errors.push("out-of-line modules must be declared outside macros for entrypoint auditing".into());
                }
            }
            if let TokenTree::Group(group) = token {
                self.tokens(group.stream());
            }
        }
    }
}

impl<'ast> Visit<'ast> for Audit<'_> {
    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        if is_test(&item.attrs) {
            return;
        }
        if item
            .attrs
            .iter()
            .any(|attribute| attribute.path().is_ident("cfg_attr"))
        {
            self.errors.push(
                "conditional module attributes require explicit cfg/path module declarations"
                    .into(),
            );
            return;
        }
        let explicit = match explicit_module_path(&item.attrs) {
            Ok(path) => path,
            Err(error) => {
                self.errors.push(error);
                return;
            }
        };
        if let Some((_, items)) = &item.content {
            let old = self.module_directory.clone();
            let old_path = self.path_directory.clone();
            self.module_directory = explicit.map_or_else(
                || old.join(item.ident.to_string()),
                |path| old_path.join(path),
            );
            self.path_directory = self.module_directory.clone();
            for child in items {
                self.visit_item(child);
            }
            self.module_directory = old;
            self.path_directory = old_path;
        } else {
            let owns_directory = explicit.is_some();
            let path = match explicit {
                Some(path) => self.path_directory.join(path),
                None => {
                    let direct = self.module_directory.join(format!("{}.rs", item.ident));
                    if direct.is_file() {
                        direct
                    } else {
                        self.module_directory
                            .join(item.ident.to_string())
                            .join("mod.rs")
                    }
                }
            };
            if let Err(error) = audit_file(&path, self.seen, self.registry_path, owns_directory) {
                self.errors.push(error);
            }
        }
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        if !is_test(&item.attrs) {
            visit::visit_item_fn(self, item);
        }
    }

    fn visit_item_macro(&mut self, item: &'ast syn::ItemMacro) {
        if is_test(&item.attrs) {
            return;
        }
        let reviewed_template = self.registry
            && item.mac.path.is_ident("macro_rules")
            && item.ident.as_ref().is_some_and(|name| {
                name == "registered_http_routes"
                    || name == "registered_web_routes"
                    || name == "registered_runtime_routes"
            });
        if !reviewed_template {
            self.visit_macro(&item.mac);
        }
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if REGISTRATION_METHODS.contains(&call.method.to_string().as_str()) {
            self.errors
                .push(format!("unclassified route registration: {}", call.method));
        }
        visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = call.func.as_ref()
            && path
                .path
                .segments
                .last()
                .is_some_and(|part| REGISTRATION_METHODS.contains(&part.ident.to_string().as_str()))
        {
            self.errors
                .push("unclassified route registration through a function call".into());
        }
        visit::visit_expr_call(self, call);
    }

    fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
        if path.path.segments.len() > 1
            && path
                .path
                .segments
                .last()
                .is_some_and(|part| REGISTRATION_METHODS.contains(&part.ident.to_string().as_str()))
        {
            self.errors.push(
                "route construction function cannot escape the registry through an alias".into(),
            );
        }
        visit::visit_expr_path(self, path);
    }

    fn visit_macro(&mut self, invocation: &'ast syn::Macro) {
        if invocation
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "include")
        {
            let generated_keys: TokenStream =
                r#"concat!(env!("OUT_DIR"), "/trusted_license_keys.rs")"#
                    .parse()
                    .expect("fixed include expression");
            let keys_module = self
                .registry_path
                .parent()
                .expect("registry has a parent")
                .join("compiled_keys");
            if self.module_directory != keys_module
                || invocation.tokens.to_string() != generated_keys.to_string()
            {
                self.errors.push(
                    "executable source includes must use an audited module declaration".into(),
                );
            }
            return;
        }
        self.tokens(invocation.tokens.clone());
    }
}

fn audit_file(
    path: &Path,
    seen: &mut BTreeSet<(PathBuf, bool)>,
    registry_path: &Path,
    owns_directory: bool,
) -> Result<(), String> {
    let path = path
        .canonicalize()
        .map_err(|error| format!("{}: {error}", path.display()))?;
    if !seen.insert((path.clone(), owns_directory)) {
        return Ok(());
    }
    let source = fs::read_to_string(&path).map_err(|error| error.to_string())?;
    let file = syn::parse_file(&source).map_err(|error| error.to_string())?;
    let parent = path.parent().ok_or("source has no parent")?;
    let stem = path
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or("invalid module name")?;
    let module_directory = if owns_directory || stem == "mod" {
        parent.to_path_buf()
    } else {
        parent.join(stem)
    };
    let mut audit = Audit {
        registry: path == registry_path,
        registry_path,
        module_directory,
        path_directory: parent.to_path_buf(),
        seen,
        errors: Vec::new(),
    };
    audit.visit_file(&file);
    if audit.errors.is_empty() {
        Ok(())
    } else {
        Err(format!("{}: {}", path.display(), audit.errors.join("; ")))
    }
}

pub fn audit_crate(source: &Path) -> Result<Vec<PathBuf>, String> {
    let source = source.canonicalize().map_err(|error| error.to_string())?;
    let registry_path = source.join("entrypoints.rs");
    let mut seen = BTreeSet::new();
    let mut roots = BTreeSet::new();
    for root in [source.join("lib.rs"), source.join("main.rs")] {
        if root.is_file() {
            roots.insert(root);
        }
    }
    let bin = source.join("bin");
    if bin.is_dir() {
        for entry in fs::read_dir(bin).map_err(|error| error.to_string())? {
            let path = entry.map_err(|error| error.to_string())?.path();
            if path.extension().is_some_and(|extension| extension == "rs") {
                roots.insert(path);
            } else if path.is_dir() && path.join("main.rs").is_file() {
                roots.insert(path.join("main.rs"));
            }
        }
    }
    let crate_root = source.parent().ok_or("source has no crate root")?;
    let manifest: toml::Value = fs::read_to_string(crate_root.join("Cargo.toml"))
        .map_err(|error| error.to_string())?
        .parse::<toml::Value>()
        .map_err(|error| error.to_string())?;
    if let Some(library) = manifest.get("lib") {
        add_explicit_target(&mut roots, crate_root, library)?;
    }
    if let Some(binaries) = manifest.get("bin") {
        for binary in binaries.as_array().ok_or("bin targets must be an array")? {
            add_explicit_target(&mut roots, crate_root, binary)?;
        }
    }
    for root in roots {
        audit_file(&root, &mut seen, &registry_path, true)?;
    }
    let mut watched: BTreeSet<_> = seen.into_iter().map(|(path, _)| path).collect();
    watched.insert(crate_root.join("Cargo.toml"));
    Ok(watched.into_iter().collect())
}

fn add_explicit_target(
    roots: &mut BTreeSet<PathBuf>,
    crate_root: &Path,
    target: &toml::Value,
) -> Result<(), String> {
    let target = target.as_table().ok_or("target must be a TOML table")?;
    if let Some(path) = target.get("path") {
        roots.insert(crate_root.join(path.as_str().ok_or("target path must be a string")?));
    }
    Ok(())
}

fn explicit_module_path(attributes: &[syn::Attribute]) -> Result<Option<String>, String> {
    let mut paths = attributes
        .iter()
        .filter(|attribute| attribute.path().is_ident("path"));
    let Some(attribute) = paths.next() else {
        return Ok(None);
    };
    if paths.next().is_some() {
        return Err("multiple module paths are not auditable".into());
    }
    match &attribute.meta {
        syn::Meta::NameValue(value) => match &value.value {
            syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(path),
                ..
            }) => Ok(Some(path.value())),
            _ => Err("unreadable module path".into()),
        },
        _ => Err("unreadable module path".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(lib: &str, extra: &[(&str, &str)]) -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("src")).unwrap();
        fs::write(
            directory.path().join("Cargo.toml"),
            "[package]\nname = 'audit-fixture'\nversion = '0.0.0'\n",
        )
        .unwrap();
        fs::write(directory.path().join("src/lib.rs"), lib).unwrap();
        fs::write(directory.path().join("src/main.rs"), "fn main() {}").unwrap();
        for (name, text) in extra {
            let path = directory.path().join("src").join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        directory
    }

    #[test]
    fn direct_methods_ufcs_nested_routers_and_fallbacks_require_registration() {
        for expression in [
            "Router::new().route(\"/forgotten\", get(handler))",
            "r.route_service(\"/forgotten\", service)",
            "r.nest(\"/private\", nested)",
            "r.nest_service(\"/private\", service)",
            "r.merge(other)",
            "r.fallback_service(service)",
            "Router::route(r, \"/forgotten\", get(handler))",
        ] {
            let directory = source(&format!("fn accidental() {{ {expression}; }}"), &[]);
            assert!(
                audit_crate(&directory.path().join("src")).is_err(),
                "{expression}"
            );
        }
    }

    #[test]
    fn module_graph_does_not_exempt_a_production_file_named_tests() {
        for declaration in ["mod tests;", "#[path = \"tests.rs\"] mod api;"] {
            let directory = source(
                declaration,
                &[("tests.rs", "fn f() { r.route(\"/missed\", get(handler)); }")],
            );
            assert!(audit_crate(&directory.path().join("src")).is_err());
        }
        let directory = source(
            "mod nested { mod routes; }",
            &[("nested/routes.rs", "fn f() { r.fallback(handler); }")],
        );
        assert!(audit_crate(&directory.path().join("src")).is_err());
    }

    #[test]
    fn routing_inside_an_unreviewed_macro_or_binary_is_rejected() {
        let directory = source(
            "macro_rules! other { () => { r.route(\"/missed\", get(handler)) }; }",
            &[],
        );
        assert!(audit_crate(&directory.path().join("src")).is_err());
        let directory = source(
            "",
            &[(
                "bin/independent.rs",
                "fn main() { r.route(\"/missed\", get(handler)); }",
            )],
        );
        assert!(audit_crate(&directory.path().join("src")).is_err());
        let directory = source(
            "macro_rules! registered_http_routes { () => { r.route(\"/missed\", get(handler)) }; }",
            &[],
        );
        assert!(audit_crate(&directory.path().join("src")).is_err());
    }

    #[test]
    fn private_runtime_template_cannot_be_redefined_outside_the_registry() {
        let directory = source(
            "macro_rules! registered_runtime_routes { () => { r.route(path, handler) }; }",
            &[],
        );
        assert!(audit_crate(&directory.path().join("src")).is_err());
    }

    #[test]
    fn test_only_routers_and_documentation_strings_are_not_public_entries() {
        let directory = source(
            r#"#[cfg(test)] mod tests { fn f() { r.route("/fixture", get(handler)); } }
            fn documentation() { let example = "r.route(path, handler)"; println!("r.route(path, handler)"); }"#,
            &[],
        );
        audit_crate(&directory.path().join("src")).unwrap();
    }

    #[test]
    fn includes_function_items_and_parameterized_macros_cannot_hide_registration() {
        for code in [
            "include!(\"extra.rs\");",
            "std::include!(\"extra.rs\");",
            "macro_rules! routes { () => { include!(\"extra.rs\"); } } routes!();",
            "macro_rules! routes { () => { std::include!(\"extra.rs\"); } } routes!();",
            "fn f() { let attach = axum::Router::route; attach(router, path, handler); }",
            "macro_rules! invoke { ($r:expr, $method:ident) => { $r.$method(path, handler) }; }",
            "macro_rules! invoke { () => { mod extra; }; }",
        ] {
            let directory = source(code, &[("extra.rs", "fn f() { r.route(path, handler); }")]);
            assert!(
                audit_crate(&directory.path().join("src")).is_err(),
                "{code}"
            );
        }
    }

    #[test]
    fn directory_binaries_and_explicit_target_paths_are_audited() {
        let directory = source(
            "",
            &[
                ("bin/other/main.rs", "mod api;"),
                ("bin/other/api.rs", "fn f() { r.route(path, handler); }"),
            ],
        );
        assert!(audit_crate(&directory.path().join("src")).is_err());
        for target in [
            "[lib]\npath = 'custom.rs'\n",
            "[[bin]]\nname = 'custom'\npath = 'custom.rs'\n",
        ] {
            let directory = source("", &[]);
            fs::write(directory.path().join("custom.rs"), "mod api;").unwrap();
            fs::write(
                directory.path().join("api.rs"),
                "fn f() { r.route(path, handler); }",
            )
            .unwrap();
            fs::write(
                directory.path().join("Cargo.toml"),
                format!("[package]\nname='fixture'\nversion='0.0.0'\n{target}"),
            )
            .unwrap();
            assert!(audit_crate(&directory.path().join("src")).is_err());
        }
    }

    #[test]
    fn explicit_module_paths_use_the_source_directory_and_own_their_child_directory() {
        let bad = "fn f() { r.route(path, handler); }";
        let directory = source(
            "mod outer;",
            &[
                ("outer.rs", "#[path=\"actual.rs\"] mod child;"),
                ("actual.rs", bad),
                ("outer/actual.rs", ""),
            ],
        );
        assert!(audit_crate(&directory.path().join("src")).is_err());
        fs::write(directory.path().join("src/actual.rs"), "").unwrap();
        audit_crate(&directory.path().join("src")).unwrap();
        let directory = source(
            "#[path=\"mapped.rs\"] mod alias;",
            &[
                ("mapped.rs", "mod child;"),
                ("child.rs", bad),
                ("mapped/child.rs", ""),
            ],
        );
        assert!(audit_crate(&directory.path().join("src")).is_err());
        fs::write(directory.path().join("src/child.rs"), "").unwrap();
        audit_crate(&directory.path().join("src")).unwrap();
        let directory = source(
            "mod outer;",
            &[
                (
                    "outer.rs",
                    "mod inline { #[path=\"actual.rs\"] mod child; }",
                ),
                ("outer/inline/actual.rs", bad),
                ("actual.rs", ""),
            ],
        );
        assert!(audit_crate(&directory.path().join("src")).is_err());
        fs::write(directory.path().join("src/outer/inline/actual.rs"), "").unwrap();
        audit_crate(&directory.path().join("src")).unwrap();
        let directory = source(
            "mod outer;",
            &[
                (
                    "outer.rs",
                    "#[path=\"alternate\"] mod inline { mod child; }",
                ),
                ("alternate/child.rs", bad),
                ("outer/alternate/child.rs", ""),
            ],
        );
        assert!(audit_crate(&directory.path().join("src")).is_err());
        fs::write(directory.path().join("src/alternate/child.rs"), "").unwrap();
        audit_crate(&directory.path().join("src")).unwrap();
    }

    #[test]
    fn incremental_build_watches_targets_and_modules_outside_the_source_directory() {
        let directory = source("#[path=\"../external.rs\"] mod external;", &[]);
        let manifest = directory.path().join("Cargo.toml");
        fs::write(&manifest, "[package]\nname='fixture'\nversion='0.0.0'\n[[bin]]\nname='custom'\npath='custom.rs'\n").unwrap();
        let external = directory.path().join("external.rs");
        let custom = directory.path().join("custom.rs");
        fs::write(&external, "").unwrap();
        fs::write(&custom, "fn main() {}").unwrap();
        let watched = audit_crate(&directory.path().join("src")).unwrap();
        for path in [manifest, custom, external] {
            assert!(
                watched.contains(&path.canonicalize().unwrap()),
                "{}",
                path.display()
            );
        }
    }

    #[test]
    fn conditional_paths_and_a_second_registry_filename_do_not_create_exceptions() {
        let directory = source(
            "#[cfg_attr(any(), path=\"alternate.rs\")] mod normal;",
            &[("normal.rs", ""), ("alternate.rs", "")],
        );
        assert!(audit_crate(&directory.path().join("src")).is_err());
        let directory = source(
            "#[path=\"other/src/entrypoints.rs\"] mod other;",
            &[(
                "other/src/entrypoints.rs",
                "macro_rules! registered_http_routes { () => { r.route(path, handler) }; }",
            )],
        );
        assert!(audit_crate(&directory.path().join("src")).is_err());
    }
}
