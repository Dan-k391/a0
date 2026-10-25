//! `a0 lang <grammar.a0g> <command> ...`: runs a language defined by its grammar alone, whose rules
//! say what their nodes mean with `@hir` forms (`a0_hir::dsl`, D55). The commands are those of
//! every language's driver (`run`, `build`, `hir`, `ir`, `lsp`, `explain`), so such a language gets
//! both backends and a language server with no code of its own.

use std::path::Path;
use std::process::ExitCode;
use std::sync::OnceLock;

use a0_diag::{RenderOptions, SourceId, SourceMap};
use a0_driver::{Codes, CompileError, ExitCodes, Language, Program};
use a0_syntax::Grammar;

/// The grammar and the language's name, set once before the driver runs; the driver takes plain
/// functions, so `compile` finds them here.
static GRAMMAR: OnceLock<(Grammar, &'static str)> = OnceLock::new();

pub fn lang(args: &[String]) -> ExitCode {
    let Some((path, rest)) = args.split_first() else {
        eprint!("{}", crate::USAGE);
        return ExitCode::from(2);
    };
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("a0: cannot read {path}: {e}");
            return ExitCode::from(2);
        }
    };
    // Grammar errors point into the grammar file, which the driver's sources do not have.
    let mut sources = SourceMap::new();
    let id = sources.add(path, &text);
    if let Err(d) = Grammar::from_str(&text, id) {
        eprint!("{}", d.render_with(&sources, &RenderOptions::stderr()));
        return ExitCode::from(2);
    }
    let grammar = match Grammar::from_str(&text, SourceId(u32::MAX)) {
        Ok(g) => g,
        Err(_) => unreachable!("the grammar was checked above"),
    };
    let stem = Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "lang".to_string());
    let name: &'static str = Box::leak(stem.into_boxed_str());
    GRAMMAR.set((grammar, name)).ok();
    let language = Language {
        name,
        compile,
        exit_codes: ExitCodes::default(),
        run_by_default: false,
        host_symbols: None,
        libraries: None,
        grammar: Some(Box::leak(text.into_boxed_str())),
        codes: Codes::A0,
        modules: None,
    };
    language.run(rest)
}

fn compile(text: &str, source: SourceId) -> Result<Program, CompileError> {
    let (grammar, name) = GRAMMAR.get().expect("the grammar is set before compiling");
    let (tree, diags) = grammar.parse(text, source);
    if diags.has_errors() {
        return Err(CompileError::Diagnostics(diags));
    }
    a0_hir::dsl::compile(grammar, &tree, name)
        .map(Program::Hir)
        .map_err(CompileError::Diagnostics)
}
