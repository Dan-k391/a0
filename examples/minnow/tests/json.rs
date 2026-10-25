//! `json`, the JSON tool in `examples/minnow/json` (v0.2's real program), run on fixtures in
//! `tests/json`: interpreted, as a native executable, and interpreted with the collector running on
//! every allocation.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

/// Runs `cmd` with `stdin` as its input.
fn run(cmd: &mut Command, stdin: &str) -> Output {
    use std::io::Write;
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let text = stdin.to_string();
    let writer = std::thread::spawn(move || input.write_all(text.as_bytes()).ok());
    let out = child.wait_with_output().unwrap();
    writer.join().unwrap();
    out
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(root().join("tests/json").join(name))
        .unwrap()
        .replace("\r\n", "\n")
}

/// (arguments, standard input, expected stdout, expected stderr, exit status); `SAMPLE` stands for
/// the sample file.
fn cases() -> Vec<(Vec<String>, String, String, String, i32)> {
    let sample = root().join("tests/json/sample.json").display().to_string();
    let args = |s: &str| -> Vec<String> {
        s.split_whitespace()
            .map(|a| {
                if a == "SAMPLE" {
                    sample.clone()
                } else {
                    a.to_string()
                }
            })
            .collect()
    };
    vec![
        (
            args("pretty SAMPLE"),
            String::new(),
            fixture("sample.pretty"),
            String::new(),
            0,
        ),
        (
            args("compact SAMPLE"),
            String::new(),
            fixture("sample.compact"),
            String::new(),
            0,
        ),
        (
            args("stats SAMPLE"),
            String::new(),
            fixture("sample.stats"),
            String::new(),
            0,
        ),
        (
            args("get .targets[1].os SAMPLE"),
            String::new(),
            "\"windows\"\n".into(),
            String::new(),
            0,
        ),
        (
            args("get .x[1].y"),
            r#"{"x": [1, {"y": "z"}]}"#.into(),
            "\"z\"\n".into(),
            String::new(),
            0,
        ),
        (
            args("compact -"),
            "[1, 2.50, -0, 1e2, \"\\u00e9\"]".into(),
            "[1,2.5,-0,100,\"é\"]\n".into(),
            String::new(),
            0,
        ),
        (
            args("pretty"),
            "{\"a\": [1, 2\n".into(),
            String::new(),
            "json: -: line 2, column 1: expected ']'\n".into(),
            1,
        ),
        (
            args("get .name.first SAMPLE"),
            String::new(),
            String::new(),
            "json: cannot take .first of string\n".into(),
            1,
        ),
        (
            args("pretty no/such/file.json"),
            String::new(),
            String::new(),
            "json: cannot read no/such/file.json: No such file or directory\n".into(),
            1,
        ),
    ]
}

fn check(mode: &str, run_one: &dyn Fn(&[String], &str) -> Output) {
    let mut failures = Vec::new();
    for (args, stdin, stdout, stderr, exit) in cases() {
        let out = run_one(&args, &stdin);
        let got = (
            String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n"),
            String::from_utf8_lossy(&out.stderr).replace("\r\n", "\n"),
            out.status.code().unwrap_or(-1),
        );
        if got != (stdout.clone(), stderr.clone(), exit) {
            failures.push(format!(
                "json {} ({mode}): got {got:?}, expected {:?}",
                args.join(" "),
                (stdout, stderr, exit)
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn interpreted(mode: &str, opt: &str, stress: bool) {
    let main = root().join("json/main.mn");
    check(mode, &|args, stdin| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_minnow"));
        cmd.arg("run").arg(&main).args(args).env("A0_OPT", opt);
        if stress {
            cmd.env("A0_GC_STRESS", "1");
        }
        run(&mut cmd, stdin)
    });
}

#[test]
fn the_json_tool_works_in_the_interpreter() {
    interpreted("interpreted", "1", false);
}

#[test]
fn the_json_tool_works_unoptimized() {
    interpreted("unoptimized, interpreted", "0", false);
}

#[test]
fn the_json_tool_survives_gc_stress() {
    interpreted("GC stress", "1", true);
}

#[test]
fn the_json_tool_works_natively() {
    let dir = std::env::temp_dir().join(format!("minnow-json-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join(format!("json{}", std::env::consts::EXE_SUFFIX));
    let build = Command::new(env!("CARGO_BIN_EXE_minnow"))
        .arg("build")
        .arg(root().join("json/main.mn"))
        .arg("-o")
        .arg(&exe)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    check("native", &|args, stdin| {
        run(Command::new(&exe).args(args), stdin)
    });
    std::fs::remove_dir_all(&dir).ok();
}
