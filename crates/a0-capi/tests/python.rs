//! Runs the Python bindings' tests (`bindings/python/tests`) against the shared library cargo built
//! along with this test: the bindings, and Pebble, the language written with them, interpreted and
//! native; and Pebble's language server and debugger, which the driver gives it through the C API.
//! Without Python the tests are skipped, except on CI, which has it on every platform.

use std::path::{Path, PathBuf};
use std::process::Command;

use a0_driver::dap::Client;
use a0_driver::json::Json;
use a0_driver::lsp::session_with_args;

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

const PEBBLE_PROGRAM: &str = "fn twice(n: int) -> int {
  let doubled = n * 2;
  return doubled;
}

fn main() {
  print(twice(21));
}
";

/// The command that runs Pebble, with the library found as the bindings' tests find it.
fn pebble(python: &str) -> (PathBuf, Vec<String>) {
    // SAFETY: tests of this file only read the variable through the processes they start.
    unsafe { std::env::set_var("A0_LIBRARY", shared_library()) };
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = root.join("bindings/python/examples/pebble/pebble.py");
    (
        PathBuf::from(python),
        vec![script.to_string_lossy().into_owned()],
    )
}

fn result(messages: &[Json], id: u32) -> &Json {
    messages
        .iter()
        .find(|m| m.get("id").as_u32() == Some(id))
        .map(|m| m.get("result"))
        .unwrap_or_else(|| panic!("no response {id} in {messages:?}"))
}

fn diagnostics(messages: &[Json]) -> Vec<Json> {
    messages
        .iter()
        .filter(|m| m.get("method").as_str() == Some("textDocument/publishDiagnostics"))
        .flat_map(|m| m.get("params").get("diagnostics").as_array().to_vec())
        .collect()
}

#[test]
fn a_python_language_has_a_language_server() {
    let Some(python) = python() else {
        assert!(std::env::var_os("CI").is_none(), "CI must have Python");
        return;
    };
    let (exe, mut args) = pebble(python);
    args.push("lsp".to_string());
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let messages = session_with_args(
        &exe,
        &args,
        "file:///demo.pb",
        PEBBLE_PROGRAM,
        &[
            ("textDocument/documentSymbol", 0, 0),
            ("textDocument/definition", 6, 9),
            ("textDocument/hover", 2, 10),
        ],
    )
    .unwrap();
    assert!(diagnostics(&messages).is_empty(), "{messages:?}");
    let names: Vec<&str> = result(&messages, 1)
        .as_array()
        .iter()
        .map(|s| s.get("name").as_str().unwrap())
        .collect();
    assert_eq!(names, ["twice", "main"]);
    let definition = &result(&messages, 2).as_array()[0];
    assert_eq!(
        definition.get("range").get("start").get("line").as_u32(),
        Some(0)
    );
    let hover = result(&messages, 3).get("contents").get("value").as_str();
    assert!(hover.unwrap().contains("int"), "{hover:?}");

    // The compiler's errors, as the editor shows them.
    let broken = "fn main() {\n  let s = 1 + \"no\";\n}\n";
    let messages = session_with_args(&exe, &args, "file:///broken.pb", broken, &[]).unwrap();
    let d = diagnostics(&messages);
    assert_eq!(d.len(), 1, "{messages:?}");
    assert_eq!(d[0].get("code").as_str(), Some("E2001"));
    assert_eq!(d[0].get("range").get("start").get("line").as_u32(), Some(1));
}

#[test]
fn a_python_language_has_a_debugger() {
    let Some(python) = python() else {
        assert!(std::env::var_os("CI").is_none(), "CI must have Python");
        return;
    };
    let dir = std::env::temp_dir().join(format!("pebble-dap-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("debug.pb");
    std::fs::write(&file, PEBBLE_PROGRAM).unwrap();
    let path = file.to_string_lossy().into_owned();
    let (exe, mut args) = pebble(python);
    args.push("dap".to_string());
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut c = Client::start(&exe, &args).unwrap();
    let launch = c.launch(&path, &[3]);
    assert_eq!(launch.get("success"), &Json::Bool(true), "{launch:?}");
    c.event("stopped");
    let stack = c.stack();
    assert_eq!(stack[0], ("twice".to_string(), 3), "{stack:?}");
    assert_eq!(stack[1], ("main".to_string(), 7), "{stack:?}");
    let locals = c.locals(1);
    for (name, value) in [("n", "21"), ("doubled", "42")] {
        assert!(
            locals.iter().any(|(n, v, _)| n == name && v == value),
            "{name} = {value} not in {locals:?}"
        );
    }
    c.resume("continue");
    let exited = c.event("exited");
    assert_eq!(exited.get("body").get("exitCode").as_u32(), Some(0));
    assert_eq!(c.output("stdout"), "42\n");
    c.finish();
    std::fs::remove_dir_all(&dir).ok();
}
