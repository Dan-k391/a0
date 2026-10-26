//! `tin dap`, the debugger every language gets from the driver: breakpoints, the stack, locals
//! (an inline struct among them), stepping, and a stop where the program fails.

use a0_driver::dap::Client;
use a0_driver::json::Json;

const PROGRAM: &str = "// A program to debug.
struct Vec2 { x: f64, y: f64 }

fn norm(v: Vec2) -> f64 {
  var squares = v.x * v.x + v.y * v.y;
  return squares / 5.0;
}

fn main() -> i32 {
  var p = (Vec2){ .x = 3.0, .y = 4.0 };
  var count: i32 = 3;
  var xs: []i64 = [1, 2, 3];
  var n = norm(p);
  println(n);
  println(xs[n as i64]);
  return count;
}
";

fn value<'a>(vars: &'a [(String, String, u32)], name: &str) -> &'a (String, String, u32) {
    vars.iter()
        .find(|v| v.0 == name)
        .unwrap_or_else(|| panic!("no `{name}` in {vars:?}"))
}

fn frame(name: &str, line: u32) -> (String, u32) {
    (name.to_string(), line)
}

#[test]
fn a_program_can_be_debugged() {
    let dir = std::env::temp_dir().join(format!("tin-dap-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("debug.tin");
    std::fs::write(&file, PROGRAM).unwrap();
    let path = file.to_string_lossy().into_owned();

    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_tin"));
    let mut c = Client::start(exe, &["dap"]).unwrap();
    let launch = c.launch(&path, &[5]);
    assert_eq!(launch.get("success"), &Json::Bool(true), "{launch:?}");

    let stopped = c.event("stopped");
    assert_eq!(
        stopped.get("body").get("reason").as_str(),
        Some("breakpoint")
    );
    assert_eq!(c.stack(), [frame("norm", 5), frame("main", 13)]);
    let vars = c.locals(1);
    assert_eq!(value(&vars, "v").1, "Vec2 { x: 3, y: 4 }");
    let vars = c.locals(2);
    assert_eq!(value(&vars, "count").1, "3");
    assert_eq!(value(&vars, "xs").1, "[1, 2, 3]");

    c.resume("next");
    c.event("stopped");
    assert_eq!(c.stack()[0], frame("norm", 6));
    assert_eq!(value(&c.locals(1), "squares").1, "25");
    c.resume("stepOut");
    c.event("stopped");
    assert_eq!(c.stack(), [frame("main", 14)]);

    c.resume("continue");
    let stopped = c.event("stopped");
    assert_eq!(
        stopped.get("body").get("reason").as_str(),
        Some("exception")
    );
    assert_eq!(c.stack(), [frame("main", 15)]);
    c.resume("continue");
    let exited = c.event("exited");
    assert_eq!(exited.get("body").get("exitCode").as_u32(), Some(101));
    c.event("terminated");
    assert_eq!(c.output("stdout"), "5\n");
    assert!(c.output("stderr").contains("panic: index out of bounds"));
    c.finish();
    std::fs::remove_dir_all(&dir).ok();
}
