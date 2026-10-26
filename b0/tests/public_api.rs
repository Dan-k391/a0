//! B0 is built only on A0's public API (`docs/compatibility.md`, D86): its dependencies are the
//! covered crates, and its source names no other A0 crate.

use std::path::Path;

/// The crates whose public items A0 1.x keeps compatible.
const COVERED: &[&str] = &[
    "a0-diag",
    "a0-syntax",
    "a0-sema",
    "a0-ir",
    "a0-hir",
    "a0-driver",
    "a0-build",
];

#[test]
fn b0_depends_only_on_covered_crates() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let mut section = String::new();
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            section = line.to_string();
            continue;
        }
        let Some(name) = line
            .split(['.', '=', ' '])
            .next()
            .filter(|n| n.starts_with("a0"))
        else {
            continue;
        };
        if section == "[dependencies]" || section == "[build-dependencies]" {
            assert!(
                COVERED.contains(&name),
                "B0 depends on `{name}`, which A0's compatibility promise does not cover"
            );
        }
    }
    for entry in std::fs::read_dir(root.join("src")).unwrap() {
        let path = entry.unwrap().path();
        let text = std::fs::read_to_string(&path).unwrap();
        for crate_name in [
            "a0_native",
            "a0_interp",
            "a0_wasm",
            "a0_runtime",
            "a0_target",
            "a0_capi",
        ] {
            assert!(
                !text.contains(crate_name),
                "{} uses `{crate_name}`, which is not part of A0's public API",
                path.display()
            );
        }
    }
}
