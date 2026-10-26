//! `kaleidoscope`: run or compile Kaleidoscope programs (`kaleidoscope run|build|ir <file.k>`).

use std::process::ExitCode;

use a0_driver::{CompileError, Language, Program};

fn main() -> ExitCode {
    Language::new("kaleidoscope", |text, source| {
        match kaleidoscope::compile(text, source) {
            Ok(m) => Ok(Program::Hir(m)),
            Err(d) => Err(CompileError::Diagnostics(d)),
        }
    })
    .grammar(kaleidoscope::GRAMMAR)
    .runtime(kaleidoscope::host_symbols, || {
        vec![kaleidoscope::runtime_library()]
    })
    .main()
}
