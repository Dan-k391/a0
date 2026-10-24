//! Minnow: a small statically typed language built on A0.
//!
//! ```text
//! struct Point { x: float, y: float }
//!
//! fn scale(p: Point, k: float) -> Point {
//!   return new Point { x: p.x * k, y: p.y * k };
//! }
//!
//! fn main() {
//!   let points: [Point] = [];
//!   for i in [1, 2, 3] {
//!     push(points, scale(new Point { x: 1.0, y: 2.0 }, i as float));
//!   }
//!   let total = 0.0;
//!   let add = fn(p: Point) { total = total + p.x + p.y; };
//!   for p in points { add(p); }
//!   print(total);
//! }
//! ```
//!
//! Types are `int`, `float`, `bool`, `string`, arrays `[T]`, functions `fn(T, ...) -> R` and structs.
//! Closures capture variables by reference. The pipeline: the grammar (`minnow.a0g`) parses; the
//! checker ([`check`]) resolves names and types with `a0-sema`'s scopes and type toolkit; the emitter
//! ([`emit`]) builds statically typed HIR, which A0 lowers and runs or compiles natively.

pub mod check;
pub mod emit;

use a0_diag::{Diagnostic, Diagnostics, SourceId};
use a0_hir::HirModule;
use a0_syntax::{Grammar, NodeId, SyntaxTree};

/// The Minnow grammar.
pub const GRAMMAR: &str = include_str!("../minnow.a0g");

/// Loads the grammar. It is part of this crate, so failing to load it is a bug.
pub fn grammar() -> Grammar {
    match Grammar::from_str(GRAMMAR, SourceId(u32::MAX)) {
        Ok(g) => g,
        Err(d) => panic!("minnow.a0g is invalid:\n{}", d.summary()),
    }
}

/// Compiles a program to HIR. `source` is the id of `text` in the caller's source map.
pub fn compile(text: &str, source: SourceId) -> Result<HirModule, Diagnostics> {
    let (tree, diags) = grammar().parse(text, source);
    if diags.has_errors() {
        return Err(diags);
    }
    let escapes = check_escapes(&tree);
    if escapes.has_errors() {
        return Err(escapes);
    }
    let checked = check::check(&tree)?;
    emit::emit(&tree, &checked).map_err(|hir| {
        // The emitter produced invalid HIR: a bug in this crate, but report it rather than crash.
        let mut d = Diagnostics::new();
        d.push(Diagnostic::error(
            "minnow.internal",
            "the compiler produced invalid HIR",
        ));
        d.extend(hir);
        d
    })
}

/// Decodes the escapes in a string literal's contents: `\n`, `\t`, `\r`, `\0`, `\\`, `\"`. Returns
/// the first unknown escape character on failure.
pub fn unescape(s: &str) -> Result<String, char> {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        out.push(match chars.next() {
            Some('n') => '\n',
            Some('t') => '\t',
            Some('r') => '\r',
            Some('0') => '\0',
            Some('\\') => '\\',
            Some('"') => '"',
            Some(other) => return Err(other),
            None => return Err('\\'),
        });
    }
    Ok(out)
}

fn check_escapes(tree: &SyntaxTree) -> Diagnostics {
    let mut diags = Diagnostics::new();
    let mut stack: Vec<NodeId> = vec![tree.root()];
    while let Some(n) = stack.pop() {
        if tree.is_token(n) {
            let text = tree.text(n);
            if tree.kind_name(n) == "STRING"
                && let Err(c) = unescape(&text[1..text.len() - 1])
            {
                diags.push(
                    Diagnostic::error("minnow.check", &format!("unknown escape `\\{c}`"))
                        .with_span(tree.span(n)),
                );
            }
        } else {
            stack.extend(tree.children(n));
        }
    }
    diags
}
