//! The top-level command's conventional introspection flags.

use std::process::{Command, Output};

fn a0(arg: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_a0"))
        .arg(arg)
        .output()
        .unwrap()
}

#[test]
fn version_is_reported() {
    for flag in ["--version", "-V"] {
        let out = a0(flag);
        assert!(out.status.success(), "{flag}: {out:?}");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            format!("a0 {}\n", env!("CARGO_PKG_VERSION"))
        );
        assert!(out.stderr.is_empty());
    }
}

#[test]
fn help_describes_cross_builds_and_debuggers() {
    let out = a0("--help");
    assert!(out.status.success(), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("another target needs its pack"), "{text}");
    assert!(
        text.contains("-g adds information for the target's debugger"),
        "{text}"
    );
    assert!(!text.contains("host target only"), "{text}");
    assert!(!text.contains("gdb and lldb (Linux, macOS)"), "{text}");
    assert!(out.stderr.is_empty());
}
