//! `a0 new` makes a language project that builds and passes its own tests (programs interpreted
//! and native, its language server), depending on this checkout of A0.

use std::path::Path;
use std::process::Command;

#[test]
fn a_new_project_builds_and_passes_its_tests() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = std::path::absolute(&root).unwrap();
    let dir = std::env::temp_dir().join(format!("a0-new-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let new = Command::new(env!("CARGO_BIN_EXE_a0"))
        .args(["new", "demo-lang", "--ext", "dl", "--a0"])
        .arg(&root)
        .arg("--dir")
        .arg(&dir)
        .output()
        .unwrap();
    assert!(
        new.status.success(),
        "{}",
        String::from_utf8_lossy(&new.stderr)
    );
    for file in [
        "Cargo.toml",
        "demo-lang.a0g",
        "src/lib.rs",
        "src/main.rs",
        "tests/programs/basics.dl",
        "examples/hello.dl",
        ".vscode/settings.json",
        ".vscode/launch.json",
        "README.md",
    ] {
        assert!(dir.join(file).is_file(), "no {file}");
    }
    // A0's own lock file pins the versions of the dependencies they share, which are then already
    // downloaded and, in a target directory of their own kept between runs, built.
    std::fs::copy(root.join("Cargo.lock"), dir.join("Cargo.lock")).unwrap();
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let test = Command::new(cargo)
        .current_dir(&dir)
        .args(["test", "--offline"])
        .env("CARGO_TARGET_DIR", root.join("target/new-project"))
        .output()
        .unwrap();
    let report = format!(
        "{}{}",
        String::from_utf8_lossy(&test.stdout),
        String::from_utf8_lossy(&test.stderr)
    );
    assert!(test.status.success(), "{report}");
    assert!(report.contains("programs_run_natively ... ok"), "{report}");
    assert!(
        report.contains("the_language_server_knows_the_program ... ok"),
        "{report}"
    );
    assert!(
        report.contains("programs_can_be_debugged ... ok"),
        "{report}"
    );
    // A program fails to compile with the language's diagnostics.
    let hello = std::fs::read_to_string(dir.join("examples/hello.dl")).unwrap();
    std::fs::write(dir.join("broken.dl"), hello.replace("greet(\"", "gret(\"")).unwrap();
    let exe = root
        .join("target/new-project/debug")
        .join(format!("demo-lang{}", std::env::consts::EXE_SUFFIX));
    let run = Command::new(exe)
        .current_dir(&dir)
        .args(["run", "broken.dl"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(
        stderr.contains("error[E1001]: unknown function `gret`"),
        "{stderr}"
    );
    std::fs::remove_dir_all(&dir).ok();
}
