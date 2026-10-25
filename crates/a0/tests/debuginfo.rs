//! `-g`: native code described for the platform's debuggers (DWARF; `docs/debugging.md`). The
//! line tables and functions are read back from object files for Linux and macOS on every host,
//! and where gdb or lldb is installed, it stops at a breakpoint in a built executable.

use std::path::{Path, PathBuf};
use std::process::Command;

use object::{Object, ObjectSection};

const PROGRAM: &str = "module \"lines\";

fn norm(x: float, y: float) -> float {
  let squares = x * x + y * y;
  squares / 5.0
}

fn main() -> int {
  let n = norm(3.0, 4.0);
  print(str(n) + \"\\n\");
  0
}
";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("a0-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("lines.hir"), PROGRAM).unwrap();
    dir
}

fn a0(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_a0"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}

/// The functions an object's DWARF names, and the (file, line) pairs its line table has.
fn read_dwarf(bytes: &[u8]) -> (Vec<String>, Vec<(String, u64)>) {
    let file = object::File::parse(bytes).unwrap();
    let load = |id: gimli::SectionId| -> Result<std::borrow::Cow<[u8]>, gimli::Error> {
        let name = if file.format() == object::BinaryFormat::MachO {
            format!("__{}", &id.name()[1..])
        } else {
            id.name().to_string()
        };
        Ok(file
            .section_by_name(&name)
            .and_then(|s| s.uncompressed_data().ok())
            .unwrap_or(std::borrow::Cow::Borrowed(&[])))
    };
    let owned = gimli::DwarfSections::load(load).unwrap();
    let dwarf = owned.borrow(|s| gimli::EndianSlice::new(s, gimli::LittleEndian));
    let mut names = Vec::new();
    let mut rows = Vec::new();
    let mut units = dwarf.units();
    while let Some(header) = units.next().unwrap() {
        let unit = dwarf.unit(header).unwrap();
        let mut entries = unit.entries();
        while let Some(entry) = entries.next_dfs().unwrap() {
            if entry.tag() == gimli::DW_TAG_subprogram
                && let Some(name) = entry.attr_value(gimli::DW_AT_name)
            {
                let name = dwarf.attr_string(&unit, name).unwrap();
                names.push(name.to_string_lossy().into_owned());
            }
        }
        let Some(program) = unit.line_program.clone() else {
            continue;
        };
        let mut lines = program.rows();
        while let Some((header, row)) = lines.next_row().unwrap() {
            if let (Some(line), Some(file)) = (row.line(), row.file(header)) {
                let path = dwarf.attr_string(&unit, file.path_name()).unwrap();
                rows.push((path.to_string_lossy().into_owned(), line.get()));
            }
        }
    }
    (names, rows)
}

#[test]
fn objects_describe_functions_and_lines_for_debuggers() {
    let dir = scratch("debuginfo");
    for target in [
        "x86_64-linux",
        "aarch64-linux",
        "x86_64-macos",
        "aarch64-macos",
    ] {
        let out = a0(
            &dir,
            &[
                "obj",
                "lines.hir",
                "-g",
                "--target",
                target,
                "-o",
                "lines.o",
            ],
        );
        assert!(
            out.status.success(),
            "{target}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let bytes = std::fs::read(dir.join("lines.o")).unwrap();
        let (names, rows) = read_dwarf(&bytes);
        assert!(
            names.contains(&"norm".to_string()) && names.contains(&"main".to_string()),
            "{target}: {names:?}"
        );
        for line in [3, 4, 5, 9, 10] {
            assert!(
                rows.iter()
                    .any(|(file, l)| file.ends_with("lines.hir") && *l == line),
                "{target}: no row for line {line} in {rows:?}"
            );
        }
        // Without -g there is no debug information.
        let out = a0(
            &dir,
            &["obj", "lines.hir", "--target", target, "-o", "plain.o"],
        );
        assert!(out.status.success());
        let plain = std::fs::read(dir.join("plain.o")).unwrap();
        assert!(read_dwarf(&plain).1.is_empty(), "{target}");
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// The platform's debugger, if one is installed: gdb, or lldb (macOS).
fn debugger() -> Option<&'static str> {
    ["gdb", "lldb"].into_iter().find(|d| {
        Command::new(d)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    })
}

#[test]
fn the_platform_debugger_stops_at_a_line() {
    let host = a0_native::Target::host().unwrap();
    let Some(debugger) = debugger().filter(|_| host.has_dwarf()) else {
        eprintln!("skipped: no gdb or lldb, or a host whose debuggers read PDB");
        return;
    };
    let dir = scratch("debugger");
    let out = a0(&dir, &["build", "lines.hir", "-g", "-o", "lines"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let commands: &[&str] = if debugger == "gdb" {
        &[
            "-batch",
            "-ex",
            "break lines.hir:4",
            "-ex",
            "run",
            "-ex",
            "bt",
            "./lines",
        ]
    } else {
        &[
            "--batch",
            "-o",
            "breakpoint set --file lines.hir --line 4",
            "-o",
            "run",
            "-o",
            "bt",
            "./lines",
        ]
    };
    let out = Command::new(debugger)
        .current_dir(&dir)
        .args(commands)
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    // Some machines do not let a debugger start processes (no developer mode on macOS).
    if ["launch failed", "attach failed", "not permitted"]
        .iter()
        .any(|why| text.contains(why))
    {
        eprintln!("skipped: {debugger} cannot start processes here:\n{text}");
        return;
    }
    // The backtrace names `norm` at line 4, called from `main`.
    assert!(text.contains("norm"), "{text}");
    assert!(text.contains("lines.hir:4"), "{text}");
    std::fs::remove_dir_all(&dir).ok();
}
