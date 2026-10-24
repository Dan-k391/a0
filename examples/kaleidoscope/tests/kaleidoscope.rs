//! Runs every program in `tests/programs` interpreted and as a native executable, and checks
//! compile errors. Expected output lines are written as `# expect: <line>` comments.

use std::path::{Path, PathBuf};
use std::process::Command;

use a0_diag::SourceMap;

fn programs() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/programs");
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "k"))
        .collect();
    files.sort();
    files
}

fn expected_stdout(file: &Path) -> String {
    let text = std::fs::read_to_string(file).unwrap().replace("\r\n", "\n");
    let mut out = String::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("# expect: ") {
            out.push_str(rest);
            out.push('\n');
        }
    }
    out
}

fn check(file: &Path, output: std::process::Output, mode: &str) -> Option<String> {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let expected = expected_stdout(file);
    if output.status.success() && stdout == expected {
        None
    } else {
        Some(format!(
            "{} ({mode}): exit {:?}\nstdout:\n{stdout}\nexpected:\n{expected}\nstderr:\n{}",
            file.display(),
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

#[test]
fn programs_run_in_the_interpreter() {
    let failures: Vec<String> = programs()
        .iter()
        .filter_map(|f| {
            let out = Command::new(env!("CARGO_BIN_EXE_kaleidoscope"))
                .arg("run")
                .arg(f)
                .output()
                .unwrap();
            check(f, out, "interpreted")
        })
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn programs_run_natively() {
    let dir = std::env::temp_dir().join(format!("kaleidoscope-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let failures: Vec<String> = programs()
        .iter()
        .filter_map(|f| {
            let exe = dir.join(format!(
                "{}{}",
                f.file_stem().unwrap().to_string_lossy(),
                std::env::consts::EXE_SUFFIX
            ));
            let build = Command::new(env!("CARGO_BIN_EXE_kaleidoscope"))
                .arg("build")
                .arg(f)
                .arg("-o")
                .arg(&exe)
                .output()
                .unwrap();
            if !build.status.success() {
                return Some(format!(
                    "{}: build failed\n{}",
                    f.display(),
                    String::from_utf8_lossy(&build.stderr)
                ));
            }
            check(f, Command::new(&exe).output().unwrap(), "native")
        })
        .collect();
    std::fs::remove_dir_all(&dir).ok();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Compiles `src` and returns the rendered diagnostics (empty if it compiled).
fn errors(src: &str) -> String {
    let mut sm = SourceMap::new();
    let id = sm.add("test.k", src);
    match kaleidoscope::compile(src, id) {
        Ok(_) => String::new(),
        Err(d) => d.render(&sm),
    }
}

#[test]
fn compile_errors_are_reported_with_locations() {
    let cases: &[(&str, &str)] = &[
        ("def f(x) y;", "unknown variable 'y'"),
        ("def f(x) g(x);", "unknown function 'g'"),
        ("def f(x y) x; f(1);", "'f' takes 2 argument(s) but 1 were given"),
        ("def f(x) x | 1;", "unknown binary operator '|'"),
        ("def f(x) !x;", "unknown unary operator '!'"),
        ("def f(x) x; def f(x) x;", "function 'f' cannot be redefined"),
        ("extern f(x); def f(x y) x;", "function 'f' was declared with 1 parameter(s), not 2"),
        ("def binary| 200 (a b) a;", "invalid precedence"),
        ("def binary| (a) a;", "a binary operator takes 2 operands, not 1"),
        ("def binary+ (a b) a;", "cannot redefine the built-in operator '+'"),
        ("def f(x) 1 + 2 = x;", "destination of '=' must be a variable"),
        ("def main() 1;", "'main' is reserved"),
        ("def f(x x) x;", "duplicate parameter 'x'"),
        ("def f(x) if x then 1;", "expected 'else'"),
        ("def f(x) (x + 1;", "expected ')'"),
        ("def f(x) for i = 1 in x;", "expected ',' after the start value"),
    ];
    let mut failures = Vec::new();
    for (src, want) in cases {
        let got = errors(src);
        if !got.contains(want) || !got.contains("test.k:1:") {
            failures.push(format!("{src}\n  want: {want}\n  got: {got}"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn several_errors_are_reported_at_once() {
    // Name errors in different functions are all reported.
    let out = errors("def f(x) y;\ndef g(x) x;\ndef h(x) z;\n");
    assert!(out.contains("unknown variable 'y'"), "{out}");
    assert!(out.contains("unknown variable 'z'"), "{out}");
    // A syntax error stops compilation before name resolution, but recovery still finds every
    // syntax error in the file.
    let out = errors("def f(x) (x;\ndef g() (1;\n");
    assert_eq!(out.matches("expected ')'").count(), 2, "{out}");
}

#[test]
fn operators_must_be_defined_before_use() {
    let out = errors("def f(a b) a | b;\ndef binary| 5 (a b) a;\n");
    assert!(out.contains("unknown binary operator '|'"), "{out}");
    assert_eq!(errors("def binary| 5 (a b) a;\ndef f(a b) a | b;\n"), "");
}

#[test]
fn ir_output_is_valid_text() {
    let out = Command::new(env!("CARGO_BIN_EXE_kaleidoscope"))
        .arg("ir")
        .arg(programs().into_iter().find(|p| p.ends_with("fib.k")).unwrap())
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("func @fibi(%0: f64) -> f64 {"), "{text}");
    // The printed IR parses and verifies on its own.
    a0_ir::parse_and_verify(&text, a0_diag::SourceId(0)).unwrap();
}
