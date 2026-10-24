//! The syntax index: token classes, declarations from `@def`, scopes from `@scope`, resolution.

use a0_diag::{SourceId, SourceMap};
use a0_syntax::{Grammar, SymbolKind, SyntaxIndex, TokenClass};

const MINI: &str = r#"
grammar mini;
skip /[ \t\r\n]+/;
skip /#[^\n]*/;
token NUM /[0-9]+/;
token STR /"[^"]*"/;
token ID /[A-Za-z_][A-Za-z0-9_]*/;
program = (!EOF item)* EOF;
item @inline = fn_decl | struct_decl | stmt;
struct_decl @def(struct) = "struct" name:ID "{" field* "}";
field @def(field) = name:ID ";";
fn_decl @def(function) @scope = "fn" name:ID "(" params? ")" body:block;
params @def(parameter: param) = param:ID ("," param:ID)*;
block @scope = "{" (!"}" !EOF stmt)* "}";
stmt @inline @recover(after ";" before "let" "fn") = let_stmt | block | expr_stmt;
let_stmt @def(variable) = "let" name:ID "=" value:expr ";";
expr_stmt = expr:expr ";";
expr @inline = @prec atom ~ "expected an expression" {
    postfix "." name:ID => get;
    postfix "(" args:args? ")" => call;
};
args = arg:expr ("," arg:expr)*;
atom = NUM | STR | ID;
"#;

const PROGRAM: &str = r#"# a comment
let x = 1;
fn f(a, b) {
  let x = a;
  { let a = x; a; }
  b.len;
  g(x, "s");
}
fn g(c) { y; x; c; }
struct P { len; }
let y = 2;
"#;

fn index(grammar: &str, text: &str) -> (Grammar, SyntaxIndex) {
    let g = Grammar::from_str(grammar, SourceId(0)).unwrap();
    let (tree, _) = g.parse(text, SourceId(1));
    let index = SyntaxIndex::new(&g, &tree);
    (g, index)
}

/// `name@line:col` for a byte offset.
fn at(text: &str, offset: u32) -> String {
    let before = &text[..offset as usize];
    let line = before.matches('\n').count() + 1;
    let col = offset as usize - before.rfind('\n').map_or(0, |i| i + 1) + 1;
    format!("{line}:{col}")
}

/// Every name token as `name@line:col -> target@line:col ...`.
fn references(text: &str, index: &SyntaxIndex) -> Vec<String> {
    (0..index.token_count())
        .filter(|&i| index.token(i).class == TokenClass::Name)
        .map(|i| {
            let t = index.token(i);
            let mut line = format!(
                "{}@{}",
                &text[t.start as usize..t.end as usize],
                at(text, t.start)
            );
            if t.declares.is_some() {
                line.push_str(" decl");
            }
            for &s in index.targets(i) {
                line.push_str(&format!(" -> {}", at(text, index.symbol_span(s).start)));
            }
            line
        })
        .collect()
}

#[test]
fn names_resolve_by_scope() {
    let (_, ix) = index(MINI, PROGRAM);
    assert_eq!(
        references(PROGRAM, &ix),
        [
            "x@2:5 decl -> 2:5",
            "f@3:4 decl -> 3:4",
            "a@3:6 decl -> 3:6",
            "b@3:9 decl -> 3:9",
            // The local `x` shadows the global one; its initializer reads the parameter.
            "x@4:7 decl -> 4:7",
            "a@4:11 -> 3:6",
            // In the inner block, `a` is the parameter until the new `a` is declared.
            "a@5:9 decl -> 5:9",
            "x@5:13 -> 4:7",
            "a@5:16 -> 5:9",
            // A member name refers to the members with that name.
            "b@6:3 -> 3:9",
            "len@6:5 -> 10:12",
            // Functions are visible before their declaration.
            "g@7:3 -> 9:4",
            "x@7:5 -> 4:7",
            "g@9:4 decl -> 9:4",
            "c@9:6 decl -> 9:6",
            // So is anything at file level; `x` here is the global.
            "y@9:11 -> 11:5",
            "x@9:14 -> 2:5",
            "c@9:17 -> 9:6",
            "P@10:8 decl -> 10:8",
            "len@10:12 decl -> 10:12",
            "y@11:5 decl -> 11:5",
        ]
    );
}

