//! `examples/wordfreq`, a B0 package with modules, threads, maps, closures and the standard
//! library, run on a sample: interpreted, in the JIT, under GC stress and as an executable.

use std::path::{Path, PathBuf};
use std::process::Command;

fn package() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/wordfreq")
}

const EXPECTED: &str = "the             10
was             10
age             2
";

const SUMMARY: &str = "4 lines, 60 words, 20 different\n";

fn check(out: std::process::Output, how: &str) {
    let stdout = String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n");
    assert!(
        out.status.success() && stdout == format!("{EXPECTED}{SUMMARY}"),
        "{how}: {}\n{stdout}{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn wordfreq_counts_in_every_mode() {
    for (how, env) in [
        ("interpreted", &[("A0_INTERP", "1")][..]),
        ("jit", &[][..]),
        ("gc stress", &[("A0_GC_STRESS", "1")][..]),
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_b0"))
            .current_dir(package())
            .args(["run", "--", "-n", "3", "tests/sample.txt"])
            .envs(env.iter().copied())
            .output()
            .unwrap();
        check(out, how);
    }
    let exe = std::env::temp_dir().join(format!(
        "wordfreq-{}{}",
        std::process::id(),
        std::env::consts::EXE_SUFFIX
    ));
    let build = Command::new(env!("CARGO_BIN_EXE_b0"))
        .current_dir(package())
        .args(["build", "-o"])
        .arg(&exe)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let out = Command::new(&exe)
        .current_dir(package())
        .args(["-n", "3", "tests/sample.txt"])
        .output()
        .unwrap();
    std::fs::remove_file(&exe).ok();
    check(out, "native");
}

#[test]
fn wordfreq_reports_a_missing_file() {
    let out = Command::new(env!("CARGO_BIN_EXE_b0"))
        .current_dir(package())
        .args(["run", "--", "missing.txt"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "{stdout}");
    assert!(
        stdout.starts_with("wordfreq: cannot read missing.txt"),
        "{stdout}"
    );
}

#[test]
fn the_readme_example_runs() {
    let readme = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md"))
        .unwrap()
        .replace("\r\n", "\n");
    let start = readme.find("```\nimport std.thread;").expect("the example") + 4;
    let end = start + readme[start..].find("```").unwrap();
    let file = std::env::temp_dir().join(format!("b0-readme-{}.b0", std::process::id()));
    std::fs::write(&file, &readme[start..end]).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_b0"))
        .arg("run")
        .arg(&file)
        .output()
        .unwrap();
    std::fs::remove_file(&file).ok();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "9.14159\n42\n");
}
