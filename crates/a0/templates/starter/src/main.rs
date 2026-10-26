//! `{{name}}`: run, compile and serve {{Name}} programs.
//!
//! ```text
//! {{name}} run hello.{{ext}}            # in A0's interpreter
//! {{name}} build hello.{{ext}} -o hello # a native executable
//! {{name}} hir hello.{{ext}}            # the HIR the compiler builds
//! {{name}} lsp                      # the language server, for editors
//! ```

use std::process::ExitCode;

use a0_driver::{CompileError, Language, Program};

fn main() -> ExitCode {
    Language::new("{{name}}", |text, source| match {{crate}}::compile(text, source) {
        Ok(m) => Ok(Program::Hir(m)),
        Err(d) => Err(CompileError::Diagnostics(d)),
    })
    .grammar({{crate}}::GRAMMAR)
    .repl_main("fn main() {\n{}}\n")
    .main()
}
