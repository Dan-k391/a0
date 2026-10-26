//! Runs every program in `tests/programs` interpreted, in the JIT, as a native executable and with
//! the collector's stress mode (which a program without the collector must not mind), each with the
//! optimizer on and off, and checks compile errors.
//!
//! Programs state what they expect in comments: `// expect: <line>` for each line of stdout,
//! `// expect-stderr: <line>` and `// expect-exit: <code>`; `// stdin: <line>` for each line of
//! their standard input and `// args: <arguments>`. `A0_TEST_OUT` names a directory of their own
//! to write files in.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use a0_diag::SourceMap;

/// Runs a program with `stdin` as its input, killing it after a minute so that a collector bug
/// fails the test instead of hanging it.
fn run(cmd: &mut Command, stdin: &str) -> Output {
    use std::io::{Read, Write};
    use std::process::Stdio;
    use std::time::{Duration, Instant};
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let text = stdin.to_string();
    std::thread::spawn(move || input.write_all(text.as_bytes()).ok());
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

/// The programs in `tests/programs`.
fn programs() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/programs");
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "tin"))
        .collect();
    files.sort();
    assert!(files.len() >= 5, "the test programs are missing");
    files
}

/// What a program reads from standard input and its arguments.
fn inputs(file: &Path) -> (String, Vec<String>) {
    let text = std::fs::read_to_string(file).unwrap().replace("\r\n", "\n");
    let (mut stdin, mut args) = (String::new(), Vec::new());
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("// stdin: ") {
            stdin.push_str(rest);
            stdin.push('\n');
        } else if let Some(rest) = line.strip_prefix("// args: ") {
            args.extend(rest.split_whitespace().map(str::to_string));
        }
    }
    (stdin, args)
}

