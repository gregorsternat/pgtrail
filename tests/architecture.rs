//! Source-level guardrails, not a substitute for behavioral tests or code review.
use std::{collections::BTreeSet, fs, path::Path};
use syn::{ItemMod, ItemUse, UseTree, visit::Visit};

// Keep changes to these edges reviewable alongside docs/architecture.md.
fn dependencies(module: &str) -> Option<&'static [&'static str]> {
    Some(match module {
        "model" | "profiles" | "cli" | "event" | "terminal" => &[],
        "collector" | "store" | "demo" => &["model"],
        "compare" => &["model", "diagnostics", "metrics"],
        "metrics" => &["model", "compare"],
        "diagnostics" => &["model", "metrics"],
        "report" => &["model", "compare", "diagnostics", "metrics", "store"],
        "incidents" => &["model", "compare", "diagnostics", "report", "store"],
        "app" => &[
            "model",
            "event",
            "store",
            "compare",
            "diagnostics",
            "metrics",
            "incidents",
            "ui",
        ],
        "ui" => &[
            "model",
            "app",
            "compare",
            "diagnostics",
            "metrics",
            "incidents",
            "report",
            "store",
        ],
        // These are the composition roots and may coordinate all modules.
        "lib" | "main" | "runtime" | "commands" => return None,
        _ => panic!(
            "Unclassified module {module}; define its boundary in tests/architecture.rs and docs/architecture.md"
        ),
    })
}

fn pure(module: &str) -> bool {
    matches!(
        module,
        "model"
            | "compare"
            | "metrics"
            | "diagnostics"
            | "report"
            | "incidents"
            | "demo"
            | "app"
            | "ui"
    )
}

struct Boundaries<'a> {
    module: &'a str,
    errors: BTreeSet<String>,
    children: Vec<Vec<String>>,
    scope: Vec<String>,
}

impl Boundaries<'_> {
    fn check(&mut self, parts: &[String]) {
        let resolved;
        let parts = if parts
            .first()
            .is_some_and(|part| part == "self" || part == "super")
        {
            let mut scope = self.scope.clone();
            let mut offset = 0;
            while parts
                .get(offset)
                .is_some_and(|part| part == "self" || part == "super")
            {
                if parts[offset] == "super" {
                    scope.pop();
                }
                offset += 1;
            }
            resolved = std::iter::once("crate".to_owned())
                .chain(scope)
                .chain(parts[offset..].iter().cloned())
                .collect::<Vec<_>>();
            &resolved
        } else {
            parts
        };
        let path = parts.join("::");
        let names: Vec<_> = parts.iter().map(String::as_str).collect();
        let allowed = dependencies(self.module);
        let forbidden_edge = match names.as_slice() {
            ["crate", target, ..] => {
                allowed.is_some_and(|edges| *target != self.module && !edges.contains(target))
            }
            _ => false,
        };
        let forbidden_io = pure(self.module)
            && match names.as_slice() {
                ["sqlx" | "tokio", ..] => true,
                [
                    "std",
                    "fs" | "io" | "net" | "process" | "env" | "thread" | "os",
                    ..,
                ] => true,
                ["crossterm", "event", name, ..]
                    if self.module == "app"
                        && matches!(*name, "KeyCode" | "KeyEvent" | "KeyModifiers") =>
                {
                    false
                }
                ["crossterm", ..] => true,
                // Existing shared records live in store; importing the service or a
                // module alias would also expose its I/O methods to pure modules.
                ["crate", "store", name, ..] => !matches!(
                    *name,
                    "SnapshotSummary"
                        | "Incident"
                        | "IncidentSummary"
                        | "IncidentNote"
                        | "IncidentCapture"
                        | "CaptureMetadata"
                ),
                ["crate", "store"] => true,
                ["crate", "event", "Message", ..] => false,
                ["crate", "event", ..] => true,
                _ => false,
            };
        if forbidden_edge || forbidden_io {
            self.errors.insert(format!("{} -> {path}: move I/O to runtime/commands or the owning adapter; pass data/messages instead. See docs/architecture.md#enforced-boundaries", self.module));
        }
    }

    fn imports(&mut self, tree: &UseTree, prefix: &mut Vec<String>) {
        match tree {
            UseTree::Path(path) => {
                prefix.push(path.ident.to_string());
                self.imports(&path.tree, prefix);
                prefix.pop();
            }
            UseTree::Group(group) => {
                for item in &group.items {
                    self.imports(item, prefix);
                }
            }
            UseTree::Name(name) => {
                if name.ident != "self" {
                    prefix.push(name.ident.to_string());
                }
                self.check(prefix);
                if name.ident != "self" {
                    prefix.pop();
                }
            }
            UseTree::Rename(rename) => {
                if rename.ident != "self" {
                    prefix.push(rename.ident.to_string());
                }
                self.check(prefix);
                if rename.ident != "self" {
                    prefix.pop();
                }
            }
            UseTree::Glob(_) => {
                prefix.push("*".into());
                self.check(prefix);
                prefix.pop();
            }
        }
    }
}

