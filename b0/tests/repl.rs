//! `b0 repl`: declarations and statements run together, unfinished lines wait for more, and an
//! input that fails is dropped.

use std::io::Write;
use std::process::{Command, Stdio};

fn repl(input: &str) -> (String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_b0"))
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
        "let x = 2;
fn double(n: int) -> int {
  return n * 2;
}
print(double(x));
print(nope);
let xs = [1, 2];
print(xs[5]);
if x > 1 {
  print(\"big\");
}
:show
",
    );
    assert_eq!(
        out,
        "4\nbig\n   1 | fn double(n: int) -> int {\n   2 |   return n * 2;\n   3 | }\n   4 | fn main() {\n   5 | let x = 2;\n   6 | print(double(x));\n   7 | let xs = [1, 2];\n   8 | if x > 1 {\n   9 |   print(\"big\");\n  10 | }\n  11 | }\n"
    );
    assert!(err.contains("error[E1001]: unknown name `nope`"), "{err}");
    assert!(err.contains("panic: index out of bounds"), "{err}");
}
