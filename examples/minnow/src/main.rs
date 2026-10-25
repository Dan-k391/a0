//! `minnow`: run or compile Minnow programs (`minnow run|build|ir|hir <file.mn>`).

use std::process::ExitCode;

use a0_driver::{Codes, CompileError, ExitCodes, Language, Modules, Program};

fn main() -> ExitCode {
    Language {
        name: "minnow",
        compile: |text, source| match minnow::compile(text, source) {
            Ok(m) => Ok(Program::Hir(m)),
            Err(d) => Err(CompileError::Diagnostics(d)),
        },
        exit_codes: ExitCodes::default(),
        run_by_default: false,
        host_symbols: None,
        libraries: None,
        grammar: Some(minnow::GRAMMAR),
        codes: Codes::A0,
        modules: Some(Modules {
            extension: "mn",
            start: minnow::start,
            compile: minnow::compile_file,
        }),
        repl_main: Some("fn main() {\n{}}\n"),
    }
    .main()
}
