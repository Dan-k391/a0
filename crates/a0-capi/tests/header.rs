//! `include/a0.h` declares exactly the functions the library exports.

use std::collections::BTreeSet;

/// Names `a0_...` directly followed by `(`, as in a declaration `T a0_name(...)` or a definition
/// `fn a0_name(...)`.
fn called_names(text: &str, after: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (i, _) in text.match_indices(after) {
        let rest = &text[i + after.len()..];
        let Some(rest) = rest.strip_prefix("a0_") else {
            continue;
        };
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        if rest[end..].starts_with('(') {
            out.insert(format!("a0_{}", &rest[..end]));
        }
    }
    out
}

#[test]
fn the_header_matches_the_exports() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let header = std::fs::read_to_string(root.join("include/a0.h")).unwrap();
    // In the header, a function name follows a space or a `*` (`const char *a0_version(void)`).
    let mut declared = called_names(&header, " ");
    declared.extend(called_names(&header, "*"));
    let mut exported = BTreeSet::new();
    for file in ["src/lib.rs", "src/hir.rs", "src/language.rs"] {
        let source = std::fs::read_to_string(root.join(file)).unwrap();
        exported.extend(called_names(&source, "extern \"C\" fn "));
        // Functions made by macros: `leaf!(a0_hir_unit, unit)`.
        for (i, _) in source.match_indices("leaf!(") {
            let rest = &source[i + "leaf!(".len()..];
            if let Some(name) = rest.split(',').next().filter(|n| n.starts_with("a0_")) {
                exported.insert(name.to_string());
            }
        }
    }
    let missing: Vec<_> = declared.difference(&exported).collect();
    let undeclared: Vec<_> = exported.difference(&declared).collect();
    assert!(
        missing.is_empty() && undeclared.is_empty(),
        "declared but not exported: {missing:?}\nexported but not declared: {undeclared:?}"
    );
    assert!(
        exported.len() > 100,
        "found only {} functions",
        exported.len()
    );
}

#[test]
fn the_header_names_the_version() {
    // `A0_VERSION_*` must be the crate's version, which `a0_version()` returns.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let header = std::fs::read_to_string(root.join("include/a0.h")).unwrap();
    let get = |name: &str| {
        let line = header
            .lines()
            .find(|l| l.starts_with(&format!("#define {name} ")))
            .unwrap_or_else(|| panic!("no {name}"));
        line.rsplit(' ').next().unwrap().to_string()
    };
    let version = format!(
        "{}.{}.{}",
        get("A0_VERSION_MAJOR"),
        get("A0_VERSION_MINOR"),
        get("A0_VERSION_PATCH")
    );
    assert_eq!(version, env!("CARGO_PKG_VERSION"));
}
