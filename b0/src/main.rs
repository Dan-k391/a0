//! `b0`: run or compile B0 programs (`b0 run|build|ir|hir <file.b0>`).

use std::process::ExitCode;

use a0_driver::{CompileError, Language, Modules, Program};

fn main() -> ExitCode {
    Language::new("b0", |text, source| match b0::compile(text, source) {
        Ok(m) => Ok(Program::Hir(m)),
        Err(d) => Err(CompileError::Diagnostics(d)),
    })
    .grammar(b0::GRAMMAR)
    .modules(Modules::new("b0", b0::start, b0::compile_file))
    .repl_main("fn main() {\n{}}\n")
    .main()
}
