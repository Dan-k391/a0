//! Runs every program in `tests/programs` through each backend and checks its output.
//!
//! Expectations live in comments at the top of each file:
//! `; expect-stdout: <line>`, `; expect-stderr: <line>` (one per line) and `; expect-exit: <code>`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Expect {
    stdout: String,
    stderr: String,
    exit: i32,
}

fn expectations(text: &str) -> Expect {
    let mut e = Expect {
        stdout: String::new(),
        stderr: String::new(),
        exit: 0,
    };
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("; expect-stdout:") {
            e.stdout.push_str(rest.strip_prefix(' ').unwrap_or(rest));
            e.stdout.push('\n');
        } else if let Some(rest) = line.strip_prefix("; expect-stderr:") {
            e.stderr.push_str(rest.strip_prefix(' ').unwrap_or(rest));
            e.stderr.push('\n');
        } else if let Some(rest) = line.strip_prefix("; expect-exit:") {
            e.exit = rest.trim().parse().unwrap();
        }
    }
    e
}

fn programs() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/programs");
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "a0ir"))
        .collect();
    files.sort();
    files
}

fn scratch_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("a0-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Runs every program with `run` and checks the finished process against the expectations.
fn check_programs(run: &dyn Fn(&Path) -> Output) {
    let mut failures = Vec::new();
    let files = programs();
    assert!(!files.is_empty());
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap().replace("\r\n", "\n");
        let expect = expectations(&text);
        let out = run(file);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        let code = out.status.code().unwrap_or(-1);
        if stdout != expect.stdout || stderr != expect.stderr || code != expect.exit {
            failures.push(format!(
                "{}:\n  exit {code} (expected {})\n  stdout:\n{stdout}\n  expected stdout:\n{}\n  stderr:\n{stderr}\n  expected stderr:\n{}",
                file.display(),
                expect.exit,
                expect.stdout,
                expect.stderr
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn interpreted_programs_match_expectations() {
    check_programs(&|file| {
        Command::new(env!("CARGO_BIN_EXE_a0"))
            .arg("run")
            .arg(file)
            .output()
            .unwrap()
    });
}

#[test]
fn native_programs_match_expectations() {
    let dir = scratch_dir("native");
    check_programs(&|file| {
        let exe = dir.join(format!(
            "{}{}",
            file.file_stem().unwrap().to_string_lossy(),
            std::env::consts::EXE_SUFFIX
        ));
        let build = Command::new(env!("CARGO_BIN_EXE_a0"))
            .arg("build")
            .arg(file)
            .arg("-o")
            .arg(&exe)
            .output()
            .unwrap();
        assert!(
            build.status.success(),
            "building {} failed:\n{}",
            file.display(),
            String::from_utf8_lossy(&build.stderr)
        );
        Command::new(&exe).output().unwrap()
    });
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn check_and_fmt_report_diagnostics() {
    let dir = scratch_dir("cli");
    let bad = dir.join("bad.a0ir");
    std::fs::write(&bad, "module \"m\"\nfunc @f() -> i64 {\n  %x: f64 = fconst 1.0\n  return %x\n}\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_a0")).arg("check").arg(&bad).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("returns f64 but the function returns i64"), "{stderr}");
    assert!(stderr.contains("bad.a0ir:4:3"), "{stderr}");

    let good = dir.join("good.a0ir");
    std::fs::write(&good, "module \"m\"\nfunc @f()   ->   i64 {\n%x: i64 = iconst 1 ; one\nreturn %x\n}\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_a0")).arg("fmt").arg(&good).output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "module \"m\"\n\nfunc @f() -> i64 {\n  %x: i64 = iconst 1\n  return %x\n}\n"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn obj_emits_for_other_targets() {
    let dir = scratch_dir("obj");
    let file = programs().into_iter().find(|p| p.ends_with("hello.a0ir")).unwrap();
    let out_path = dir.join("hello.o");
    let out = Command::new(env!("CARGO_BIN_EXE_a0"))
        .args(["obj", "--target", "aarch64-linux", "-o"])
        .arg(&out_path)
        .arg(&file)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let bytes = std::fs::read(&out_path).unwrap();
    assert_eq!(&bytes[..4], b"\x7fELF");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn parse_prints_trees_and_errors() {
    let dir = scratch_dir("parse");
    let grammar = dir.join("calc.a0g");
    std::fs::write(
        &grammar,
        "grammar calc;
skip /[ ]+/;
token NUM /[0-9]+/;
program = sum EOF;
sum = NUM (\"+\" NUM)*;
",
    )
    .unwrap();
    let good = dir.join("good.txt");
    std::fs::write(&good, "1 + 2").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_a0")).arg("parse").arg(&grammar).arg(&good).output().unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), "(program (sum '1' '+' '2'))
");

    let bad = dir.join("bad.txt");
    std::fs::write(&bad, "1 + + 2").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_a0")).arg("parse").arg(&grammar).arg(&bad).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("expected NUM, found '+'"), "{stderr}");
    assert!(stderr.contains("bad.txt:1:5"), "{stderr}");
    std::fs::remove_dir_all(&dir).ok();
}
