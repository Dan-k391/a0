//! `tin`: run or compile Tin programs (`tin run|build|ir|hir <file.tin>`).

use std::process::ExitCode;

use a0_driver::{CompileError, Language, Program};

fn main() -> ExitCode {
    Language::new("tin", |text, source| match tin::compile(text, source) {
        Ok(m) => Ok(Program::Hir(m)),
        Err(d) => Err(CompileError::Diagnostics(d)),
    })
    .grammar(tin::GRAMMAR)
    .runtime(tin::host_symbols, || vec![tin::runtime_library()])
    .repl_main("fn main() -> i32 {\n{}return 0;\n}\n")
    .main()
}