#[test]
fn symbols_form_an_outline() {
    let (_, ix) = index(MINI, PROGRAM);
    let outline: Vec<String> = (0..ix.symbol_count())
        .map(|s| {
            let parent = ix
                .symbol_parent(s)
                .map(|p| format!(" in {}", ix.symbol_name(p)))
                .unwrap_or_default();
            format!("{:?} {}{parent}", ix.symbol_kind(s), ix.symbol_name(s))
        })
        .collect();
    assert_eq!(
        outline,
        [
            "Variable x",
            "Function f",
            "Parameter a in f",
            "Parameter b in f",
            "Variable x in f",
            "Variable a in f",
            "Function g",
            "Parameter c in g",
            "Struct P",
            "Field len in P",
            "Variable y",
        ]
    );
    // A declaration spans its whole node; a parameter only its name.
    let f = 1;
    assert_eq!(
        &PROGRAM[ix.symbol_extent(f).start as usize..ix.symbol_extent(f).end as usize][..10],
        "fn f(a, b)"
    );
    assert_eq!(ix.symbol_extent(2), ix.symbol_span(2));
}

#[test]
fn tokens_are_classified_from_the_grammar() {
    let (_, ix) = index(MINI, PROGRAM);
    let classes: Vec<(String, TokenClass)> = (0..6)
        .map(|i| {
            let t = ix.token(i);
            (
                PROGRAM[t.start as usize..t.end as usize].to_string(),
                t.class,
            )
        })
        .collect();
    assert_eq!(
        classes,
        [
            ("# a comment".to_string(), TokenClass::Comment),
            ("let".to_string(), TokenClass::Keyword),
            ("x".to_string(), TokenClass::Name),
            ("=".to_string(), TokenClass::Operator),
            ("1".to_string(), TokenClass::Number),
            (";".to_string(), TokenClass::Operator),
        ]
    );
    let s = PROGRAM.find("\"s\"").unwrap() as u32;
    assert_eq!(ix.token(ix.token_at(s).unwrap()).class, TokenClass::String);
    // The cursor right after a name still finds it.
    let len = PROGRAM.find("len;").unwrap() as u32;
    let t = ix.token(ix.token_at(len + 3).unwrap());
    assert_eq!(&PROGRAM[t.start as usize..t.end as usize], "len");
    assert!(t.member);
}

#[test]
fn files_with_syntax_errors_are_indexed() {
    let text = "fn f(a) { let b = ; a; }\nfn g() { f(1); }";
    let (_, ix) = index(MINI, text);
    let names = references(text, &ix);
    assert!(names.contains(&"f@2:10 -> 1:4".to_string()), "{names:?}");
    assert!(names.iter().any(|n| n.starts_with("a@1:21")), "{names:?}");
}

#[test]
fn bad_annotations_are_reported() {
    let errors = |extra: &str| {
        let text = format!(
            "grammar t;\nskip /[ ]+/;\ntoken ID /[a-z]+/;\nprogram = item* EOF;\n{extra}\n"
        );
        let mut sm = SourceMap::new();
        let id = sm.add("t.a0g", &text);
        match Grammar::from_str(&text, id) {
            Ok(_) => String::new(),
            Err(d) => d.summary(),
        }
    };
    assert!(
        errors("item @inline @def(function) = \"fn\" name:ID;").contains("'item' is @inline"),
        "{}",
        errors("item @inline @def(function) = \"fn\" name:ID;")
    );
    let unknown = errors("item @def(fnction) = \"fn\" name:ID;");
    assert!(
        unknown.contains("unknown symbol kind 'fnction'"),
        "{unknown}"
    );
    assert!(unknown.contains("function, method"), "{unknown}");
    let unnamed = errors("item @def(function) = \"fn\" id:ID;");
    assert!(unnamed.contains("@def(kind: field)"), "{unnamed}");
    let missing = errors("item @def(parameter: nope) = \"fn\" id:ID;");
    assert!(missing.contains("there is no field 'nope'"), "{missing}");
    assert_eq!(
        errors("item @def(parameter: id) @scope = \"fn\" id:ID;"),
        ""
    );
    let g = Grammar::from_str(
        "grammar t;\ntoken ID /[a-z]+/;\nprogram = item* EOF;\nitem @def(function) @scope = \"fn\" name:ID;\n",
        SourceId(0),
    )
    .unwrap();
    let item = g.kind("item").unwrap();
    assert!(g.is_scope(item));
    let def = g.definition(item).unwrap();
    assert_eq!((def.kind, def.each), (SymbolKind::Function, false));
    assert_eq!(g.field_name(def.field), "name");
    assert!(!g.is_scope(g.kind("program").unwrap()));
}
