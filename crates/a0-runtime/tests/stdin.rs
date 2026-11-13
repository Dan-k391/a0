//! Standard input reads bytes exactly, on every platform: no CRLF translation, and 0x1A (which
//! ends text-mode input on Windows) is an ordinary byte.

use std::io::Write;
use std::process::{Command, Stdio};

unsafe extern "C" {
    fn a0_std_read_all() -> *const i64;
    fn a0_std_read_line() -> *const i64;
}

/// The lengths the child read: all of its input at once, or line by line.
fn read(how: &str, input: &[u8]) -> String {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "stdin_is_read_byte_for_byte", "--nocapture"])
        .env("A0_TEST_STDIN", how)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).unwrap();
    let line = text.lines().find_map(|l| l.strip_prefix("lengths:"));
    line.expect("the child printed no lengths")
        .trim()
        .to_string()
}

#[test]
fn stdin_is_read_byte_for_byte() {
    if let Ok(how) = std::env::var("A0_TEST_STDIN") {
        a0_runtime::init(&[]);
        // A string starts with its length in bytes.
        let mut lengths = Vec::new();
        if how == "all" {
            lengths.push(unsafe { *a0_std_read_all() });
        } else {
            loop {
                let line = unsafe { a0_std_read_line() };
                if line.is_null() {
                    break;
                }
                lengths.push(unsafe { *line });
            }
        }
        let lengths: Vec<String> = lengths.iter().map(i64::to_string).collect();
        println!("lengths: {}", lengths.join(" "));
        return;
    }
    assert_eq!(read("all", b"a\r\nb\r\n"), "6");
    assert_eq!(read("all", b"ab\x1acd\n"), "6");
    assert_eq!(read("lines", b"ab\x1acd\nef\n"), "5 2");
    assert_eq!(read("lines", b"a\r\nb\r\n"), "1 1");
}