fn test_only(module: &ItemMod) -> bool {
    module.attrs.iter().any(|attr| {
        attr.path().is_ident("cfg")
            && attr
                .parse_args::<syn::Path>()
                .is_ok_and(|path| path.is_ident("test"))
    })
}

impl<'ast> Visit<'ast> for Boundaries<'_> {
    fn visit_item_use(&mut self, item: &'ast ItemUse) {
        self.imports(&item.tree, &mut Vec::new());
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        self.check(
            &path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect::<Vec<_>>(),
        );
        syn::visit::visit_path(self, path);
    }

    fn visit_item_mod(&mut self, module: &'ast ItemMod) {
        if test_only(module) {
            return;
        }
        if module.attrs.iter().any(|attr| attr.path().is_ident("path")) {
            self.errors.insert("Explicit module paths need checker support; use the conventional src module layout".into());
            return;
        }
        if self.scope.is_empty() && module.content.is_some() {
            self.errors.insert(format!(
                "Move top-level {} into its own source file and classify its dependencies",
                module.ident
            ));
            return;
        }
        self.scope.push(module.ident.to_string());
        if module.content.is_none() {
            self.children.push(self.scope.clone());
        }
        syn::visit::visit_item_mod(self, module);
        self.scope.pop();
    }

    fn visit_macro(&mut self, item: &'ast syn::Macro) {
        if pure(self.module)
            && item.path.segments.last().is_some_and(|part| {
                matches!(
                    part.ident.to_string().as_str(),
                    "print" | "println" | "eprint" | "eprintln" | "dbg"
                )
            })
        {
            self.errors.insert(format!(
                "{}: terminal output belongs in runtime/commands; return data from pure modules",
                self.module
            ));
        }
        syn::visit::visit_macro(self, item);
    }
}

fn inspect<'a>(source: &str, module: &'a str) -> Boundaries<'a> {
    let mut visitor = Boundaries {
        module,
        errors: BTreeSet::new(),
        children: Vec::new(),
        scope: vec![module.into()],
    };
    visitor.visit_file(&syn::parse_file(source).expect("fixture parses"));
    visitor
}

fn check_file(root: &Path, scope: Vec<String>, errors: &mut Vec<String>) {
    let module = scope.first().map_or("lib", String::as_str);
    let relative: std::path::PathBuf = scope.iter().collect();
    let file = if scope.is_empty() {
        root.join("lib.rs")
    } else {
        root.join(&relative).with_extension("rs")
    };
    let file = if file.exists() {
        file
    } else {
        root.join(relative).join("mod.rs")
    };
    let source = fs::read_to_string(&file).expect("read Rust module");
    let ast = syn::parse_file(&source).expect("parse Rust module");
    dependencies(module);
    let mut visitor = Boundaries {
        module,
        errors: BTreeSet::new(),
        children: Vec::new(),
        scope: scope.clone(),
    };
    visitor.visit_file(&ast);
    errors.extend(
        visitor
            .errors
            .into_iter()
            .map(|error| format!("{}: {error}", file.display())),
    );
    for child in visitor.children {
        check_file(root, child, errors);
    }
}

#[test]
fn production_modules_respect_boundaries() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut errors = Vec::new();
    check_file(&source, Vec::new(), &mut errors);
    check_file(&source, vec!["main".into()], &mut errors);
    assert!(
        errors.is_empty(),
        "Architecture violations:\n{}",
        errors.join("\n")
    );
}

#[test]
fn rejects_nested_aliases_qualified_io_and_adapter_coupling() {
    for source in [
        "use std::{collections::BTreeMap, fs as disk};",
        "fn render() { std::fs::read(\"file\"); }",
        "use crate::{store::{Store as History}};",
        "use crate::store::{self as history};",
        "use crate::store::*;",
        "use crate::collector::Collector;",
        "use super::collector::Collector;",
        "mod nested { use super::super::store::Store; }",
        "fn render() { println!(\"output\"); }",
        "mod nested { use tokio::fs; }",
    ] {
        assert!(
            !inspect(source, "ui").errors.is_empty(),
            "accepted forbidden dependency: {source}"
        );
    }
    assert!(
        !inspect("use crate::store::Store;", "collector")
            .errors
            .is_empty()
    );
}

#[test]
fn permits_pure_records_and_ignores_test_modules_comments_and_strings() {
    let source = r#"
        use crate::{app::App, store::Incident};
        // std::fs::read is discussed, not called.
        const LABEL: &str = "crate::collector::Collector";
        #[cfg(test)] mod tests { use sqlx::PgPool; }
    "#;
    assert!(inspect(source, "ui").errors.is_empty());
}
