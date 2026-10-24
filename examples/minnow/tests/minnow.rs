//! Runs every program in `tests/programs` interpreted, as a native executable, and interpreted with
//! the collector running on every allocation, and checks compile errors.
//!
//! Programs state what they expect in comments: `// expect: <line>` for each line of stdout,
//! `// expect-stderr: <line>` and `// expect-exit: <code>`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use a0_diag::SourceMap;

/// Runs a program, killing it after a minute so that a collector bug fails the test instead of
/// hanging it.
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
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if start.elapsed() > Duration::from_secs(60) {
            child.kill().ok();
            break child.wait().unwrap();
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    Output {
        status,
        stdout: stdout.join().unwrap(),
        stderr: stderr.join().unwrap(),
    }
}

fn programs() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/programs");
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "mn"))
        .collect();
    files.sort();
    assert!(files.len() >= 5, "the test programs are missing");
    files
}

/// Compares a run with the program's expectations, returning a failure message.
fn check(file: &Path, out: &Output, mode: &str) -> Option<String> {
    let text = std::fs::read_to_string(file).unwrap().replace("\r\n", "\n");
    let (mut stdout, mut stderr, mut exit) = (String::new(), String::new(), 0);
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("// expect: ") {
            stdout.push_str(rest);
            stdout.push('\n');
        } else if let Some(rest) = line.strip_prefix("// expect-stderr: ") {
            stderr.push_str(rest);
            stderr.push('\n');
        } else if let Some(rest) = line.strip_prefix("// expect-exit: ") {
            exit = rest.trim().parse().unwrap();
        }
    }
    let got_stdout = String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n");
    let got_stderr = String::from_utf8_lossy(&out.stderr).replace("\r\n", "\n");
    let got_exit = out.status.code().unwrap_or(-1);
    if got_stdout == stdout && got_stderr == stderr && got_exit == exit {
        None
    } else {
        Some(format!(
            "{} ({mode}): exit {got_exit} (expected {exit})\nstdout:\n{got_stdout}expected:\n{stdout}stderr:\n{got_stderr}expected:\n{stderr}",
            file.display()
        ))
    }
}

