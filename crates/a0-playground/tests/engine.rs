//! The playground's engine, built for the host: its modes, errors and exit statuses. CI also builds it
//! for WebAssembly and runs it with Node (`playground/test.mjs`).

use std::process::{Command, Output};

const CALC: &str = include_str!("../../../examples/calc/calc.a0g");

fn engine(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_a0-playground"))
        .args(args)
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

#[test]
fn programs_run_and_show_their_tree_and_ir() {
    let program = "fn twice(x) {\n  return x * 2\n}\nprint twice(21), \"!\"\n";
    let out = engine(&["run", CALC, program]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout), "42 !\n");
    let tree = text(&engine(&["tree", CALC, "print 1"]).stdout);
    assert!(tree.starts_with("(program (print_stmt"), "{tree}");
    let hir = text(&engine(&["hir", CALC, program]).stdout);
    assert!(hir.contains("fn twice(x: float) -> float"), "{hir}");
    let ir = text(&engine(&["ir", CALC, program]).stdout);
    assert!(ir.contains("func @main"), "{ir}");
}

#[test]
fn errors_point_into_the_grammar_or_the_program() {
    let out = engine(&["run", CALC, "print nope(1)"]);
    assert_eq!(out.status.code(), Some(1));
    let stderr = text(&out.stderr);
    assert!(stderr.contains("--> program:1:7"), "{stderr}");
    let out = engine(&["run", "grammar g; program = missing;", ""]);
    assert_eq!(out.status.code(), Some(1));
    let stderr = text(&out.stderr);
    assert!(stderr.contains("--> grammar:1:22"), "{stderr}");
    let out = engine(&["run", CALC]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn panics_exit_with_101() {
    let out = engine(&[
        "run",
        CALC,
        "fn f(n) {\n  return 1 + f(n + 1)\n}\nprint f(1)\n",
    ]);
    assert_eq!(out.status.code(), Some(101));
    assert!(text(&out.stderr).starts_with("panic: stack overflow"));
}
