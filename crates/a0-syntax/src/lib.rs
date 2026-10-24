//! The A0 grammar layer: from a grammar file to syntax trees with good diagnostics.
//!
//! A language is described by a grammar written in a small DSL (see `docs/grammar.md`). The grammar
//! is loaded at run time with [`Grammar::from_str`]; [`Grammar::parse`] turns source text into a
//! [`SyntaxTree`] plus [`a0_diag::Diagnostics`]. Trees are uniform (every rule makes a node of its
//! kind, every token is a leaf) and children can be looked up by field label, so a frontend walks
//! them directly instead of defining its own AST types.
//!
//! ```
//! use a0_diag::SourceId;
//! use a0_syntax::Grammar;
//!
//! let grammar = Grammar::from_str(r#"
//!     grammar calc;
//!     skip /[ \t\n]+/;
//!     token NUM /[0-9]+/;
//!     program = expr EOF;
//!     expr @inline = @prec atom ~ "expected a number" {
//!         left "+" "-" => binary;
//!         left "*" "/" => binary;
//!         prefix "-" => negate;
//!     };
//!     atom = NUM | "(" expr ")" ~ "expected ')'";
//! "#, SourceId(0)).unwrap();
//! let (tree, diags) = grammar.parse("1 + 2 * -3", SourceId(1));
//! assert!(diags.is_empty());
//! assert_eq!(
//!     tree.to_sexpr(tree.root()),
//!     "(program (binary left=(atom '1') op='+' right=(binary left=(atom '2') op='*' right=(negate op='-' operand=(atom '3')))))"
//! );
//! ```

mod grammar;
mod index;
mod literal;
mod parser;
mod precedence;
mod regex;
mod tree;

pub use grammar::{Definition, FieldId, Grammar, KindId, SymbolKind};
pub use index::{IndexedToken, SyntaxIndex, TokenClass};
pub use literal::unquote;
pub use parser::Token;
pub use precedence::{Combine, Operand, resolve_precedence};
pub use tree::{NodeId, SyntaxTree};