fn check_all(mode: &str, run_one: &dyn Fn(&Path) -> Output) {
    let failures: Vec<String> = programs()
        .iter()
        .filter_map(|f| check(f, &run_one(f), mode))
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn programs_run_in_the_interpreter() {
    check_all("interpreted", &|f| {
        run(Command::new(env!("CARGO_BIN_EXE_minnow")).arg("run").arg(f))
    });
}

#[test]
fn programs_survive_gc_stress() {
    check_all("GC stress", &|f| {
        run(Command::new(env!("CARGO_BIN_EXE_minnow"))
            .arg("run")
            .arg(f)
            .env("A0_GC_STRESS", "1"))
    });
}

#[test]
fn programs_run_natively() {
    let dir = std::env::temp_dir().join(format!("minnow-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    check_all("native", &|f| {
        let exe = dir.join(format!(
            "{}{}",
            f.file_stem().unwrap().to_string_lossy(),
            std::env::consts::EXE_SUFFIX
        ));
        let build = run(Command::new(env!("CARGO_BIN_EXE_minnow"))
            .arg("build")
            .arg(f)
            .arg("-o")
            .arg(&exe));
        if !build.status.success() {
            return build;
        }
        run(&mut Command::new(&exe))
    });
    std::fs::remove_dir_all(&dir).ok();
}

/// Compiles `src` and returns each error as `line:col: message`.
fn errors(src: &str) -> Vec<String> {
    let mut sm = SourceMap::new();
    let id = sm.add("test.mn", src);
    match minnow::compile(src, id) {
        Ok(_) => Vec::new(),
        Err(d) => d
            .as_slice()
            .iter()
            .map(|d| match d.span {
                Some(s) => {
                    let (line, col) = sm.line_col(id, s.start);
                    format!("{line}:{col}: {}", d.message)
                }
                None => d.message.clone(),
            })
            .collect(),
    }
}

#[test]
fn compile_errors_are_reported_with_locations() {
    let main = "fn main() {}\n";
    let cases: &[(&str, &str)] = &[
        (
            "fn main() { let x: int = \"a\"; }",
            "1:26: mismatched types: expected `int`, found `string`",
        ),
        ("fn main() { print(y); }", "1:19: unknown name `y`"),
        ("fn f(p: Pt) {}", "1:9: unknown type `Pt`"),
        (
            "fn f(a: int) {} fn main() { f(1, 2); }",
            "1:29: expected 1 argument, found 2",
        ),
        (
            "fn main() { let x = 1; x(2); }",
            "1:24: `int` is not a function",
        ),
        (
            "fn f() -> int { if true { return 1; } }",
            "1:4: this function can end without returning a value of type `int`",
        ),
        (
            "fn main() { break; }",
            "1:13: `break` and `continue` can only be used inside a loop",
        ),
        (
            "fn main() { 1 = 2; }",
            "1:13: cannot assign to this expression",
        ),
        (
            "fn main() { main = main; }",
            "1:13: cannot assign to this expression",
        ),
        (
            "struct P { x: int } fn main() { let p = new P { x: 1 }; print(p.z); }",
            "1:65: `P` has no field `z`",
        ),
        (
            "struct P { x: int, y: int } fn main() { let p = new P { x: 1 }; }",
            "1:53: missing fields in `P`: `y`",
        ),
        (
            "fn main() { print(\"a\" - \"b\"); }",
            "1:19: operator `-` cannot be applied to `string`",
        ),
        (
            "fn main() { let xs = []; }",
            "1:22: cannot infer the element type of this array",
        ),
        ("fn not_main() {}", "the program has no `main` function"),
        (
            "fn main(x: int) {}",
            "1:4: `main` must take no parameters and return nothing or `int`",
        ),
        (
            "fn f() {} fn f() {}",
            "1:14: the function `f` is defined twice",
        ),
        (
            "fn main() { let a = 1; let a = 2; }",
            "1:28: `a` is already defined in this scope",
        ),
        (
            "fn main() { let n = \"a\" as int; }",
            "1:21: cannot cast `string` to `int`",
        ),
        (
            "fn main() { return 1; }",
            "1:20: mismatched types: expected `unit`, found `int`",
        ),
        (
            "fn f() -> int { return; }",
            "1:17: this function must return a value of type `int`",
        ),
        (
            "fn main() { print([1]); }",
            "1:19: `print` does not accept `[int]`",
        ),
        (
            "fn main() { print(\"\\q\"); }",
            "1:19: unknown escape `\\q`",
        ),
        (
            "fn main() { if 1 { } }",
            "1:16: mismatched types: expected `bool`, found `int`",
        ),
        (
            "fn main() { let xs = [1, true]; }",
            "1:26: mismatched types: expected `int`, found `bool`",
        ),
        (
            "fn main() { let f = fn(x: int) -> int { return x; }; f(true); }",
            "1:56: mismatched types: expected `int`, found `bool`",
        ),
        ("fn main() { let x = 1 }", "1:23: expected ';'"),
    ];
    let mut failures = Vec::new();
    for (src, expected) in cases {
        let src = if src.contains("main") {
            src.to_string()
        } else {
            format!("{src}\n{main}")
        };
        let got = errors(&src);
        if !got.iter().any(|e| e.starts_with(expected)) {
            failures.push(format!("{src}\n  expected: {expected}\n  got: {got:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn errors_do_not_cascade() {
    // One unknown name gives one error, not a chain of mismatches from its unknown type.
    let got = errors("fn main() { let x = y; print(x.f); x(); print(x); }");
    assert_eq!(got, vec!["1:21: unknown name `y`".to_string()]);
    let got = errors("struct P { x: int } fn main() { let p = new P { x: 1 }; print(p.z); }");
    assert_eq!(got, vec!["1:65: `P` has no field `z`".to_string()]);
    // Without an earlier error, an unknown type is the program's problem.
    let got = errors("fn main() { let xs = []; print(xs[0]); push(xs, 1); }");
    assert_eq!(
        got,
        vec!["1:32: the type of this value must be known here; add a type annotation".to_string()]
    );
}

#[test]
fn emitted_hir_round_trips_through_text() {
    for f in programs() {
        let text = std::fs::read_to_string(&f).unwrap();
        let mut sm = SourceMap::new();
        let id = sm.add("test.mn", &text);
        let printed = minnow::compile(&text, id).unwrap().to_string();
        let hir_id = sm.add("test.hir", &printed);
        let reparsed = match a0_hir::parse(&printed, hir_id) {
            Ok(m) => m,
            Err(d) => panic!("{}: {}\n{printed}", f.display(), d.render(&sm)),
        };
        assert_eq!(reparsed.to_string(), printed, "{}", f.display());
    }
}
