//! `kaleidoscope`: run or compile Kaleidoscope programs (`kaleidoscope run|build|ir <file.k>`).

use std::process::ExitCode;

use a0_driver::{CompileError, ExitCodes, Language, Program};

fn main() -> ExitCode {
    Language {
        name: "kaleidoscope",
        compile: |text, source| match kaleidoscope::compile(text, source) {
            Ok(m) => Ok(Program::Ir(m)),
            Err(d) => Err(CompileError::Diagnostics(d)),
        },
        exit_codes: ExitCodes::default(),
        run_by_default: false,
        host_symbols: Some(kaleidoscope::host_symbols),
        libraries: Some(|| vec![kaleidoscope::runtime_library()]),
        grammar: Some(kaleidoscope::GRAMMAR),
    }
    .main()
}
