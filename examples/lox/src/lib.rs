//! Lox (from Crafting Interpreters) built on A0.
//!
//! The grammar (`lox.a0g`) reproduces jlox's syntax errors; [`analyze`] performs jlox's parse-time
//! checks and its resolver; [`emit`] builds a HIR module on top of Lox's runtime, which is itself
//! written in HIR (`prelude.hir`). Programs run in the A0 interpreter or as native executables, with
//! jlox's output, error messages and exit codes.

pub mod analyze;
pub mod emit;

use std::fmt;

use a0_diag::{SourceId, SourceMap};
use a0_hir::HirModule;
use a0_ir::Module;
use a0_syntax::Grammar;

/// The Lox grammar.
pub const GRAMMAR: &str = include_str!("../lox.a0g");

/// Exit status for compile errors (`EX_DATAERR`), as in jlox.
pub const EXIT_COMPILE_ERROR: u8 = 65;

/// Loads the grammar. It is part of this crate, so failing to load it is a bug.
pub fn grammar() -> Grammar {
    match Grammar::from_str(GRAMMAR, SourceId(u32::MAX)) {
        Ok(g) => g,
        Err(d) => panic!("lox.a0g is invalid:\n{}", d.summary()),
    }
}

/// Where a compile error is reported, in jlox's terms.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErrorPlace {
    /// A lexical error: `Error: message`.
    Lexical,
    /// `Error at end: message`.
    End,
    /// `Error at 'lexeme': message`.
    Token(String),
}

/// A compile error, printed exactly like jlox prints it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompileError {
    pub line: u32,
    pub place: ErrorPlace,
    pub message: String,
    /// Byte offset, for ordering errors by position.
    pub offset: u32,
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.place {
            ErrorPlace::Lexical => write!(f, "[line {}] Error: {}", self.line, self.message),
            ErrorPlace::End => write!(f, "[line {}] Error at end: {}", self.line, self.message),
            ErrorPlace::Token(t) => {
                write!(f, "[line {}] Error at '{}': {}", self.line, t, self.message)
            }
        }
    }
}

/// Compiles a Lox program to an IR module whose `@main` runs the script.
pub fn compile(text: &str) -> Result<Module, Vec<CompileError>> {
    let hir = compile_hir(text)?;
    match a0_hir::lower(&hir) {
        Ok(m) => Ok(m),
        Err(d) => panic!("the HIR of a Lox program does not lower:\n{}", d.summary()),
    }
}

/// Compiles a Lox program to a HIR module (the prelude plus the program).
pub fn compile_hir(text: &str) -> Result<HirModule, Vec<CompileError>> {
    let mut sources = SourceMap::new();
    let id = sources.add("script", text);
    let g = grammar();
    let (tree, diags) = g.parse(text, id);
    let lines = Lines {
        sources: &sources,
        source: id,
    };

    // Syntax errors, plus jlox's checks that happen while parsing (assignment targets, argument and
    // parameter limits). If there are any, the resolver does not run, just like jlox.
    let mut errors: Vec<CompileError> = diags
        .as_slice()
        .iter()
        .map(|d| {
            let span = d.span.expect("syntax diagnostics have spans");
            let place = if d.code == "syntax.lexical" {
                ErrorPlace::Lexical
            } else if span.start as usize == text.len() {
                ErrorPlace::End
            } else {
                ErrorPlace::Token(sources.slice(span).to_string())
            };
            CompileError {
                line: sources.line(id, span.end),
                place,
                message: d.message.clone(),
                offset: span.start,
            }
        })
        .collect();
    let names = analyze::Names::new(&g);
    errors.extend(analyze::parse_time_checks(&tree, &names, &lines));
    if !errors.is_empty() {
        sort_errors(&mut errors);
        return Err(errors);
    }
    let analysis = match analyze::resolve(&tree, &names, &lines) {
        Ok(a) => a,
        Err(mut errors) => {
            sort_errors(&mut errors);
            return Err(errors);
        }
    };
    Ok(emit::generate(&tree, &names, &analysis, &lines))
}

fn sort_errors(errors: &mut [CompileError]) {
    // Lexical errors come first, as jlox scans before it parses; the rest in source order.
    errors.sort_by_key(|e| (e.place != ErrorPlace::Lexical, e.offset));
}

/// Line numbers of tree nodes, as jlox counts them (the line where a token ends).
pub struct Lines<'a> {
    sources: &'a SourceMap,
    source: SourceId,
}

impl Lines<'_> {
    pub fn of(&self, tree: &a0_syntax::SyntaxTree, node: a0_syntax::NodeId) -> u32 {
        self.sources.line(self.source, tree.span(node).end)
    }

    /// The error a token produces in jlox's format.
    pub fn error(
        &self,
        tree: &a0_syntax::SyntaxTree,
        token: a0_syntax::NodeId,
        message: &str,
    ) -> CompileError {
        CompileError {
            line: self.of(tree, token),
            place: ErrorPlace::Token(tree.text(token).to_string()),
            message: message.to_string(),
            offset: tree.span(token).start,
        }
    }
}
