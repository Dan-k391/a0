//! Calc and Lispy (`examples/calc`, `examples/lispy`), languages defined by their grammars alone,
//! through `a0 lang`: their programs interpreted, under GC stress, without the optimizer and native;
//! Calc's errors and language server.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use a0_driver::json::Json;
use a0_driver::lsp::session_with_args;

fn root() -> PathBuf {
    std::path::absolute(Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")).unwrap()
}

fn lang(grammar: &str, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_a0"));
    cmd.current_dir(root()).args(["lang", grammar]).args(args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().unwrap()
}

fn calc(args: &[&str], env: &[(&str, &str)]) -> Output {
    lang("examples/calc/calc.a0g", args, env)
}

/// The lines after `<comment> expect-stdout:`, each a comment.
fn expected(text: &str, comment: &str) -> String {
    let mut out = String::new();
    let mut lines = text
        .lines()
        .skip_while(|l| *l != format!("{comment} expect-stdout:"));
    lines.next();
    for line in lines {
        let line = line.strip_prefix(comment).unwrap_or(line);
        out.push_str(line.strip_prefix(' ').unwrap_or(line));
        out.push('\n');
    }
    out
}

/// Each example's programs: the grammar, the program, and what it should print.
fn programs() -> Vec<(&'static str, PathBuf, String)> {
    let mut all = Vec::new();
    for (grammar, dir, extension, comment) in [
        (
            "examples/calc/calc.a0g",
            "examples/calc/programs",
            "calc",
            "#",
        ),
        (
            "examples/lispy/lispy.a0g",
            "examples/lispy/programs",
            "lisp",
            ";",
        ),
    ] {
        let mut v: Vec<PathBuf> = std::fs::read_dir(root().join(dir))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == extension))
            .collect();
        v.sort();
        assert!(!v.is_empty(), "no programs in {dir}");
        for p in v {
            let want = expected(&std::fs::read_to_string(&p).unwrap(), comment);
            all.push((grammar, p, want));
        }
    }
    all
}

fn check(name: &str, mode: &str, out: &Output, want: &str) {
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{name} ({mode}): {stderr}");
    assert_eq!(stdout, want, "{name} ({mode})");
}

#[test]
fn programs_run_interpreted_and_in_the_jit() {
    for (grammar, p, want) in programs() {
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        let file = p.to_string_lossy();
        for (mode, env) in [
            ("interpreted", &[("A0_INTERP", "1")][..]),
            ("JIT", &[][..]),
            ("GC stress", &[("A0_GC_STRESS", "1")][..]),
            ("unoptimized", &[("A0_OPT", "0")][..]),
            (
                "unoptimized, interpreted",
                &[("A0_OPT", "0"), ("A0_INTERP", "1")][..],
            ),
        ] {
            check(&name, mode, &lang(grammar, &["run", &file], env), &want);
        }
    }
}

