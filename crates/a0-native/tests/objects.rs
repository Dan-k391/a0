//! Object emission for every supported target. Linking is host-only, but generating correct object
//! files is not, and that is what keeps the backend honest about never assuming the host.

use std::path::Path;

use a0_diag::SourceId;
use a0_native::{CodegenOptions, Target, compile_object};
use a0_target::{Arch, ObjectFormat};
use object::{Object, ObjectSymbol};

fn corpus() -> Vec<(String, a0_ir::Module)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/programs");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "a0ir") {
            let text = std::fs::read_to_string(&path).unwrap();
            let m = a0_ir::parse_and_verify(&text, SourceId(0)).unwrap();
            out.push((path.file_name().unwrap().to_string_lossy().into_owned(), m));
        }
    }
    assert!(!out.is_empty());
    out
}

fn expected_format(t: Target) -> object::BinaryFormat {
    match t.object_format() {
        ObjectFormat::Elf => object::BinaryFormat::Elf,
        ObjectFormat::MachO => object::BinaryFormat::MachO,
        ObjectFormat::Coff => object::BinaryFormat::Coff,
    }
}

fn expected_arch(t: Target) -> object::Architecture {
    match t.arch() {
        Arch::X86_64 => object::Architecture::X86_64,
        Arch::Aarch64 => object::Architecture::Aarch64,
    }
}

/// Symbol names as C sees them (Mach-O prefixes an underscore).
fn c_name(t: Target, name: &str) -> String {
    match t.object_format() {
        ObjectFormat::MachO => format!("_{name}"),
        _ => name.to_string(),
    }
}

/// Every target, aarch64-windows included since the COFF relocations it needs were added
/// (`vendor/README.md`).
fn emittable_targets() -> Vec<Target> {
    Target::ALL.to_vec()
}

#[test]
fn aarch64_windows_objects_have_arm64_relocations() {
    use object::{Object, ObjectSection};
    // `hello` takes a string's address: an `adrp`/`add` pair.
    let (_, m) = corpus()
        .into_iter()
        .find(|(name, _)| name.contains("hello"))
        .unwrap();
    let bytes = compile_object(&m, Target::AARCH64_WINDOWS, &CodegenOptions::default()).unwrap();
    let file = object::File::parse(&*bytes).unwrap();
    assert_eq!(file.architecture(), object::Architecture::Aarch64);
    let kinds: Vec<object::pe::RelocationType> = file
        .sections()
        .flat_map(|s| s.relocations().collect::<Vec<_>>())
        .filter_map(|(_, r)| match r.flags() {
            object::RelocationFlags::Coff { typ } => Some(typ),
            _ => None,
        })
        .collect();
    assert!(
        kinds.contains(&object::pe::IMAGE_REL_ARM64_PAGEBASE_REL21),
        "{kinds:?}"
    );
    assert!(
        kinds.contains(&object::pe::IMAGE_REL_ARM64_PAGEOFFSET_12A),
        "{kinds:?}"
    );
}

#[test]
fn every_program_compiles_for_every_target() {
    for (name, m) in corpus() {
        for target in emittable_targets() {
            let opts = CodegenOptions {
                optimize: true,
                entry_point: true,
                debug_info: false,
            };
            let bytes = compile_object(&m, target, &opts)
                .unwrap_or_else(|d| panic!("{name} for {target}: {}", d.summary()));
            let file =
                object::File::parse(&*bytes).unwrap_or_else(|e| panic!("{name} for {target}: {e}"));
            assert_eq!(
                file.format(),
                expected_format(target),
                "{name} for {target}"
            );
            assert_eq!(
                file.architecture(),
                expected_arch(target),
                "{name} for {target}"
            );
            let defined: Vec<String> = file
                .symbols()
                .filter(|s| s.is_definition())
                .filter_map(|s| s.name().ok().map(str::to_string))
                .collect();
            for sym in ["main", a0_native::ENTRY_SYMBOL] {
                let want = c_name(target, sym);
                assert!(
                    defined.contains(&want),
                    "{name} for {target}: {want} missing in {defined:?}"
                );
            }
            let undefined: Vec<String> = file
                .symbols()
                .filter(|s| s.is_undefined())
                .filter_map(|s| s.name().ok().map(str::to_string))
                .collect();
            assert!(
                undefined.contains(&c_name(target, "a0_rt_init")),
                "{name} for {target}: runtime init not referenced"
            );
        }
    }
}

#[test]
fn library_objects_export_functions_under_their_names() {
    let text = "module \"lib\"\nfunc @\"binary|\"(%a: f64, %b: f64) -> f64 {\n  %r: f64 = fadd %a, %b\n  return %r\n}\nfunc @main() -> i32 {\n  %0: i32 = iconst 0\n  return %0\n}\n";
    let m = a0_ir::parse_and_verify(text, SourceId(0)).unwrap();
    for target in emittable_targets() {
        let bytes = compile_object(&m, target, &CodegenOptions::default()).unwrap();
        let file = object::File::parse(&*bytes).unwrap();
        let names: Vec<String> = file
            .symbols()
            .filter(|s| s.is_definition())
            .filter_map(|s| s.name().ok().map(str::to_string))
            .collect();
        // Without an entry point, @main keeps its own name and no C wrapper is generated.
        assert!(
            names.contains(&c_name(target, "main")),
            "{target}: {names:?}"
        );
        assert!(
            !names.contains(&c_name(target, a0_native::ENTRY_SYMBOL)),
            "{target}: {names:?}"
        );
        assert!(
            names.contains(&c_name(target, "binary|")),
            "{target}: {names:?}"
        );
    }
}

#[test]
fn invalid_modules_are_rejected_with_diagnostics() {
    let text = "module \"m\"\nfunc @f() -> i64 {\n  %x: f64 = fconst 1.0\n  return %x\n}\n";
    let m = a0_ir::parse(text, SourceId(0)).unwrap();
    let err = compile_object(&m, Target::X86_64_LINUX, &CodegenOptions::default()).unwrap_err();
    assert!(
        err.summary()
            .contains("returns f64 but the function returns i64")
    );

    let no_main = a0_ir::parse_and_verify("module \"m\"\n", SourceId(0)).unwrap();
    let opts = CodegenOptions {
        optimize: false,
        entry_point: true,
        debug_info: false,
    };
    let err = compile_object(&no_main, Target::X86_64_LINUX, &opts).unwrap_err();
    assert!(err.summary().contains("no function @main"));
}

#[test]
fn linking_for_another_target_is_refused() {
    let host = Target::host().unwrap();
    let other = Target::ALL.into_iter().find(|&t| t != host).unwrap();
    let err = a0_native::link_executable(&[], other, Path::new("never"), &Default::default())
        .unwrap_err();
    assert!(
        err.summary()
            .contains("linking is only supported for the host target")
    );
}
