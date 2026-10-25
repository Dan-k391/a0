//! Runs the Python bindings' tests (`bindings/python/tests`) against the shared library cargo built
//! along with this test: the bindings, and Pebble, the language written with them, interpreted and
//! native. Without Python the test is skipped, except on CI, which has it on every platform.

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
    let path = deps.join(name);
    assert!(path.is_file(), "no {}", path.display());
    path
}

/// The first of `python3` and `python` that runs (Windows may have a `python3` that only opens the
/// store).
fn python() -> Option<&'static str> {
    ["python3", "python"].into_iter().find(|p| {
        Command::new(p)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    })
}

#[test]
fn the_python_bindings_pass_their_tests() {
    let Some(python) = python() else {
        assert!(
            std::env::var_os("CI").is_none(),
            "CI must have Python to test the bindings"
        );
        eprintln!("skipped: no Python");
        return;
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = Command::new(python)
        .current_dir(&root)
        .args(["-m", "unittest", "discover", "-v", "-s"])
        .arg(root.join("bindings/python/tests"))
        .env("A0_LIBRARY", shared_library())
        .output()
        .unwrap();
    let report = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{report}");
    assert!(report.contains("test_programs_run_natively"), "{report}");
}
