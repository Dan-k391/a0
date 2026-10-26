//! `tin repl`: declarations and statements of `main` run together, unfinished lines wait for
//! more, and an input that fails is dropped.

use std::io::Write;
use std::process::{Command, Stdio};

fn repl(input: &str) -> (String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_tin"))
        .arg("repl")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn inputs_run_together_and_failures_are_dropped() {
    let (out, err) = repl(
        "struct Pair { a: i32, b: i32 }
fn swap(p: Pair) -> Pair {
  return (Pair){ .a = p.b, .b = p.a };
}
var p = swap((Pair){ .a = 1, .b = 2 });
println(p.a, \" \", p.b);
println(nope);
var xs: []i64 = [1, 2];
println(xs[5]);
",
    );
    assert_eq!(out, "2 1\n");
    assert!(err.contains("error[E1001]: unknown name `nope`"), "{err}");
    assert!(err.contains("panic: index out of bounds"), "{err}");
}
