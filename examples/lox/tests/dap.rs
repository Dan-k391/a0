//! `lox dap`: the driver's debugger on a dynamically typed language, whose locals are `any`.

use a0_driver::dap::Client;
use a0_driver::json::Json;

const PROGRAM: &str = "fun add(a, b) {
  var sum = a + b;
  var label = \"sum\";
  return sum;
}
print add(1, 2);
print \"done\";
";

#[test]
fn lox_programs_can_be_debugged() {
    let dir = std::env::temp_dir().join(format!("lox-dap-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("debug.lox");
    std::fs::write(&file, PROGRAM).unwrap();
    let path = file.to_string_lossy().into_owned();

    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_lox"));
    let mut c = Client::start(exe, &["dap"]).unwrap();
    let launch = c.launch(&path, &[3]);
    assert_eq!(launch.get("success"), &Json::Bool(true), "{launch:?}");
    let stopped = c.event("stopped");
    assert_eq!(
        stopped.get("body").get("reason").as_str(),
        Some("breakpoint")
    );
    let stack = c.stack();
    assert_eq!((stack[0].0.as_str(), stack[0].1), ("add", 3), "{stack:?}");
    assert_eq!(stack.last().unwrap().1, 6, "{stack:?}");
    let vars = c.locals(1);
    let shown: Vec<(&str, &str)> = vars
        .iter()
        .map(|(n, v, _)| (n.as_str(), v.as_str()))
        .collect();
    for wanted in [("a", "1"), ("b", "2"), ("sum", "3")] {
        assert!(shown.contains(&wanted), "{wanted:?} not in {shown:?}");
    }
    c.resume("next");
    c.event("stopped");
    assert_eq!(c.stack()[0].1, 4);
    let vars = c.locals(1);
    assert!(
        vars.iter().any(|(n, v, _)| n == "label" && v == "\"sum\""),
        "{vars:?}"
    );
    c.resume("continue");
    let exited = c.event("exited");
    assert_eq!(exited.get("body").get("exitCode").as_u32(), Some(0));
    assert_eq!(c.output("stdout"), "3\ndone\n");
    c.finish();
    std::fs::remove_dir_all(&dir).ok();
}
