//! Runs every program in `tests/programs` with `a0 run` and checks its output.
//!
//! Expectations live in comments at the top of each file:
//! `; expect-stdout: <line>`, `; expect-stderr: <line>` (one per line) and `; expect-exit: <code>`.

use std::path::{Path, PathBuf};
use std::process::Command;

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

#[test]
fn interpreted_programs_match_expectations() {
    let mut failures = Vec::new();
    let files = programs();
    assert!(!files.is_empty());
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap().replace("\r\n", "\n");
        let expect = expectations(&text);
        let out = Command::new(env!("CARGO_BIN_EXE_a0"))
            .arg("run")
            .arg(file)
            .output()
            .unwrap();
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
fn check_and_fmt_report_diagnostics() {
    let dir = std::env::temp_dir().join(format!("a0-cli-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
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
