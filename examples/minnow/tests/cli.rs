//! User-facing driver commands shared by every A0 language.

use std::path::Path;
use std::process::{Command, Output};

fn minnow(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_minnow"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn help_succeeds_and_lists_check() {
    for flag in ["--help", "-h", "help"] {
        let out = minnow(&[flag]);
        assert!(out.status.success(), "{flag}: {out:?}");
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("check <file>"), "{flag}: {text}");
        assert!(out.stderr.is_empty(), "{flag}: {out:?}");
    }
}

#[test]
fn check_compiles_without_running() {
    let file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/programs/closures.mn")
        .to_string_lossy()
        .into_owned();
    let out = minnow(&["check", &file]);
    assert!(out.status.success(), "{out:?}");
    assert!(out.stdout.is_empty(), "{out:?}");
    assert!(out.stderr.is_empty(), "{out:?}");
}

#[test]
fn build_accepts_options_before_the_file() {
    let file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/programs/closures.mn")
        .to_string_lossy()
        .into_owned();
    let dir = std::env::temp_dir().join(format!("a0-minnow-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let output = dir.join("closures").to_string_lossy().into_owned();
    let out = minnow(&["build", "-g", "-o", &output, &file]);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.status.success(), "{out:?}");
    assert!(out.stdout.is_empty(), "{out:?}");
    assert!(out.stderr.is_empty(), "{out:?}");
}
