//! The official Lox test suite (`tests/craftinginterpreters/test`), checked the way the book's
//! `tool/bin/test.dart` checks the `jlox` suite.
//!
//! Each test file states what it expects in comments: `// expect: output`, `// Error ...` and
//! `// [line N] Error ...` for compile errors (exit 65), `// expect runtime error: message` for runtime
//! errors (exit 70). Tests run interpreted, as native executables, and interpreted with the garbage
//! collector running on every allocation.
//!
//! Skipped, as in the jlox suite: the early-chapter tests (`scanning`, `expressions`), the benchmarks,
//! and clox's bytecode limits (`limit/` except `stack_overflow.lox`). Two tests that the jlox suite skips
//! because of JVM behavior run here and pass: `number/nan_equality.lox` and `limit/stack_overflow.lox`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Mutex;

const SKIPPED: &[&str] = &[
    "test/scanning",
    "test/expressions",
    "test/benchmark",
    "test/limit/loop_too_large.lox",
    "test/limit/no_reuse_constants.lox",
    "test/limit/too_many_constants.lox",
    "test/limit/too_many_locals.lox",
    "test/limit/too_many_upvalues.lox",
];

fn suite_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/craftinginterpreters")
}

/// Every test file, as `(path relative to the suite root with forward slashes, absolute path)`.
fn tests() -> Vec<(String, PathBuf)> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, PathBuf)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, root, out);
            } else if path.extension().is_some_and(|e| e == "lox") {
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((rel, path));
            }
        }
    }
    let root = suite_dir();
    let mut out = Vec::new();
    walk(&root.join("test"), &root, &mut out);
    out.sort();
    out.retain(|(rel, _)| !SKIPPED.iter().any(|s| rel == s || rel.starts_with(&format!("{s}/"))));
    out
}

#[derive(Default)]
struct Expectations {
    output: Vec<(usize, String)>,
    errors: Vec<String>,
    runtime_error: Option<(usize, String)>,
    exit_code: i32,
}

/// Parses the expectation comments exactly like test.dart's regular expressions do.
fn expectations(text: &str) -> Option<Expectations> {
    let mut e = Expectations::default();
    for (i, line) in text.lines().enumerate() {
        let line_num = i + 1;
        if line.contains("// nontest") {
            return None;
        }
        if let Some(pos) = line.find("// expect: ").or_else(|| line.find("// expect:")) {
            let rest = &line[pos + "// expect:".len()..];
            let rest = rest.strip_prefix(' ').unwrap_or(rest);
            e.output.push((line_num, rest.to_string()));
            continue;
        }
        if let Some(pos) = line.find("// Error") {
            e.errors.push(format!("[{line_num}] {}", &line[pos + 3..]));
            e.exit_code = 65;
            continue;
        }
        if let Some(pos) = line.find("// [") {
            // // [line N] Error..., // [java line N] Error..., // [c line N] Error...
            let rest = &line[pos + 4..];
            let (language, rest) = if let Some(r) = rest.strip_prefix("java ") {
                (Some("java"), r)
            } else if let Some(r) = rest.strip_prefix("c ") {
                (Some("c"), r)
            } else {
                (None, rest)
            };
            if let Some(r) = rest.strip_prefix("line ") {
                let digits: String = r.chars().take_while(char::is_ascii_digit).collect();
                let after = &r[digits.len()..];
                if let Some(msg) = after.strip_prefix("] ") {
                    if msg.starts_with("Error") && !digits.is_empty() {
                        if language.is_none() || language == Some("java") {
                            e.errors.push(format!("[{digits}] {msg}"));
                            e.exit_code = 65;
                        }
                        continue;
                    }
                }
            }
        }
        if let Some(pos) = line.find("// expect runtime error: ") {
            let msg = &line[pos + "// expect runtime error: ".len()..];
            e.runtime_error = Some((line_num, msg.to_string()));
            e.exit_code = 70;
        }
    }
    Some(e)
}

/// `\[.*line (\d+)\] (Error.+)` from test.dart: the "[N] Error..." form of a compile error line.
fn syntax_error(line: &str) -> Option<String> {
    let start = line.find('[')?;
    let rest = &line[start..];
    let at = rest.find("line ")?;
    let digits: String = rest[at + 5..].chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    let after = &rest[at + 5 + digits.len()..];
    let msg = after.strip_prefix("] ")?;
    (msg.starts_with("Error") && msg.len() > "Error".len()).then(|| format!("[{digits}] {msg}"))
}

/// `\[line (\d+)\]` from test.dart.
fn stack_trace_line(line: &str) -> Option<usize> {
    let pos = line.find("[line ")?;
    let digits: String = line[pos + 6..].chars().take_while(char::is_ascii_digit).collect();
    let after = &line[pos + 6 + digits.len()..];
    if digits.is_empty() || !after.starts_with(']') {
        return None;
    }
    digits.parse().ok()
}

