//! `{{name}}`: run, compile and serve {{Name}} programs.
//!
//! ```text
//! {{name}} run hello.{{ext}}            # in A0's interpreter
//! {{name}} build hello.{{ext}} -o hello # a native executable
//! {{name}} hir hello.{{ext}}            # the HIR the compiler builds
//! {{name}} lsp                      # the language server, for editors
//! ```

use std::process::ExitCode;

use a0_driver::{Codes, CompileError, ExitCodes, Language, Program};

fn main() -> ExitCode {
    Language {
        name: "{{name}}",
        compile: |text, source| match {{crate}}::compile(text, source) {
            Ok(m) => Ok(Program::Hir(m)),
            Err(d) => Err(CompileError::Diagnostics(d)),
        },
        exit_codes: ExitCodes::default(),
        run_by_default: false,
        host_symbols: None,
        libraries: None,
        grammar: Some({{crate}}::GRAMMAR),
        codes: Codes::A0,
        modules: None,
    }
    .main()
}
