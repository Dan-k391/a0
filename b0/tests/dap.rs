//! `b0 dap`, the debugger every language gets from the driver: breakpoints, the stack, locals
//! and their values, stepping, evaluation, and a stop where the program fails.

use a0_driver::dap::Client;
use a0_driver::json::Json;

const PROGRAM: &str = "// A program to debug.
struct Point { x: float, y: float }

fn norm(p: Point) -> float {
  let squares = p.x * p.x + p.y * p.y;
  return squares / 5.0;
}

fn main() {
  let name = \"origin\";
  let p = new Point { x: 3.0, y: 4.0 };
  let xs = [1, 2, 3]; let flags = [true, false, true];
  let n = norm(p);
  print(n);
  print(xs[n as int]);
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
    let dir = std::env::temp_dir().join(format!("b0-dap-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("debug.b0");
    std::fs::write(&file, PROGRAM).unwrap();
    let path = file.to_string_lossy().into_owned();

    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_b0"));
    let mut c = Client::start(exe, &["dap"]).unwrap();
    let launch = c.launch(&path, &[5]);
    assert_eq!(launch.get("success"), &Json::Bool(true), "{launch:?}");

    // At the breakpoint, in `norm` called from `main`.
    let stopped = c.event("stopped");
    assert_eq!(
        stopped.get("body").get("reason").as_str(),
        Some("breakpoint")
    );
    assert_eq!(c.stack(), [frame("norm", 5), frame("main", 13)]);
    let vars = c.locals(1);
    let p = value(&vars, "p");
    assert_eq!(p.1, "Point { x: 3, y: 4 }");
    let fields = c.variables(p.2);
    assert_eq!((fields[0].0.as_str(), fields[1].1.as_str()), ("x", "4"));
    let vars = c.locals(2);
    assert_eq!(value(&vars, "name").1, "\"origin\"");
    assert_eq!(value(&vars, "xs").1, "[1, 2, 3]");
    // Booleans are stored a byte each.
    assert_eq!(value(&vars, "flags").1, "[true, false, true]");

    // A step to the next line, then out to the caller's next line.
    c.resume("next");
    c.event("stopped");
    assert_eq!(c.stack()[0], frame("norm", 6));
    assert_eq!(value(&c.locals(1), "squares").1, "25");
    c.resume("stepOut");
    c.event("stopped");
    assert_eq!(c.stack(), [frame("main", 14)]);
    let n = c.request(
        "evaluate",
        Json::object([("expression", "n".into()), ("frameId", 1u32.into())]),
    );
    assert_eq!(n.get("body").get("result").as_str(), Some("5"));

    // The program fails: it stops there, then ends with the panic's status.
    c.resume("continue");
    let stopped = c.event("stopped");
    assert_eq!(
        stopped.get("body").get("reason").as_str(),
        Some("exception")
    );
    let text = stopped.get("body").get("text").as_str().unwrap();
    assert!(text.contains("index out of bounds"), "{text}");
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

const THREADED: &str = "import std.thread;

fn work(n: int) -> int {
  let total = n * 2;
  return total;
}

fn main() {
  let t = thread.spawn(fn() -> int {
    return work(21);
  });
  let r = thread.join(t);
  print(r);
}
";

#[test]
fn threads_are_debugged_with_the_program() {
    let dir = std::env::temp_dir().join(format!("b0-dap-threads-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("threads.b0");
    std::fs::write(&file, THREADED).unwrap();
    let path = file.to_string_lossy().into_owned();

    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_b0"));
    let mut c = Client::start(exe, &["dap"]).unwrap();
    let launch = c.launch(&path, &[4]);
    assert_eq!(launch.get("success"), &Json::Bool(true), "{launch:?}");

    // The breakpoint is in `work`, which the spawned thread runs.
    let started = c.event("thread");
    assert_eq!(started.get("body").get("reason").as_str(), Some("started"));
    let stopped = c.event("stopped");
    assert_eq!(stopped.get("body").get("threadId").as_u32(), Some(2));
    let threads = c.threads();
    assert!(threads.contains(&(1, "main".to_string())), "{threads:?}");
    assert!(
        threads.contains(&(2, "thread 2".to_string())),
        "{threads:?}"
    );
    let stack = c.stack_of(2);
    assert_eq!(stack[0], frame("work", 4));
    assert_eq!(stack[1], frame("main::{closure}", 10));
    assert_eq!(value(&c.locals(1), "n").1, "21");

    c.resume("continue");
    let ended = c.event("thread");
    assert_eq!(ended.get("body").get("reason").as_str(), Some("exited"));
    let exited = c.event("exited");
    assert_eq!(exited.get("body").get("exitCode").as_u32(), Some(0));
    c.event("terminated");
    assert_eq!(c.output("stdout"), "42\n");
    c.finish();
    std::fs::remove_dir_all(&dir).ok();
}