/// Checks one run against the expectations, returning the failures (test.dart's messages).
fn validate(e: &Expectations, out: &Output) -> Vec<String> {
    let mut failures = Vec::new();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let mut output_lines: Vec<&str> = stdout.lines().collect();
    let error_lines: Vec<&str> = stderr.lines().collect();

    if let Some((line, msg)) = &e.runtime_error {
        if error_lines.len() < 2 {
            failures.push(format!("Expected runtime error '{msg}' and got none."));
        } else {
            if error_lines[0] != msg {
                failures.push(format!("Expected runtime error '{msg}' and got: {}", error_lines[0]));
            }
            match error_lines[1..].iter().find_map(|l| stack_trace_line(l)) {
                None => failures.push(format!("Expected stack trace and got: {:?}", &error_lines[1..])),
                Some(n) if n != *line => {
                    failures.push(format!("Expected runtime error on line {line} but was on line {n}."))
                }
                _ => {}
            }
        }
    } else {
        let mut found = Vec::new();
        for l in &error_lines {
            match syntax_error(l) {
                Some(err) if e.errors.contains(&err) => found.push(err),
                Some(_) => failures.push(format!("Unexpected error: {l}")),
                None if !l.is_empty() => failures.push(format!("Unexpected output on stderr: {l}")),
                None => {}
            }
        }
        for err in &e.errors {
            if !found.contains(err) {
                failures.push(format!("Missing expected error: {err}"));
            }
        }
    }

    let code = out.status.code().unwrap_or(-1);
    if code != e.exit_code {
        failures.push(format!(
            "Expected return code {} and got {code}. Stderr: {:?}",
            e.exit_code, error_lines
        ));
    }

    if output_lines.last() == Some(&"") {
        output_lines.pop();
    }
    for (i, line) in output_lines.iter().enumerate() {
        match e.output.get(i) {
            None => failures.push(format!("Got output '{line}' when none was expected.")),
            Some((at, expected)) if expected != line => {
                failures.push(format!("Expected output '{expected}' on line {at} and got '{line}'."))
            }
            _ => {}
        }
    }
    for (at, expected) in e.output.iter().skip(output_lines.len()) {
        failures.push(format!("Missing expected output '{expected}' on line {at}."));
    }
    failures
}

/// Runs `run` on every test in parallel and asserts that all pass.
fn run_suite(mode: &str, run: &(dyn Fn(&str, &Path) -> Output + Sync)) {
    let all = tests();
    assert!(all.len() > 200, "the suite is missing ({} tests found)", all.len());
    let failures = Mutex::new(Vec::new());
    let passed = Mutex::new(0usize);
    let next = Mutex::new(0usize);
    let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(16);
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                loop {
                    let i = {
                        let mut n = next.lock().unwrap();
                        let i = *n;
                        *n += 1;
                        i
                    };
                    let Some((rel, path)) = all.get(i) else { break };
                    let text = std::fs::read_to_string(path).unwrap();
                    let Some(e) = expectations(&text) else { continue };
                    let out = run(rel, path);
                    let f = validate(&e, &out);
                    if f.is_empty() {
                        *passed.lock().unwrap() += 1;
                    } else {
                        failures.lock().unwrap().push(format!("FAIL {rel}\n     {}", f.join("\n     ")));
                    }
                }
            });
        }
    });
    let failures = failures.into_inner().unwrap();
    let passed = passed.into_inner().unwrap();
    assert!(
        failures.is_empty(),
        "{mode}: {passed} passed, {} failed\n{}",
        failures.len(),
        failures.join("\n")
    );
    eprintln!("{mode}: all {passed} tests passed");
}

#[test]
fn suite_interpreted() {
    run_suite("interpreted", &|_, path| {
        Command::new(env!("CARGO_BIN_EXE_lox")).arg(path).output().unwrap()
    });
}

#[test]
fn suite_interpreted_with_gc_stress() {
    run_suite("interpreted, GC stress", &|_, path| {
        Command::new(env!("CARGO_BIN_EXE_lox"))
            .arg(path)
            .env("LOX_GC_STRESS", "1")
            .output()
            .unwrap()
    });
}

#[test]
fn suite_native() {
    let dir = std::env::temp_dir().join(format!("lox-suite-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    run_suite("native", &|rel, path| {
        let exe = dir.join(format!("{}{}", rel.replace('/', "_"), std::env::consts::EXE_SUFFIX));
        let build = Command::new(env!("CARGO_BIN_EXE_lox"))
            .arg("build")
            .arg(path)
            .arg("-o")
            .arg(&exe)
            .output()
            .unwrap();
        if !build.status.success() {
            // Compile errors are reported by the compiler, as they are when interpreting.
            return build;
        }
        Command::new(&exe).output().unwrap()
    });
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn expectation_parsing_matches_test_dart() {
    let e = expectations(
        "print 1; // expect: 1\nprint;// expect:2\nx; // Error at 'x': bad\n// [java line 9] Error: j\n// [c line 9] Error: c\nfoo(); // expect runtime error: Boom.\n",
    )
    .unwrap();
    assert_eq!(e.output, vec![(1, "1".to_string()), (2, "2".to_string())]);
    assert_eq!(e.errors, vec!["[3] Error at 'x': bad".to_string(), "[9] Error: j".to_string()]);
    assert_eq!(e.runtime_error, Some((6, "Boom.".to_string())));
    assert!(expectations("// nontest\n").is_none());
    assert_eq!(syntax_error("[line 3] Error at 'b': x"), Some("[3] Error at 'b': x".to_string()));
    assert_eq!(syntax_error("[java line 3] Error: x"), Some("[3] Error: x".to_string()));
    assert_eq!(stack_trace_line("[line 12] in script"), Some(12));
}
