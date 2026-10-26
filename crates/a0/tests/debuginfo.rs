//! `-g`: native code described for the platform's debuggers (DWARF; `docs/debugging.md`). The
//! line tables, functions and variables are read back from object files for Linux and macOS on
//! every host, and where gdb or lldb is installed, it stops at a breakpoint in a built executable
//! and prints variables.

use std::path::{Path, PathBuf};
use std::process::Command;

use object::{Object, ObjectSection, ObjectSymbol};

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

/// Variables of most kinds, all set by line 16.
const VARIABLES: &str = "module \"vars\";

struct Point { x: int, y: int }
struct Node { value: int, next: Node? }

fn main() -> int {
  let count = 42;
  let ratio = 2.5;
  let ok = true;
  let name = \"hello\";
  let p = new Point { x: 3, y: 4 };
  let list = new Node { value: 1, next: new Node { value: 2, next: null<Node> } };
  let xs = [10, 20, 30];
  let captured = 7;
  let get = fn() -> int { captured };
  print(name + \" \" + str(count + p.x + get() + xs[1]) + \"\\n\");
  0
}
";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("a0-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("lines.hir"), PROGRAM).unwrap();
    std::fs::write(dir.join("vars.hir"), VARIABLES).unwrap();
    dir
}

fn a0(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_a0"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}

/// A function's name and address, or a line table row's file, line and address, as written in an
/// object (before relocation).
type Function = (String, u64);
type Row = (String, u64, u64);

/// The functions an object's DWARF names, and the rows its line table has.
fn read_dwarf(bytes: &[u8]) -> (Vec<Function>, Vec<Row>) {
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
                && let Some(gimli::AttributeValue::Addr(low)) =
                    entry.attr_value(gimli::DW_AT_low_pc)
            {
                let name = dwarf.attr_string(&unit, name).unwrap();
                names.push((name.to_string_lossy().into_owned(), low));
            }
        }
        let Some(program) = unit.line_program.clone() else {
            continue;
        };
        let mut lines = program.rows();
        while let Some((header, row)) = lines.next_row().unwrap() {
            if let (Some(line), Some(file)) = (row.line(), row.file(header)) {
                let path = dwarf.attr_string(&unit, file.path_name()).unwrap();
                rows.push((
                    path.to_string_lossy().into_owned(),
                    line.get(),
                    row.address(),
                ));
            }
        }
    }
    (names, rows)
}

/// The variables an object's DWARF describes, with their types' names (a pointer's is its
/// target's, followed by `*`).
fn read_variables(bytes: &[u8]) -> Vec<(String, String)> {
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
    let mut vars = Vec::new();
    let mut units = dwarf.units();
    while let Some(header) = units.next().unwrap() {
        let unit = dwarf.unit(header).unwrap();
        let name_of = |offset: gimli::UnitOffset| -> String {
            let mut offset = offset;
            let mut suffix = String::new();
            loop {
                let entry = unit.entry(offset).unwrap();
                if let Some(name) = entry.attr_value(gimli::DW_AT_name) {
                    let name = dwarf.attr_string(&unit, name).unwrap();
                    return format!("{}{suffix}", name.to_string_lossy());
                }
                match entry.attr_value(gimli::DW_AT_type) {
                    Some(gimli::AttributeValue::UnitRef(to)) => {
                        suffix.push('*');
                        offset = to;
                    }
                    _ => return format!("?{suffix}"),
                }
            }
        };
        let mut entries = unit.entries();
        while let Some(entry) = entries.next_dfs().unwrap() {
            if entry.tag() == gimli::DW_TAG_variable
                && let Some(name) = entry.attr_value(gimli::DW_AT_name)
                && let Some(gimli::AttributeValue::UnitRef(ty)) =
                    entry.attr_value(gimli::DW_AT_type)
                && entry.attr_value(gimli::DW_AT_location).is_some()
            {
                let name = dwarf.attr_string(&unit, name).unwrap();
                vars.push((name.to_string_lossy().into_owned(), name_of(ty)));
            }
        }
    }
    vars
}