/// A directory of its own for each run of a program (modes run at the same time).
fn scratch(file: &Path, mode: &str) -> PathBuf {
    let name = format!(
        "tin-out-{}-{}-{}",
        file.file_stem().unwrap().to_string_lossy(),
        mode.replace([' ', ','], "-"),
        std::process::id()
    );
    let dir = std::env::temp_dir().join(name);
    std::fs::create_dir_all(&dir).unwrap();
    dir
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

// Every mode runs with the HIR optimizer on (the default) and off (`A0_OPT=0`): interpreted
// (`A0_INTERP=1`), compiled into memory (the JIT), the JIT under GC stress, and native.

/// The environment of each way `run` runs a program.
const INTERPRETED: &[(&str, &str)] = &[("A0_INTERP", "1")];
const JIT: &[(&str, &str)] = &[];
const GC_STRESS: &[(&str, &str)] = &[("A0_GC_STRESS", "1")];

fn in_process(mode: &str, opt: &str, env: &[(&str, &str)]) {
    check_all(mode, &|f| {
        // Run where the program is, so that stack traces name it as its expectations do.
        let (stdin, args) = inputs(f);
        let out = scratch(f, mode);
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_tin"));
        cmd.current_dir(f.parent().unwrap())
            .arg("run")
            .arg(f.file_name().unwrap())
            .args(&args)
            .env("A0_OPT", opt)
            .env("A0_TEST_OUT", &out)
            .envs(env.iter().copied());
        let output = run(&mut cmd, &stdin);
        std::fs::remove_dir_all(&out).ok();
        output
    });
}

fn native(mode: &str, opt: &str) {
    let dir = std::env::temp_dir().join(format!("tin-test-{opt}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    check_all(mode, &|f| {
        let exe = dir.join(format!(
            "{}{}",
            f.file_stem().unwrap().to_string_lossy(),
            std::env::consts::EXE_SUFFIX
        ));
        let build = run(
            Command::new(env!("CARGO_BIN_EXE_tin"))
                .current_dir(f.parent().unwrap())
                .arg("build")
                .arg(f.file_name().unwrap())
                .arg("-o")
                .arg(&exe)
                .env("A0_OPT", opt),
            "",
        );
        if !build.status.success() {
            return build;
        }
        let (stdin, args) = inputs(f);
        let out = scratch(f, mode);
        let output = run(
            Command::new(&exe)
                .current_dir(f.parent().unwrap())
                .args(&args)
                .env("A0_TEST_OUT", &out),
            &stdin,
        );
        std::fs::remove_dir_all(&out).ok();
        output
    });
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn programs_run_in_the_interpreter() {
    in_process("interpreted", "1", INTERPRETED);
}

#[test]
fn programs_run_in_the_jit() {
    in_process("JIT", "1", JIT);
}

#[test]
fn programs_survive_gc_stress() {
    in_process("GC stress", "1", GC_STRESS);
}

#[test]
fn programs_run_natively() {
    native("native", "1");
}

#[test]
fn unoptimized_programs_run_in_the_interpreter() {
    in_process("unoptimized, interpreted", "0", INTERPRETED);
}

#[test]
fn unoptimized_programs_run_in_the_jit() {
    in_process("unoptimized, JIT", "0", JIT);
}

#[test]
fn unoptimized_programs_survive_gc_stress() {
    in_process("unoptimized, GC stress", "0", GC_STRESS);
}

#[test]
fn unoptimized_programs_run_natively() {
    native("unoptimized, native", "0");
}

/// Compiles `src` and returns each error as `line:col: message`, followed by `: label` when the
/// error says what is wrong at its span.
fn errors(src: &str) -> Vec<String> {
    let mut sm = SourceMap::new();
    let id = sm.add("test.tin", src);
    match tin::compile(src, id) {
        Ok(_) => Vec::new(),
        Err(d) => d
            .as_slice()
            .iter()
            .map(|d| {
                let text = match d.label.as_str() {
                    "" => d.message.clone(),
                    label => format!("{}: {label}", d.message),
                };
                match d.span {
                    Some(s) => {
                        let (line, col) = sm.line_col(id, s.start);
                        format!("{line}:{col}: {text}")
                    }
                    None => text,
                }
            })
            .collect(),
    }
}

#[test]
fn compile_errors_are_reported_with_locations() {
    let cases: &[(&str, &str)] = &[
        (
            "fn main() -> i32 { var x: i32 = 1.5; return 0; }",
            "1:33: mismatched types: expected `i32`, found `f64`",
        ),
        ("fn main() -> i32 { return y; }", "1:27: unknown name `y`"),
        ("fn f(p: Pt) {}", "1:9: unknown type `Pt`"),
        (
            "struct P { x: i64 } fn main() -> i32 { var p = (P){ .x = 1 }; return p.z as i32; }",
            "1:72: no field `z` on type `P`",
        ),
        (
            "fn main() -> i32 { var p: *i64 = null; var q: *i32 = p; return 0; }",
            "1:54: mismatched types: expected `*i32`, found `*i64`",
        ),
        (
            "fn main() -> i32 { 1 = 2; return 0; }",
            "1:20: cannot assign to this expression",
        ),
        (
            "fn main() -> i32 { var a = 1; var a = 2; return 0; }",
            "1:35: `a` is already declared in this scope: declared again here",
        ),
        (
            "fn main() -> i32 { if (1) { } return 0; }",
            "1:24: mismatched types: expected `bool`, found `i64`",
        ),
        (
            "fn main() -> i32 { var c = 'ab'; return 0; }",
            "1:28: a character literal is one byte",
        ),
        (
            "struct S { name: []i64 } extern fn f(s: S); fn main() -> i32 { return 0; }",
            "extern fn f takes S; C functions take numbers",
        ),
        ("fn main() -> i32 { return 0 }", "1:29: expected ';'"),
        ("fn f() {}", "the program has no `main` function"),
    ];
    for (src, want) in cases {
        let got = errors(src);
        assert!(
            got.iter().any(|e| e.contains(want)),
            "{src}
expected: {want}
got: {got:#?}"
        );
    }
}