#[test]
fn programs_run_natively() {
    let dir = std::env::temp_dir().join(format!("a0-calc-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (grammar, p, want) in programs() {
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        let exe = dir.join(p.file_stem().unwrap());
        let build = lang(
            grammar,
            &["build", &p.to_string_lossy(), "-o", &exe.to_string_lossy()],
            &[],
        );
        assert!(
            build.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&build.stderr)
        );
        let exe = exe.with_extension(std::env::consts::EXE_EXTENSION);
        for (mode, stress) in [("native", "0"), ("native, GC stress", "1")] {
            let out = Command::new(&exe)
                .env("A0_GC_STRESS", stress)
                .output()
                .unwrap();
            check(&name, mode, &out, &want);
        }
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn errors_are_reported_in_the_program_s_terms() {
    let dir = std::env::temp_dir().join(format!("a0-calc-errors-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("bad.calc");
    std::fs::write(
        &file,
        "let x = 1\nprint x + \"a\"\nprint nope(2)\nlet y = \n",
    )
    .unwrap();
    let out = calc(&["run", &file.to_string_lossy()], &[]);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("error[E0001]: expected a value"),
        "{stderr}"
    );
    std::fs::write(&file, "let x = 1\nprint x + \"a\"\nprint nope(2)\n").unwrap();
    let stderr =
        String::from_utf8_lossy(&calc(&["run", &file.to_string_lossy()], &[]).stderr).into_owned();
    assert!(
        stderr.contains("error[E2001]: mismatched types"),
        "{stderr}"
    );
    assert!(
        stderr.contains("error[E1001]: unknown function `nope`"),
        "{stderr}"
    );
    assert!(stderr.contains("bad.calc:3:7"), "{stderr}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn bad_grammars_are_reported() {
    let dir = std::env::temp_dir().join(format!("a0-calc-grammar-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let grammar = dir.join("g.a0g");
    let program = dir.join("p.g");
    let run = |text: &str, source: &str| {
        std::fs::write(&grammar, text).unwrap();
        std::fs::write(&program, source).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_a0"))
            .args(["lang", &grammar.to_string_lossy(), "run"])
            .arg(&program)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stderr).into_owned()
    };
    let head = "grammar g;\nskip /[ \\n]+/;\ntoken NUMBER /[0-9]+/;\n";
    let stderr = run(&format!("{head}program @hir(sum) = NUMBER;\n"), "1");
    assert!(
        stderr.contains("the rule `program` has the @hir form `sum`"),
        "{stderr}"
    );
    let stderr = run(
        &format!("{head}program = x;\nx @inline @hir(int) = NUMBER;\n"),
        "1",
    );
    assert!(
        stderr.contains("'x' is @inline, so it has no nodes to give an @hir form"),
        "{stderr}"
    );
    // A node the grammar gives no meaning: neither a form nor a single child that has one.
    let stderr = run(
        &format!("{head}program = stuff;\nstuff = \"<\" NUMBER \">\";\n"),
        "<1>",
    );
    assert!(
        stderr.contains("the grammar does not say what a `stuff` means"),
        "{stderr}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

const PROGRAM: &str = "fn square(x) {\n  return x * x\n}\n\nlet n = square(3)\nprint n\n";

fn session(text: &str, requests: &[(&str, u32, u32)]) -> Vec<Json> {
    let grammar = root().join("examples/calc/calc.a0g");
    session_with_args(
        Path::new(env!("CARGO_BIN_EXE_a0")),
        &["lang", &grammar.to_string_lossy(), "lsp"],
        "file:///demo.calc",
        text,
        requests,
    )
    .unwrap()
}

fn diagnostics(messages: &[Json]) -> Vec<&Json> {
    messages
        .iter()
        .filter(|m| m.get("method").as_str() == Some("textDocument/publishDiagnostics"))
        .flat_map(|m| m.get("params").get("diagnostics").as_array())
        .collect()
}

#[test]
fn the_language_server_knows_calc_programs() {
    let messages = session(
        PROGRAM,
        &[
            ("textDocument/documentSymbol", 0, 0),
            ("textDocument/definition", 4, 9),
            ("textDocument/hover", 4, 4),
        ],
    );
    let result = |id: u32| {
        messages
            .iter()
            .find(|m| m.get("id").as_u32() == Some(id))
            .map(|m| m.get("result").clone())
            .unwrap()
    };
    let outline = result(1);
    let names: Vec<&str> = outline
        .as_array()
        .iter()
        .map(|s| s.get("name").as_str().unwrap())
        .collect();
    assert_eq!(names, ["square", "n"]);
    // `square` in `let n = square(3)` is defined on line 0.
    let definition = result(2);
    let line = definition.as_array()[0]
        .get("range")
        .get("start")
        .get("line")
        .as_u32();
    assert_eq!(line, Some(0), "{definition:?}");
    // Numbers are floats, and so is `n`.
    let hover = format!("{:?}", result(3));
    assert!(hover.contains("float"), "{hover}");
    assert!(diagnostics(&messages).is_empty());

    let broken = PROGRAM.replace("print n", "print cube(n)");
    let messages = session(&broken, &[]);
    let diagnostics = diagnostics(&messages);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    let message = diagnostics[0].get("message").as_str().unwrap();
    assert!(message.contains("cube"), "{message}");
}

#[test]
fn calc_programs_can_be_debugged() {
    let dir = std::env::temp_dir().join(format!("a0-calc-dap-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("debug.calc");
    std::fs::write(
        &file,
        "fn area(w, h) {\n  let a = w * h\n  return a\n}\nlet total = area(3, 4)\nprint total\n",
    )
    .unwrap();
    let grammar = root().join("examples/calc/calc.a0g");
    let mut c = a0_driver::dap::Client::start(
        Path::new(env!("CARGO_BIN_EXE_a0")),
        &["lang", &grammar.to_string_lossy(), "dap"],
    )
    .unwrap();
    let launch = c.launch(&file.to_string_lossy(), &[3]);
    assert_eq!(launch.get("success"), &Json::Bool(true), "{launch:?}");
    c.event("stopped");
    assert_eq!(
        c.stack(),
        [("area".to_string(), 3), ("main".to_string(), 5)]
    );
    let locals = c.locals(1);
    for (name, value) in [("w", "3"), ("h", "4"), ("a", "12")] {
        assert!(
            locals.iter().any(|(n, v, _)| n == name && v == value),
            "{name} = {value} not in {locals:?}"
        );
    }
    c.resume("continue");
    c.event("exited");
    assert_eq!(c.output("stdout"), "12\n");
    c.finish();
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn calc_has_a_repl() {
    use std::io::Write;
    let mut child = Command::new(env!("CARGO_BIN_EXE_a0"))
        .current_dir(root())
        .args(["lang", "examples/calc/calc.a0g", "repl"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let input = "let total = 1\nrepeat 3 {\n  total = total * 2\n}\nprint total\nprint total + 1\n";
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "8\n9\n");
}
