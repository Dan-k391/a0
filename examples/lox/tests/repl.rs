//! `lox repl`: statements run at the top level in the order they are typed.

use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn lox_has_a_repl() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lox"))
        .arg("repl")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let input =
        "var x = 1;\nfun f() {\n  return x + 1;\n}\nprint f();\nprint 1 / nil;\nprint \"ok\";\n";
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "2\nok\n");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("Operands must be numbers."), "{err}");
}
