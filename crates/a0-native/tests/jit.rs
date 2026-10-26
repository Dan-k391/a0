//! Running modules from memory. The differential tests run every program through the JIT (`a0
//! run`); these check the library's own contract.

use a0_diag::SourceId;
use a0_native::{JitOptions, run_main};
use a0_runtime::HostSymbols;

fn module(text: &str) -> a0_ir::Module {
    a0_ir::parse_and_verify(text, SourceId(0)).unwrap()
}

#[test]
fn main_runs_with_its_arguments_and_calls_c() {
    // Returns the number of arguments times the length of "four", found by C's strlen.
    let m = module(
        r#"module "args"
extern func @a0_rt_argc() -> i32
extern func @strlen(ptr) -> i64
data @word = "four\00"

func @main() -> i32 {
  %argc: i32 = call @a0_rt_argc()
  %n: i64 = sext %argc
  %w: ptr = addr @word
  %len: i64 = call @strlen(%w)
  %r: i64 = imul %n, %len
  %c: i32 = trunc %r
  return %c
}
"#,
    );
    let args = vec!["prog".to_string(), "a".to_string(), "b".to_string()];
    let code = run_main(&m, HostSymbols::new(), args, &JitOptions::default()).unwrap();
    assert_eq!(code, 12);
}

#[test]
fn unknown_c_functions_are_reported_before_running() {
    let m = module(
        r#"module "missing"
extern func @no_such_function_anywhere() -> i32

func @main() -> i32 {
  %r: i32 = call @no_such_function_anywhere()
  return %r
}
"#,
    );
    let err = run_main(&m, HostSymbols::empty(), vec![], &JitOptions::default()).unwrap_err();
    assert!(
        err.summary()
            .contains("unresolved external function @no_such_function_anywhere"),
        "{}",
        err.summary()
    );
}
