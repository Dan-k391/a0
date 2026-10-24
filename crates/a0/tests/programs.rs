//! Runs every program in `tests/programs` (core IR) and `tests/hir` (high-level IR) through each
//! backend and checks its output.
//!
//! Expectations live in comments at the top of each file:
//! `; expect-stdout: <line>`, `; expect-stderr: <line>` (one per line) and `; expect-exit: <code>`,
//! written with `//` instead of `;` in `.hir` files.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Runs a program, killing it after a minute: a collector bug can make a program loop forever, and
/// the test should then fail rather than hang.
fn run(cmd: &mut Command) -> Output {
    use std::io::Read;
    use std::process::Stdio;
    use std::time::{Duration, Instant};
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut out = child.stdout.take().unwrap();
    let mut err = child.stderr.take().unwrap();
    let stdout = std::thread::spawn(move || {
        let mut v = Vec::new();
        out.read_to_end(&mut v).ok();
        v
    });
    let stderr = std::thread::spawn(move || {
        let mut v = Vec::new();
        err.read_to_end(&mut v).ok();
        v
    });
    let start = Instant::now();
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if start.elapsed() > Duration::from_secs(60) {
            child.kill().ok();
            timed_out = true;
            break child.wait().unwrap();
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    let mut stderr = stderr.join().unwrap();
    if timed_out {
        stderr.extend_from_slice(b"\n(killed after 60 seconds)\n");
    }
    Output {
        status,
        stdout: stdout.join().unwrap(),
        stderr,
    }
}

struct Expect {
    stdout: String,
    stderr: String,
    exit: i32,
}

fn expectations(text: &str, comment: &str) -> Expect {
    let mut e = Expect {
        stdout: String::new(),
        stderr: String::new(),
        exit: 0,
    };
    let key = |k: &str| format!("{comment} {k}:");
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix(&key("expect-stdout")) {
            e.stdout.push_str(rest.strip_prefix(' ').unwrap_or(rest));
            e.stdout.push('\n');
        } else if let Some(rest) = line.strip_prefix(&key("expect-stderr")) {
            e.stderr.push_str(rest.strip_prefix(' ').unwrap_or(rest));
            e.stderr.push('\n');
        } else if let Some(rest) = line.strip_prefix(&key("expect-exit")) {
            e.exit = rest.trim().parse().unwrap();
        }
    }
    e
}

/// Every program: core IR in `tests/programs`, HIR in `tests/hir`.
fn programs() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for (dir, ext) in [("tests/programs", "a0ir"), ("tests/hir", "hir")] {
        for entry in std::fs::read_dir(root.join(dir)).unwrap() {
            let p = entry.unwrap().path();
            if p.extension().is_some_and(|x| x == ext) {
                files.push(p);
            }
        }
    }
    files.sort();
    assert!(
        files
            .iter()
            .any(|f| f.extension().is_some_and(|x| x == "hir"))
    );
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
        let comment = if file.extension().is_some_and(|x| x == "hir") {
            "//"
        } else {
            ";"
        };
        let expect = expectations(&text, comment);
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
    check_programs(&|file| run(Command::new(env!("CARGO_BIN_EXE_a0")).arg("run").arg(file)));
}

#[test]
fn interpreted_programs_survive_gc_stress() {
    check_programs(&|file| {
        run(Command::new(env!("CARGO_BIN_EXE_a0"))
            .arg("run")
            .arg(file)
            .env("A0_GC_STRESS", "1"))
    });
}

#[test]
fn native_programs_match_expectations() {
    let dir = scratch_dir("native");
    check_programs(&|file| {
        let exe = dir.join(format!(
            "{}{}",
            file.file_name()
                .unwrap()
                .to_string_lossy()
                .replace('.', "_"),
            std::env::consts::EXE_SUFFIX
        ));
        let build = run(Command::new(env!("CARGO_BIN_EXE_a0"))
            .arg("build")
            .arg(file)
            .arg("-o")
            .arg(&exe));
        assert!(
            build.status.success(),
            "building {} failed:\n{}",
            file.display(),
            String::from_utf8_lossy(&build.stderr)
        );
        run(&mut Command::new(&exe))
    });
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn check_and_fmt_report_diagnostics() {
    let dir = scratch_dir("cli");
    let bad = dir.join("bad.a0ir");
    std::fs::write(
        &bad,
        "module \"m\"\nfunc @f() -> i64 {\n  %x: f64 = fconst 1.0\n  return %x\n}\n",
    )
    .unwrap();
    let out = run(Command::new(env!("CARGO_BIN_EXE_a0"))
        .arg("check")
        .arg(&bad));
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("returns f64 but the function returns i64"),
        "{stderr}"
    );
    assert!(stderr.contains("bad.a0ir:4:3"), "{stderr}");

    let good = dir.join("good.a0ir");
    std::fs::write(
        &good,
        "module \"m\"\nfunc @f()   ->   i64 {\n%x: i64 = iconst 1 ; one\nreturn %x\n}\n",
    )
    .unwrap();
    let out = run(Command::new(env!("CARGO_BIN_EXE_a0")).arg("fmt").arg(&good));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "module \"m\"\n\nfunc @f() -> i64 {\n  %x: i64 = iconst 1\n  return %x\n}\n"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn obj_emits_for_other_targets() {
    let dir = scratch_dir("obj");
    let file = programs()
        .into_iter()
        .find(|p| p.ends_with("hello.a0ir"))
        .unwrap();
    let out_path = dir.join("hello.o");
    let out = run(Command::new(env!("CARGO_BIN_EXE_a0"))
        .args(["obj", "--target", "aarch64-linux", "-o"])
        .arg(&out_path)
        .arg(&file));
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
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
    let out = run(Command::new(env!("CARGO_BIN_EXE_a0"))
        .arg("parse")
        .arg(&grammar)
        .arg(&good));
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "(program (sum '1' '+' '2'))
"
    );

    let bad = dir.join("bad.txt");
    std::fs::write(&bad, "1 + + 2").unwrap();
    let out = run(Command::new(env!("CARGO_BIN_EXE_a0"))
        .arg("parse")
        .arg(&grammar)
        .arg(&bad));
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("expected NUM, found '+'"), "{stderr}");
    assert!(stderr.contains("bad.txt:1:5"), "{stderr}");
    std::fs::remove_dir_all(&dir).ok();
}
