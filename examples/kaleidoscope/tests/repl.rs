//! `kaleidoscope repl`: definitions and top-level expressions, as in LLVM's tutorial.

use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn kaleidoscope_has_a_repl() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_kaleidoscope"))
        .arg("repl")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let input = "extern printd(x);\ndef twice(x)\n  x * 2;\nprintd(twice(21));\nprintd(4);\n";
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "42.000000\n4.000000\n"
    );
}
