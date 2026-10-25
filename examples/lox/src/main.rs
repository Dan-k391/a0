//! `lox`: run or compile Lox programs, with jlox's command line behavior.
//!
//! `lox file.lox` (or `lox run file.lox`) interprets the program; `lox build file.lox [-o out]` makes a
//! native executable; `lox hir file.lox` prints the HIR and `lox ir file.lox` the core IR. Compile
//! errors exit with 65 and runtime errors with 70, as in jlox.

use std::process::ExitCode;

use a0_diag::Span;
use a0_driver::{Codes, CompileError, ExitCodes, Language, Message, Program};

fn main() -> ExitCode {
    Language {
        name: "lox",
        compile: |text, source| match lox::compile_hir(text) {
            Ok(m) => Ok(Program::Hir(m)),
            Err(errors) => Err(CompileError::Messages(
                errors
                    .iter()
                    .map(|e| {
                        let span = Span::new(source, e.offset, e.end);
                        Message::new(&e.to_string(), Some(span))
                    })
                    .collect(),
            )),
        },
        // The BSD sysexits codes jlox uses: EX_USAGE, EX_NOINPUT, EX_DATAERR.
        exit_codes: ExitCodes {
            usage: 64,
            io: 66,
            compile: lox::EXIT_COMPILE_ERROR,
            internal: 1,
        },
        run_by_default: true,
        host_symbols: None,
        libraries: None,
        grammar: Some(lox::GRAMMAR),
        codes: Codes::A0,
        modules: None,
    }
    .main()
}