#[test]
fn objects_describe_variables_for_debuggers() {
    let dir = scratch("debugvars");
    for target in [
        "x86_64-linux",
        "aarch64-linux",
        "x86_64-macos",
        "aarch64-macos",
    ] {
        let out = a0(
            &dir,
            &["obj", "vars.hir", "-g", "--target", target, "-o", "vars.o"],
        );
        assert!(
            out.status.success(),
            "{target}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let vars = read_variables(&std::fs::read(dir.join("vars.o")).unwrap());
        for (name, ty) in [
            ("count", "int"),
            ("ratio", "float"),
            ("ok", "bool"),
            ("name", "string"),
            ("p", "Point*"),
            ("list", "Node*"),
            ("xs", "array<int>*"),
            ("captured", "int"),
        ] {
            assert!(
                vars.contains(&(name.to_string(), ty.to_string())),
                "{target}: no {name}: {ty} in {vars:?}"
            );
        }
    }
    std::fs::remove_dir_all(&dir).ok();
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
        let (functions, rows) = read_dwarf(&bytes);
        let names: Vec<&str> = functions.iter().map(|(n, _)| n.as_str()).collect();
        assert!(
            names.contains(&"norm") && names.contains(&"main"),
            "{target}: {names:?}"
        );
        for line in [3, 4, 5, 9, 10] {
            assert!(
                rows.iter()
                    .any(|(file, l, _)| file.ends_with("lines.hir") && *l == line),
                "{target}: no row for line {line} in {rows:?}"
            );
        }
        // dsymutil and lldb read Mach-O debug addresses as written: each must be where the
        // function is in the object, and line 4 must be inside `norm`. (ELF addresses are 0 until
        // the linker relocates them.)
        if target.ends_with("macos") {
            let file = object::File::parse(&*bytes).unwrap();
            let symbol = |name: &str| {
                file.symbols()
                    .find(|s| s.name() == Ok(name))
                    .map(|s| s.address())
                    .unwrap_or_else(|| panic!("{target}: no symbol {name}"))
            };
            let norm = symbol("_hir.norm");
            assert!(
                functions.contains(&("norm".to_string(), norm))
                    && functions.contains(&("main".to_string(), symbol("_hir.main"))),
                "{target}: {functions:?}"
            );
            // Mach-O symbols have no sizes: `norm` ends where the next symbol starts.
            let end = file
                .symbols()
                .map(|s| s.address())
                .filter(|&a| a > norm)
                .min()
                .unwrap();
            let line4 = rows.iter().find(|(_, l, _)| *l == 4).unwrap().2;
            assert!(
                (norm..end).contains(&line4),
                "{target}: line 4 at {line4:#x}, norm at {norm:#x}..{end:#x}"
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
    // What the debug information looked like, if the debugger did not find it.
    let mut text = text;
    if !text.contains("lines.hir:4") && cfg!(target_os = "macos") {
        for command in [
            &[
                "lldb",
                "--batch",
                "-o",
                "target create ./lines",
                "-o",
                "image list",
                "-o",
                "image dump line-table lines.hir",
            ][..],
            &["sh", "-c", "dsymutil -s ./lines | grep -E 'N_(OSO|FUN|SO)'"][..],
            &["dwarfdump", "--debug-info", "lines.dSYM"][..],
        ] {
            let out = Command::new(command[0])
                .args(&command[1..])
                .current_dir(&dir)
                .output();
            let shown = out.map_or_else(
                |e| e.to_string(),
                |o| {
                    format!(
                        "{}{}",
                        String::from_utf8_lossy(&o.stdout),
                        String::from_utf8_lossy(&o.stderr)
                    )
                },
            );
            let shown: String = shown.lines().take(40).collect::<Vec<_>>().join("\n");
            text.push_str(&format!("\n$ {}\n{shown}", command.join(" ")));
        }
    }
    // The backtrace names `norm` at line 4, called from `main`.
    assert!(text.contains("norm"), "{text}");
    assert!(text.contains("lines.hir:4"), "{text}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_platform_debugger_prints_variables() {
    let host = a0_native::Target::host().unwrap();
    let Some(debugger) = debugger().filter(|_| host.has_dwarf()) else {
        eprintln!("skipped: no gdb or lldb, or a host whose debuggers read PDB");
        return;
    };
    let dir = scratch("debugger-vars");
    let out = a0(&dir, &["build", "vars.hir", "-g", "-o", "vars"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let (batch, flag, breakpoint, print) = if debugger == "gdb" {
        ("-batch", "-ex", "break vars.hir:16", "print")
    } else {
        (
            "--batch",
            "-o",
            "breakpoint set --file vars.hir --line 16",
            "frame variable",
        )
    };
    let mut args = vec![batch.to_string()];
    let commands = [breakpoint.to_string(), "run".to_string()]
        .into_iter()
        .chain(
            ["count", "ratio", "ok", "name", "*p", "*list", "captured"]
                .iter()
                .map(|v| format!("{print} {v}")),
        );
    for c in commands {
        args.push(flag.to_string());
        args.push(c);
    }
    let out = Command::new(debugger)
        .current_dir(&dir)
        .args(&args)
        .arg("./vars")
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if ["launch failed", "attach failed", "not permitted"]
        .iter()
        .any(|why| text.contains(why))
    {
        eprintln!("skipped: {debugger} cannot start processes here:\n{text}");
        return;
    }
    for shown in [
        "42",
        "2.5",
        "true",
        "\"hello\"",
        "x = 3",
        "y = 4",
        "value = 1",
        "7",
    ] {
        assert!(
            text.contains(shown),
            "{debugger} did not show {shown}:\n{text}"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// The procedures and the variables (with their types' indices) of an object's CodeView, and
/// the line numbers its line tables have.
fn read_codeview(bytes: &[u8]) -> (Vec<String>, Vec<(String, u32)>, Vec<u32>) {
    let file = object::File::parse(bytes).unwrap();
    let data = file
        .section_by_name(".debug$S")
        .expect("no .debug$S")
        .data()
        .unwrap()
        .to_vec();
    let u16_at = |at: usize| u16::from_le_bytes([data[at], data[at + 1]]);
    let u32_at = |at: usize| u32::from_le_bytes(data[at..at + 4].try_into().unwrap());
    let name_at = |at: usize| {
        let end = data[at..].iter().position(|&b| b == 0).unwrap();
        String::from_utf8_lossy(&data[at..at + end]).into_owned()
    };
    assert_eq!(u32_at(0), 4, "CodeView signature");
    let (mut procs, mut vars, mut lines) = (Vec::new(), Vec::new(), Vec::new());
    let mut at = 4;
    while at < data.len() {
        let (kind, len) = (u32_at(at), u32_at(at + 4) as usize);
        let body = at + 8;
        if kind == 0xf1 {
            let mut r = body;
            while r < body + len {
                let (size, rec) = (u16_at(r) as usize, u16_at(r + 2));
                match rec {
                    0x1110 => procs.push(name_at(r + 4 + 35)),
                    0x1111 => vars.push((name_at(r + 4 + 10), u32_at(r + 8))),
                    _ => {}
                }
                r += 2 + size;
            }
        } else if kind == 0xf2 {
            let mut b = body + 12;
            while b < body + len {
                let count = u32_at(b + 4) as usize;
                for i in 0..count {
                    lines.push(u32_at(b + 12 + 8 * i + 4) & 0xff_ffff);
                }
                b += u32_at(b + 8) as usize;
            }
        }
        at = (body + len).next_multiple_of(4);
    }
    (procs, vars, lines)
}

#[test]
fn objects_describe_functions_lines_and_variables_for_windows_debuggers() {
    let dir = scratch("codeview");
    for target in ["x86_64-windows", "aarch64-windows"] {
        let out = a0(
            &dir,
            &[
                "obj", "vars.hir", "-g", "--target", target, "-o", "vars.obj",
            ],
        );
        assert!(
            out.status.success(),
            "{target}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let bytes = std::fs::read(dir.join("vars.obj")).unwrap();
        let (procs, vars, lines) = read_codeview(&bytes);
        assert!(procs.iter().any(|p| p == "main"), "{target}: {procs:?}");
        // 64-bit integers, doubles and booleans are CodeView's own types; objects are pointers
        // to the program's structures (indices from 0x1000).
        for (name, ty) in [
            ("count", Some(0x76)),
            ("ratio", Some(0x41)),
            ("ok", Some(0x30)),
        ]
        .into_iter()
        .chain(["name", "p", "list", "xs", "captured"].map(|n| (n, None)))
        {
            assert!(
                vars.iter().any(|(n, t)| n == name
                    && ty.is_none_or(|ty| *t == ty)
                    && (ty.is_some() || *t >= 0x1000)),
                "{target}: no {name} in {vars:?}"
            );
        }
        for line in [7, 11, 16] {
            assert!(
                lines.contains(&line),
                "{target}: no line {line} in {lines:?}"
            );
        }
        let file = object::File::parse(&*bytes).unwrap();
        assert!(file.section_by_name(".debug$T").is_some(), "{target}");
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// cdb, Windows' console debugger: on the path, from the Windows SDK, or from WinDbg.
fn cdb() -> Option<PathBuf> {
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x64"
    };
    let mut candidates = vec![PathBuf::from("cdb.exe")];
    for root in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Some(dir) = std::env::var_os(root) {
            candidates
                .push(Path::new(&dir).join(format!("Windows Kits/10/Debuggers/{arch}/cdb.exe")));
        }
    }
    if let Some(dir) = std::env::var_os("LOCALAPPDATA") {
        let name = if arch == "arm64" {
            "cdbARM64.exe"
        } else {
            "cdbX64.exe"
        };
        candidates.push(Path::new(&dir).join("Microsoft/WindowsApps").join(name));
    }
    candidates.into_iter().find(|c| {
        Command::new(c)
            .arg("-version")
            .output()
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("cdb version"))
    })
}

#[test]
fn the_windows_debugger_stops_at_a_line_and_shows_variables() {
    if !cfg!(windows) {
        return;
    }
    let Some(cdb) = cdb() else {
        eprintln!("skipped: no cdb (Windows SDK debugging tools or WinDbg)");
        return;
    };
    let dir = scratch("cdb");
    let out = a0(&dir, &["build", "vars.hir", "-g", "-o", "vars.exe"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(dir.join("vars.pdb").exists());
    let out = Command::new(&cdb)
        .current_dir(&dir)
        .arg("-lines")
        .arg("-y")
        .arg(&dir)
        .arg("-c")
        .arg("bp `vars!vars.hir:16`;g;k 1;dv /t;dx -r1 *p;dx xs;dx captured;q")
        .arg(dir.join("vars.exe"))
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    for shown in [
        "vars.hir @ 16",
        "count = 0n42",
        "ratio = 2.5",
        "ok = true",
        "\"hello\"",
        "x                : 3",
        "{ len=3 }",
        "value            : 7",
    ] {
        assert!(text.contains(shown), "cdb did not show {shown}:\n{text}");
    }
    std::fs::remove_dir_all(&dir).ok();
}
