//! The engine of the browser playground (`playground/`, D57): A0 itself, built for WebAssembly
//! (wasm32-wasip1), without the native backend. It takes texts rather than files, since a page has
//! none:
//!
//! ```text
//! a0-playground run <grammar> <program> [args...]   run the program
//! a0-playground tree <grammar> <program>            print its syntax tree
//! a0-playground hir <grammar> <program>             print its high-level IR
//! a0-playground ir <grammar> <program>              print its core IR
//! a0-playground wasm <grammar> <program>            print it compiled to WebAssembly (D78)
//! a0-playground wasm-file <name> <text>             the same for core IR or HIR text (by the
//!                                                   name's extension), for tests
//! ```
//!
//! A compiled module links against a fresh instance of this engine, which holds A0's runtime
//! (`playground/compiled.js`).
//!
//! The grammar defines the language with `@hir` forms (`docs/grammar.md`, "Languages without host
//! code"), as for `a0 lang`. Errors are rendered as the `a0` tool renders them, against the grammar
//! or the program; compile errors exit with 1, and a program's own exit status is passed through.

use std::io::Write;
use std::process::ExitCode;

use a0_diag::{Diagnostics, RenderOptions, SourceId, SourceMap};
use a0_syntax::Grammar;

const USAGE: &str = "usage: a0-playground run|tree|hir|ir|wasm <grammar> <program> [args...]\n";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [mode, grammar, program, rest @ ..] = args.as_slice() else {
        eprint!("{USAGE}");
        return ExitCode::from(2);
    };
    if !matches!(
        mode.as_str(),
        "run" | "tree" | "hir" | "ir" | "wasm" | "wasm-file"
    ) || (mode != "run" && !rest.is_empty())
    {
        eprint!("{USAGE}");
        return ExitCode::from(2);
    }
    let mut sources = SourceMap::new();
    let report = |d: &Diagnostics, sources: &SourceMap| {
        eprint!("{}", d.render_with(sources, &RenderOptions::default()));
        ExitCode::from(1)
    };
    if mode == "wasm-file" {
        let (name, text) = (grammar, program);
        let id = sources.add(name, text);
        let module = if name.ends_with(".hir") {
            a0_hir::parse(text, id).and_then(|m| a0_hir::lower(&m))
        } else {
            a0_ir::parse_and_verify(text, id)
        };
        let mut module = match module {
            Ok(m) => m,
            Err(d) => return report(&d, &sources),
        };
        module.set_sources(sources.clone());
        return emit_wasm(&module, &sources);
    }
    let grammar_id = sources.add("grammar", grammar);
    let grammar = match Grammar::from_str(grammar, grammar_id) {
        Ok(g) => g,
        Err(d) => return report(&d, &sources),
    };
    let program_id: SourceId = sources.add("program", program);
    let (tree, diags) = grammar.parse(program, program_id);
    if mode == "tree" {
        println!("{}", tree.to_sexpr(tree.root()));
    }
    if diags.has_errors() {
        return report(&diags, &sources);
    }
    if mode == "tree" {
        return ExitCode::SUCCESS;
    }
    let hir = match a0_hir::dsl::compile(&grammar, &tree, grammar.name()) {
        Ok(m) => m,
        Err(d) => return report(&d, &sources),
    };
    if !hir.warnings().is_empty() {
        eprintln!(
            "{}",
            hir.warnings()
                .render_with(&sources, &RenderOptions::default())
        );
    }
    if mode == "hir" {
        print!("{hir}");
        return ExitCode::SUCCESS;
    }
    let mut module = match a0_hir::lower(&hir) {
        Ok(m) => m,
        Err(d) => return report(&d, &sources),
    };
    if mode == "ir" {
        print!("{module}");
        return ExitCode::SUCCESS;
    }
    module.set_sources(sources.clone());
    if mode == "wasm" {
        return emit_wasm(&module, &sources);
    }
    let mut argv = vec!["program".to_string()];
    argv.extend(rest.iter().cloned());
    match a0_interp::run_main(module, a0_interp::HostSymbols::new(), argv) {
        Err(d) => report(&d, &sources),
        Ok(Err(trap)) => trap.report_and_exit(),
        Ok(Ok(code)) => a0_runtime::exit(code),
    }
}

/// Writes the module compiled to WebAssembly to standard output.
fn emit_wasm(module: &a0_ir::Module, sources: &SourceMap) -> ExitCode {
    match a0_wasm::compile(module) {
        Ok(bytes) => {
            let mut out = std::io::stdout();
            if out.write_all(&bytes).and_then(|_| out.flush()).is_err() {
                return ExitCode::from(1);
            }
            ExitCode::SUCCESS
        }
        Err(d) => {
            eprint!("{}", d.render_with(sources, &RenderOptions::default()));
            ExitCode::from(1)
        }
    }
}
