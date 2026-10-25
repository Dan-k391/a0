//! The modules a source file imports, from the grammar's `@import` annotations.
//!
//! A rule marked `@import` imports the module named by its `path` field: a string literal is a file
//! path relative to the importing file (`"lib/json.mn"`), anything else is a module name whose
//! segments are separated by `.`, `::` or `/` (`text.json`, which is the file `text/json.<ext>`).
//! An optional `alias` field names the module in the importing file (by default its last segment),
//! and `name` fields import single items unqualified (`import json.{parse}`).

use a0_diag::Span;

use crate::grammar::Grammar;
use crate::literal::unquote;
use crate::tree::{NodeId, SyntaxTree};

/// One import, as written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Import {
    /// The module's name: its path's segments joined with `.` (`text.json`), or for a string path
    /// the path without its extension (`lib/json`).
    pub module: String,
    /// The file it is, relative to the importing file's directory, without the extension for a
    /// module name (`text/json`) and as written for a string (`lib/json.mn`).
    pub file: String,
    /// Whether `file` was written with its extension (a string path).
    pub literal: bool,
    /// The name the importing file uses for the module.
    pub alias: String,
    /// Items imported unqualified, with where their names are.
    pub names: Vec<(String, Span)>,
    /// The path as written, and the whole import.
    pub path_span: Span,
    pub span: Span,
}

/// Every import in `tree`, in source order. Works on trees with syntax errors (incomplete imports
/// are left out).
pub fn imports(grammar: &Grammar, tree: &SyntaxTree) -> Vec<Import> {
    let mut out = Vec::new();
    collect(grammar, tree, tree.root(), &mut out);
    out
}

fn collect(grammar: &Grammar, tree: &SyntaxTree, n: NodeId, out: &mut Vec<Import>) {
    if !tree.is_token(n) && grammar.is_import(tree.kind(n)) {
        if let Some(import) = read(tree, n) {
            out.push(import);
        }
        return;
    }
    for c in tree.children(n) {
        if !tree.is_token(c) {
            collect(grammar, tree, c, out);
        }
    }
}

fn read(tree: &SyntaxTree, n: NodeId) -> Option<Import> {
    let path = tree.field_named(n, "path")?;
    let text = tree.text(path);
    let (module, file, literal) = match unquote(text) {
        Ok(s) if text.starts_with('"') => {
            let module = s
                .rsplit_once('.')
                .filter(|(_, ext)| !ext.contains('/'))
                .map_or(s.as_str(), |(stem, _)| stem)
                .to_string();
            (module, s, true)
        }
        _ => {
            let segments = module_path(text);
            if segments.is_empty() {
                return None;
            }
            (segments.join("."), segments.join("/"), false)
        }
    };
    let alias = match tree.field_named(n, "alias") {
        Some(a) => tree.text(a).to_string(),
        None => module
            .rsplit(['.', '/'])
            .next()
            .unwrap_or(&module)
            .to_string(),
    };
    let names = tree
        .fields_named(n, "name")
        .into_iter()
        .map(|m| (tree.text(m).to_string(), tree.span(m)))
        .collect();
    Some(Import {
        module,
        file,
        literal,
        alias,
        names,
        path_span: tree.span(path),
        span: tree.span(n),
    })
}

/// The segments of a module name as written: `text.json`, `text::json` and `text/json` are all
/// `["text", "json"]`. Empty if it has an empty segment.
pub fn module_path(text: &str) -> Vec<String> {
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let segments: Vec<String> = compact
        .replace("::", ".")
        .split(['.', '/'])
        .map(str::to_string)
        .collect();
    if segments.iter().any(String::is_empty) {
        return Vec::new();
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;
    use a0_diag::SourceId;

    const GRAMMAR: &str = r#"
        grammar g;
        skip /[ \t\n]+/;
        token NAME /[a-z]+/;
        token STRING /"[^"]*"/;
        program = (!EOF item)* EOF;
        item @inline = import_decl | use_decl | other;
        import_decl @import = "import" path:module ("as" alias:NAME)? ("{" name:NAME ("," name:NAME)* "}")? ";";
        use_decl @import = "use" path:STRING ";";
        module = NAME ("." NAME)*;
        other = NAME ";";
    "#;

    #[test]
    fn imports_are_read_from_annotated_rules() {
        let g = Grammar::from_str(GRAMMAR, SourceId(0)).unwrap();
        let text =
            "import text . json as j;\nimport util { trim, pad };\nuse \"lib/x.mn\";\nfoo;\n";
        let (tree, diags) = g.parse(text, SourceId(1));
        assert!(diags.is_empty(), "{diags:?}");
        let found = imports(&g, &tree);
        assert_eq!(found.len(), 3);
        assert_eq!(
            (
                found[0].module.as_str(),
                found[0].file.as_str(),
                found[0].alias.as_str()
            ),
            ("text.json", "text/json", "j")
        );
        assert_eq!(found[1].alias, "util");
        let names: Vec<&str> = found[1].names.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["trim", "pad"]);
        assert_eq!(
            (
                found[2].module.as_str(),
                found[2].file.as_str(),
                found[2].literal
            ),
            ("lib/x", "lib/x.mn", true)
        );
        assert_eq!(found[2].alias, "x");
    }

    #[test]
    fn import_rules_need_a_path_field() {
        let bad =
            "grammar g; token NAME /[a-z]+/; program = imp EOF; imp @import = \"import\" NAME;";
        let Err(err) = Grammar::from_str(bad, SourceId(0)) else {
            panic!("the grammar should be rejected");
        };
        assert!(
            err.summary().contains("field called 'path'"),
            "{}",
            err.summary()
        );
    }

    #[test]
    fn module_paths_accept_dots_colons_and_slashes() {
        assert_eq!(module_path("a.b"), ["a", "b"]);
        assert_eq!(module_path("a :: b"), ["a", "b"]);
        assert_eq!(module_path("a/b"), ["a", "b"]);
        assert!(module_path("a..b").is_empty());
    }
}
