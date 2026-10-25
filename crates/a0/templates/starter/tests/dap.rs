//! `{{name}} dap`, the debugger: a breakpoint, the stack and the locals of a small program.

use a0_driver::dap::Client;

const PROGRAM: &str = "fn twice(x: int) -> int {\n  let y = x * 2;\n  return y;\n}\n\nfn main() {\n  print(twice(21));\n}\n";

#[test]
fn programs_can_be_debugged() {
    let dir = std::env::temp_dir().join(format!("{{name}}-dap-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("demo.{{ext}}");
    std::fs::write(&file, PROGRAM).unwrap();
    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_{{name}}"));
    let mut c = Client::start(exe, &["dap"]).unwrap();
    c.launch(&file.to_string_lossy(), &[3]);
    c.event("stopped");
    assert_eq!(c.stack()[0], ("twice".to_string(), 3));
    let locals = c.locals(1);
    assert!(
        locals.iter().any(|(n, v, _)| n == "y" && v == "42"),
        "{locals:?}"
    );
    c.resume("continue");
    c.event("exited");
    assert_eq!(c.output("stdout"), "42\n");
    c.finish();
    std::fs::remove_dir_all(&dir).ok();
}
