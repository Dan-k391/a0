//! Runs every program in `tests/programs` in A0's interpreter and as a native executable, and
//! compares what it prints with its `// expect: <line>` comments (`// expect-exit: <code>` for the
//! exit status, `// expect-stderr: <line>` for errors).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn programs() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/programs");
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "{{ext}}"))
        .collect();
    files.sort();
    files
}

fn check(file: &Path, out: &Output, mode: &str) -> Option<String> {
    let text = std::fs::read_to_string(file).unwrap().replace("\r\n", "\n");
    let (mut stdout, mut stderr, mut exit) = (String::new(), String::new(), 0);
    for line in text.lines() {
        if let Some(rest) = line.trim_start().strip_prefix("// expect: ") {
            stdout.push_str(rest);
            stdout.push('\n');
        } else if let Some(rest) = line.trim_start().strip_prefix("// expect-stderr: ") {
            stderr.push_str(rest);
            stderr.push('\n');
        } else if let Some(rest) = line.trim_start().strip_prefix("// expect-exit: ") {
            exit = rest.trim().parse().unwrap();
        }
    }
    let got = (
        String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n"),
        String::from_utf8_lossy(&out.stderr).replace("\r\n", "\n"),
        out.status.code().unwrap_or(-1),
    );
    (got != (stdout.clone(), stderr.clone(), exit)).then(|| {
        format!(
            "{} ({mode}): got {got:?}, expected {:?}",
            file.display(),
            (stdout, stderr, exit)
        )
    })
}

#[test]
fn programs_run_in_the_interpreter() {
    let failures: Vec<String> = programs()
        .iter()
        .filter_map(|f| {
            let out = Command::new(env!("CARGO_BIN_EXE_{{name}}"))
                .current_dir(f.parent().unwrap())
                .arg("run")
                .arg(f.file_name().unwrap())
                .output()
                .unwrap();
            check(f, &out, "interpreted")
        })
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn programs_run_natively() {
    let dir = std::env::temp_dir().join(format!("{{name}}-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let failures: Vec<String> = programs()
        .iter()
        .filter_map(|f| {
            let exe = dir.join(format!(
                "{}{}",
                f.file_stem().unwrap().to_string_lossy(),
                std::env::consts::EXE_SUFFIX
            ));
            let build = Command::new(env!("CARGO_BIN_EXE_{{name}}"))
                .current_dir(f.parent().unwrap())
                .arg("build")
                .arg(f.file_name().unwrap())
                .arg("-o")
                .arg(&exe)
                .output()
                .unwrap();
            if !build.status.success() {
                return check(f, &build, "native build");
            }
            let out = Command::new(&exe).output().unwrap();
            check(f, &out, "native")
        })
        .collect();
    std::fs::remove_dir_all(&dir).ok();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
