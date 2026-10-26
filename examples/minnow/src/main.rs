//! `minnow`: run or compile Minnow programs (`minnow run|build|ir|hir <file.mn>`).

use std::process::ExitCode;

use a0_driver::{CompileError, Language, Modules, Program};

fn main() -> ExitCode {
    Language::new("minnow", |text, source| {
        match minnow::compile(text, source) {
            Ok(m) => Ok(Program::Hir(m)),
            Err(d) => Err(CompileError::Diagnostics(d)),
        }
    })
    .grammar(minnow::GRAMMAR)
    .modules(Modules::new("mn", minnow::start, minnow::compile_file))
    .repl_main("fn main() {\n{}}\n")
    .main()
}
