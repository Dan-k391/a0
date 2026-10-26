//! `tin`: run or compile Tin programs (`tin run|build|ir|hir <file.tin>`).

use std::process::ExitCode;

use a0_driver::{Codes, CompileError, ExitCodes, Language, Program};

fn main() -> ExitCode {
    Language {
        name: "tin",
        compile: |text, source| match tin::compile(text, source) {
            Ok(m) => Ok(Program::Hir(m)),
            Err(d) => Err(CompileError::Diagnostics(d)),
        },
        exit_codes: ExitCodes::default(),
        run_by_default: false,
        host_symbols: Some(tin::host_symbols),
        libraries: Some(|| vec![tin::runtime_library()]),
        grammar: Some(tin::GRAMMAR),
        codes: Codes::A0,
        modules: None,
        repl_main: Some("fn main() -> i32 {\n{}return 0;\n}\n"),
    }
    .main()
}
