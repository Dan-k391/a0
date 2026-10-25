//! `kaleidoscope dap`: the driver's debugger on a language that compiles to the core IR, whose
//! variables have no source-level types (they show as the IR's).

use a0_driver::dap::Client;
use a0_driver::json::Json;

const PROGRAM: &str = "extern printd(x);

def twice(x)
  var y = x * 2 in
    y + 1;

printd(twice(20));
";

#[test]
fn kaleidoscope_programs_can_be_debugged() {
    let dir = std::env::temp_dir().join(format!("kaleidoscope-dap-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("debug.k");
    std::fs::write(&file, PROGRAM).unwrap();
    let path = file.to_string_lossy().into_owned();

    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_kaleidoscope"));
    let mut c = Client::start(exe, &["dap"]).unwrap();
    let launch = c.launch(&path, &[5]);
    assert_eq!(launch.get("success"), &Json::Bool(true), "{launch:?}");
    let stopped = c.event("stopped");
    assert_eq!(
        stopped.get("body").get("reason").as_str(),
        Some("breakpoint")
    );
    let stack = c.stack();
    assert_eq!((stack[0].0.as_str(), stack[0].1), ("twice", 5), "{stack:?}");
    let vars = c.locals(1);
    assert!(
        vars.iter().any(|(n, v, _)| n == "x" && v == "20"),
        "{vars:?}"
    );
    assert!(
        vars.iter().any(|(n, v, _)| n == "y" && v == "40"),
        "{vars:?}"
    );
    c.resume("continue");
    let exited = c.event("exited");
    assert_eq!(exited.get("body").get("exitCode").as_u32(), Some(0));
    assert_eq!(c.output("stdout"), "41.000000\n");
    c.finish();
    std::fs::remove_dir_all(&dir).ok();
}
