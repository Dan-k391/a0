//! Every IR and HIR program of the differential corpus compiles to a module that validates, with
//! the imports and exports a runtime instance links it by. (They run, and are compared with what
//! they expect, in the playground's CI job: `playground/test.mjs`.)

use std::path::Path;

use a0_diag::SourceMap;

fn corpus() -> Vec<(String, a0_ir::Module)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut out = Vec::new();
    for dir in ["tests/programs", "tests/hir"] {
        let mut paths: Vec<_> = std::fs::read_dir(root.join(dir))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        paths.sort();
        for path in paths {
            let text = std::fs::read_to_string(&path).unwrap();
            let mut sources = SourceMap::new();
            let id = sources.add(&path.to_string_lossy(), &text);
            let m = match path.extension().and_then(|e| e.to_str()) {
                Some("a0ir") => a0_ir::parse_and_verify(&text, id).unwrap(),
                Some("hir") => a0_hir::lower(&a0_hir::parse(&text, id).unwrap()).unwrap(),
                _ => continue,
            };
            out.push((path.file_name().unwrap().to_string_lossy().into_owned(), m));
        }
    }
    assert!(out.len() > 20);
    out
}

#[test]
fn every_program_compiles_to_a_valid_module() {
    let mut failures = Vec::new();
    for (name, m) in corpus() {
        match a0_wasm::compile(&m) {
            Ok(bytes) => {
                let mut v = wasmparser::Validator::new();
                if let Err(e) = v.validate_all(&bytes) {
                    failures.push(format!("{name}: invalid module: {e}"));
                }
            }
            // C structs by value are not supported yet; nothing else may fail.
            Err(d) if name == "c_structs.a0ir" || name == "c_structs.hir" => {
                assert!(
                    d.summary().contains("not supported in WebAssembly"),
                    "{}",
                    d.summary()
                );
            }
            Err(d) => failures.push(format!("{name}: {}", d.summary())),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn modules_import_the_runtime_and_export_their_entry() {
    let (_, m) = corpus()
        .into_iter()
        .find(|(n, _)| n == "hello.a0ir")
        .unwrap();
    let bytes = a0_wasm::compile(&m).unwrap();
    let mut imports = Vec::new();
    let mut exports = Vec::new();
    for payload in wasmparser::Parser::new(0).parse_all(&bytes) {
        match payload.unwrap() {
            wasmparser::Payload::ImportSection(s) => {
                for i in s.into_imports() {
                    let i = i.unwrap();
                    imports.push(format!("{}.{}", i.module, i.name));
                }
            }
            wasmparser::Payload::ExportSection(s) => {
                for e in s {
                    exports.push(e.unwrap().name.to_string());
                }
            }
            _ => {}
        }
    }
    for wanted in [
        "env.memory",
        "env.__indirect_function_table",
        "env.__memory_base",
        "env.__table_base",
        "env.a0_rt_trap",
        "env.a0_rt_set_frames",
    ] {
        assert!(
            imports.contains(&wanted.to_string()),
            "{wanted} in {imports:?}"
        );
    }
    for wanted in ["a0_wasm_main", "__data_size", "__table_size"] {
        assert!(
            exports.contains(&wanted.to_string()),
            "{wanted} in {exports:?}"
        );
    }
}
