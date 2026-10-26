//! Fuzzes the C API (D84): `fuzz_capi.py` makes random sequences of calls through ctypes, keeping
//! the header's contract and breaking everything else, in a process of its own, since a crash
//! across the C boundary aborts. Without Python the test is skipped, except on CI.
//!
//! `A0_FUZZ_ITERATIONS=n` runs `n` sequences (100 by default) and `A0_FUZZ_SEED=s` starts at seed
//! `s`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn shared_library() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let deps = exe.parent().unwrap();
    let name = if cfg!(windows) {
        "a0_capi.dll"
    } else if cfg!(target_os = "macos") {
        "liba0_capi.dylib"
    } else {
        "liba0_capi.so"
    };
    deps.join(name)
}

fn python() -> Option<&'static str> {
    ["python3", "python"].into_iter().find(|p| {
        Command::new(p)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    })
}

#[test]
fn random_calls_never_crash() {
    let Some(python) = python() else {
        assert!(std::env::var_os("CI").is_none(), "CI must have Python");
        eprintln!("skipped: no Python");
        return;
    };
    let env = |name: &str, default: u64| {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };
    let (count, first) = (env("A0_FUZZ_ITERATIONS", 100), env("A0_FUZZ_SEED", 1));
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let log = std::env::temp_dir().join(format!("a0-fuzz-capi-{}.log", std::process::id()));
    let out = Command::new(python)
        .arg(manifest.join("tests/fuzz_capi.py"))
        .arg(manifest.join("include/a0.h"))
        .arg(first.to_string())
        .arg(count.to_string())
        .arg(&log)
        .env("A0_LIBRARY", shared_library())
        .output()
        .unwrap();
    let calls = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        out.status.success() && calls.ends_with("done\n"),
        "{}\n{}\nthe sequence that failed, call by call ({}):\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr),
        log.display(),
        calls
            .lines()
            .rev()
            .take(40)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n")
    );
    std::fs::remove_file(&log).ok();
}
