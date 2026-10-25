//! Snapshot tests of what a language's command prints: every program in a directory is run, and
//! its output (diagnostics rendered in full, the program's own output, the exit status) must be
//! exactly what the `.out` file next to it says. Programs that compile are also built natively, and
//! the executable must print and exit the same way as the interpreter.
//!
//! `A0_BLESS=1` writes the `.out` files from what the programs print; review the changes before
//! committing them.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// What a run printed, as a snapshot: stdout, stderr and the exit status, each in a section.
fn snapshot(command: &str, out: &Output) -> String {
    let text = |b: &[u8]| String::from_utf8_lossy(b).replace("\r\n", "\n");
    let mut s = format!("$ {command}\n");
    let stdout = text(&out.stdout);
    if !stdout.is_empty() {
        s += "--- stdout\n";
        s += &stdout;
        if !stdout.ends_with('\n') {
            s.push('\n');
        }
    }
    let stderr = text(&out.stderr);
    if !stderr.is_empty() {
        s += "--- stderr\n";
        s += &stderr;
        if !stderr.ends_with('\n') {
            s.push('\n');
        }
    }
    s += &format!("--- exit {}\n", out.status.code().unwrap_or(-1));
    s
}

fn run(cmd: &mut Command) -> Output {
    // Snapshots have no colors, whoever runs the tests.
    cmd.env_remove("CLICOLOR_FORCE")
        .stdin(Stdio::null())
        .output()
        .unwrap_or_else(|e| panic!("cannot run {cmd:?}: {e}"))
}

/// Checks every `*.{extension}` program in `dir` against its `.out` file, running `binary run` (and
/// `binary build`, for programs that compile, when `native` is set) with `dir` as the working
/// directory so that paths print as written. Returns a description of each difference.
pub fn check(binary: &Path, dir: &Path, extension: &str, native: bool) -> Vec<String> {
    let bless = std::env::var("A0_BLESS").is_ok_and(|v| v == "1");
    let name = binary
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == extension))
        .collect();
    files.sort();
    assert!(
        !files.is_empty(),
        "no .{extension} programs in {}",
        dir.display()
    );
    let scratch = std::env::temp_dir().join(format!("a0-snapshots-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let mut failures = Vec::new();
    for file in files {
        let file_name = file.file_name().unwrap().to_string_lossy().to_string();
        let out = run(Command::new(binary)
            .arg("run")
            .arg(&file_name)
            .current_dir(dir));
        let got = snapshot(&format!("{name} run {file_name}"), &out);
        let expected_path = file.with_extension("out");
        let expected = std::fs::read_to_string(&expected_path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        if got != expected {
            if bless {
                std::fs::write(&expected_path, &got).unwrap();
            } else {
                failures.push(format!(
                    "{}: the output differs from {}:\n{got}",
                    file.display(),
                    expected_path.display()
                ));
            }
        }
        if !native {
            continue;
        }
        let exe = scratch.join(format!(
            "{}{}",
            file.file_stem().unwrap().to_string_lossy(),
            std::env::consts::EXE_SUFFIX
        ));
        let build = run(Command::new(binary)
            .arg("build")
            .arg(&file_name)
            .arg("-o")
            .arg(&exe)
            .current_dir(dir));
        if !build.status.success() {
            continue;
        }
        let native_out = run(Command::new(&exe).current_dir(dir));
        // The interpreter prints the compiler's warnings before the program runs; the executable
        // only what the program prints.
        let interpreted_stderr = String::from_utf8_lossy(&out.stderr).replace("\r\n", "\n");
        let warnings = String::from_utf8_lossy(&build.stderr).replace("\r\n", "\n");
        let program_stderr = interpreted_stderr
            .strip_prefix(warnings.as_str())
            .unwrap_or(&interpreted_stderr);
        let native_stderr = String::from_utf8_lossy(&native_out.stderr).replace("\r\n", "\n");
        if native_out.stdout != out.stdout
            || native_stderr != program_stderr
            || native_out.status.code() != out.status.code()
        {
            failures.push(format!(
                "{}: native code differs from the interpreter:\n{}",
                file.display(),
                snapshot("(native)", &native_out)
            ));
        }
    }
    std::fs::remove_dir_all(&scratch).ok();
    failures
}
