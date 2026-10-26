use a0_diag::{SourceId, SourceMap};
use a0_syntax::{Grammar, KindId};

fn grammar(text: &str) -> Grammar {
    let mut sm = SourceMap::new();
    let id = sm.add("test.a0g", text);
    match Grammar::from_str(text, id) {
        Ok(g) => g,
        Err(d) => panic!("grammar errors:\n{}", d.render(&sm)),
    }
}

fn grammar_errors(text: &str) -> String {
    let mut sm = SourceMap::new();
    let id = sm.add("test.a0g", text);
    match Grammar::from_str(text, id) {
        Ok(_) => String::new(),
        Err(d) => d.summary(),
    }
}

fn lox() -> Grammar {
    grammar(include_str!("data/lox.a0g"))
}

/// Formats syntax diagnostics the way jlox does: `[line N] Error at 'x': message`.
fn jlox_errors(g: &Grammar, source: &str) -> Vec<String> {
    let mut sm = SourceMap::new();
    let id = sm.add("test.lox", source);
    let (_, diags) = g.parse(source, id);
    diags
        .as_slice()
        .iter()
        .map(|d| {
            let span = d.span.unwrap();
            if d.code == a0_diag::codes::INVALID_TOKEN {
                format!("[line {}] Error: {}", sm.line(id, span.end), d.message)
            } else if span.start as usize == source.len() {
                format!(
                    "[line {}] Error at end: {}",
                    sm.line(id, span.start),
                    d.message
                )
            } else {
                format!(
                    "[line {}] Error at '{}': {}",
                    sm.line(id, span.start),
                    sm.slice(span),
                    d.message
                )
            }
        })
        .collect()
}

#[test]
fn grammar_validation_reports_problems() {
    let e = grammar_errors("program = missing;");
    assert!(e.contains("'missing' is not a rule or token"), "{e}");

    let e = grammar_errors("expr = expr \"+\" NUM | NUM; token NUM /[0-9]+/;");
    assert!(e.contains("rule 'expr' is left-recursive"), "{e}");

    let e = grammar_errors("a = b c; b = \"x\"?; c = a;");
    assert!(e.contains("rule 'a' is left-recursive"), "{e}");

    let e = grammar_errors("a = (\"x\"?)*;");
    assert!(e.contains("can match nothing"), "{e}");

    let e = grammar_errors("token T /[a-/; a = T;");
    assert!(e.contains("invalid pattern"), "{e}");

    let e = grammar_errors("skip /a*/; a = \"x\";");
    assert!(e.contains("can match empty text"), "{e}");

    let e = grammar_errors("a = \"x\"; a = \"y\";");
    assert!(e.contains("'a' is defined more than once"), "{e}");

    let e = grammar_errors("a @inline = \"x\";");
    assert!(e.contains("cannot be @inline"), "{e}");

    let e = grammar_errors("a = \"x\"\nb = \"y\";");
    assert!(e.contains("expected"), "{e}");
}

#[test]
fn lexer_uses_longest_match_and_literal_priority() {
    let g = lox();
    let (toks, diags) = g.lex("printer print 12.5 >= > 1.", SourceId(0));
    assert!(diags.is_empty());
    let kinds: Vec<KindId> = toks.iter().map(|t| t.kind).collect();
    let ident = g.kind("IDENTIFIER").unwrap();
    let number = g.kind("NUMBER").unwrap();
    assert_eq!(
        kinds,
        vec![
            ident,
            g.literal("print").unwrap(),
            number,
            g.literal(">=").unwrap(),
            g.literal(">").unwrap(),
            number,
            g.literal(".").unwrap(),
            KindId::EOF
        ]
    );
    assert_eq!((toks[2].start, toks[2].end), (14, 18));
}

#[test]
fn lexer_reports_and_skips_bad_characters() {
    let g = lox();
    let (toks, diags) = g.lex("a | b @", SourceId(0));
    assert_eq!(toks.len(), 3); // a, b, EOF
    let msgs: Vec<&str> = diags
        .as_slice()
        .iter()
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(msgs, vec!["Unexpected character.", "Unexpected character."]);
}

#[test]
fn trees_have_kinds_and_fields() {
    let g = lox();
    let src = "var x = 1 + 2 * 3;\nif (x > 1) print -x; else { print f(x, \"s\").y; }";
    let (tree, diags) = g.parse(src, SourceId(0));
    assert!(diags.is_empty(), "{}", diags.summary());
    let root = tree.root();
    assert_eq!(tree.kind_name(root), "program");
    let decls = tree.children(root);
    assert_eq!(decls.len(), 2);
    assert_eq!(
        tree.to_sexpr(decls[0]),
        "(var_decl 'var' name='x' '=' init=(binary left=(primary '1') op='+' right=(binary left=(primary '2') op='*' right=(primary '3'))) ';')"
    );
    let if_stmt = decls[1];
    assert_eq!(tree.kind_name(if_stmt), "if_stmt");
    let cond = tree.field_named(if_stmt, "cond").unwrap();
    assert_eq!(tree.text(cond), "x > 1");
    let els = tree.field_named(if_stmt, "else").unwrap();
    assert_eq!(tree.kind_name(els), "block");
    let print = tree.children(els)[1];
    let value = tree.field_named(print, "value").unwrap();
    assert_eq!(tree.kind_name(value), "get");
    let call = tree.field_named(value, "operand").unwrap();
    assert_eq!(tree.kind_name(call), "call");
    let args = tree.field_named(call, "args").unwrap();
    assert_eq!(tree.fields_named(args, "arg").len(), 2);
    assert_eq!(tree.text(tree.field_named(value, "name").unwrap()), "y");
}

#[test]
fn precedence_and_associativity() {
    let g = lox();
    let (tree, diags) = g.parse("a = b = c or d and e == -f.g(h) - i - j;", SourceId(0));
    assert!(diags.is_empty(), "{}", diags.summary());
    let stmt = tree.children(tree.root())[0];
    let expr = tree.field_named(stmt, "expr").unwrap();
    assert_eq!(
        tree.to_sexpr(expr),
        "(assign left=(primary 'a') op='=' right=(assign left=(primary 'b') op='=' right=\
         (logical left=(primary 'c') op='or' right=(logical left=(primary 'd') op='and' right=\
         (binary left=(primary 'e') op='==' right=(binary left=(binary left=(unary op='-' operand=\
         (call operand=(get operand=(primary 'f') '.' name='g') '(' args=(arguments arg=(primary 'h')) ')')) \
         op='-' right=(primary 'i')) op='-' right=(primary 'j')))))))"
    );
}

#[test]
fn jlox_error_messages() {
    let g = lox();
    let cases: &[(&str, &[&str])] = &[
        ("print;", &["[line 1] Error at ';': Expect expression."]),
        (".123;", &["[line 1] Error at '.': Expect expression."]),
        (
            "123.;",
            &["[line 1] Error at ';': Expect property name after '.'."],
        ),
        (
            "123.",
            &["[line 1] Error at end: Expect property name after '.'."],
        ),
        (
            "print 1",
            &["[line 1] Error at end: Expect ';' after value."],
        ),
        (
            "var false = \"value\";",
            &["[line 1] Error at 'false': Expect variable name."],
        ),
        (
            "fun f() 123;",
            &["[line 1] Error at '123': Expect '{' before function body."],
        ),
        (
            "fun foo(a, b c, d, e, f) {}",
            &["[line 1] Error at 'c': Expect ')' after parameters."],
        ),
        (
            "if (true) class Foo {}",
            &["[line 1] Error at 'class': Expect expression."],
        ),
        (
            "if (true) \"ok\"; else var foo;",
            &["[line 1] Error at 'var': Expect expression."],
        ),
        (
            "while (true) fun foo() {}",
            &["[line 1] Error at 'fun': Expect expression."],
        ),
        (
            "class Bar < (Foo) {}",
            &["[line 1] Error at '(': Expect superclass name."],
        ),
        (
            "class A { method() { (super).method(); } }",
            &["[line 1] Error at ')': Expect '.' after 'super'."],
        ),
        (
            "class A { method() { super; } }",
            &["[line 1] Error at ';': Expect '.' after 'super'."],
        ),
        (
            "class A { method() { super.; } }",
            &["[line 1] Error at ';': Expect superclass method name."],
        ),
        (
            "class A { 123 }",
            &["[line 1] Error at '123': Expect method name."],
        ),
        (
            "{ var a = 1;",
            &["[line 1] Error at end: Expect '}' after block."],
        ),
        (
            "for (var a = 1; {}; a = a + 1) {}",
            &[
                "[line 1] Error at '{': Expect expression.",
                "[line 1] Error at ')': Expect ';' after expression.",
            ],
        ),
        (
            "for ({}; a < 2; a = a + 1) {}",
            &[
                "[line 1] Error at '{': Expect expression.",
                "[line 1] Error at ')': Expect ';' after expression.",
            ],
        ),
        (
            "for (var a = 1; a < 2; {}) {}",
            &["[line 1] Error at '{': Expect expression."],
        ),
        (
            "// comment\n\"this string has no close quote",
            &["[line 2] Error: Unterminated string."],
        ),
        (
            "\n\nfoo(a | b);",
            &[
                "[line 3] Error: Unexpected character.",
                "[line 3] Error at 'b': Expect ')' after arguments.",
            ],
        ),
        (
            "print 1;\nprint 2",
            &["[line 2] Error at end: Expect ';' after value."],
        ),
    ];
    let mut failures = Vec::new();
    for (src, expected) in cases {
        let got = jlox_errors(&g, src);
        let want: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
        if got != want {
            failures.push(format!("{src:?}\n  got:  {got:?}\n  want: {want:?}"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn recovery_continues_after_errors() {
    let g = lox();
    let src = "print;\nvar 1;\nprint \"ok\";\n{ print; print 2; }\nfun () {}\n";
    let got = jlox_errors(&g, src);
    assert_eq!(
        got,
        vec![
            "[line 1] Error at ';': Expect expression.",
            "[line 2] Error at '1': Expect variable name.",
            "[line 4] Error at ';': Expect expression.",
            "[line 5] Error at '(': Expect function name.",
        ]
    );
    let (tree, _) = g.parse(src, SourceId(0));
    assert!(tree.has_errors());
    let kinds: Vec<String> = tree
        .children(tree.root())
        .iter()
        .map(|&n| tree.kind_name(n).to_string())
        .collect();
    assert_eq!(
        kinds,
        vec!["error", "error", "print_stmt", "block", "error"]
    );
    // The block kept its good statement next to the error.
    let block = tree.children(tree.root())[3];
    let inner: Vec<String> = tree
        .children(block)
        .iter()
        .map(|&n| tree.kind_name(n).to_string())
        .collect();
    assert_eq!(inner, vec!["\"{\"", "error", "print_stmt", "\"}\""]);
}

#[test]
fn generic_messages_list_expected_tokens() {
    let g = grammar(
        "grammar g; skip /[ ]+/; token ID /[a-z]+/;
         list = \"[\" (ID (\",\" ID)*)? \"]\" EOF;",
    );
    let (_, diags) = g.parse("[a b]", SourceId(0));
    assert_eq!(diags.summary(), "error: expected ',' or ']', found 'b'\n");
    let (_, diags) = g.parse("[a,", SourceId(0));
    assert_eq!(diags.summary(), "error: expected ID, found end of input\n");
    let (_, diags) = g.parse("[a] b", SourceId(0));
    assert_eq!(diags.summary(), "error: expected end of input, found 'b'\n");
}

#[test]
fn lookahead_and_repetition() {
    let g = grammar(
        "grammar g; skip /[ ]+/; token ID /[a-z]+/;
         items = (!\"end\" item:ID)* \"end\" &EOF;",
    );
    let (tree, diags) = g.parse("a b c end", SourceId(0));
    assert!(diags.is_empty());
    assert_eq!(tree.fields_named(tree.root(), "item").len(), 3);
}

#[test]
fn rules_can_give_their_nodes_hir_forms() {
    let g = grammar(
        "token N /[0-9]+/; skip / +/; program = (item)*; item @inline = let_stmt | N;
         let_stmt @hir(let) = \"let\" value:N;",
    );
    let kind = |name: &str| g.kind(name).unwrap();
    assert_eq!(g.hir_form(kind("let_stmt")), Some("let"));
    assert_eq!(g.hir_form(kind("program")), None);
    assert_eq!(g.hir_form(kind("N")), None);
    let e = grammar_errors("token N /[0-9]+/; program = x; x @inline @hir(int) = N;");
    assert!(e.contains("'x' is @inline"), "{e}");
    let e = grammar_errors("token N /[0-9]+/; program @hir = N;");
    assert!(e.contains("expected '('"), "{e}");
}

/// Nesting past `MAX_NESTING`, by recursion (parentheses) or by a chain an operator loop builds,
/// is one diagnostic (D83).
#[test]
fn nesting_too_deep_is_reported() {
    // Parsing recurses: on a thread with room for it, as the drivers parse.
    std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(|| {
            let g = grammar(
                "expr = atom (\"+\" atom)*; atom = NUM | \"(\" expr \")\"; token NUM /[0-9]+/; \
                 skip / +/;",
            );
            let n = a0_syntax::MAX_NESTING + 10;
            let deep = format!("{}1{}", "(".repeat(n), ")".repeat(n));
            let (_, diags) = g.parse(&deep, SourceId(0));
            assert_eq!(diags.len(), 1);
            assert_eq!(diags.get(0).code, "E0003");
            let (_, diags) = g.parse("((1)) + 2", SourceId(0));
            assert!(diags.is_empty());
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn grammars_name_their_format_version() {
    grammar("grammar g version 1;\nstart = \"x\";\n");
    grammar("grammar g;\nstart = \"x\";\n");
    let mut sm = SourceMap::new();
    let text = "grammar g version 2;\nstart = \"x\";\n";
    let id = sm.add("test.a0g", text);
    let e = Grammar::from_str(text, id).err().unwrap().render(&sm);
    assert!(e.contains("[E0004]") && e.contains("version 2"), "{e}");
    let e = grammar_errors("grammar g version x;\nstart = \"x\";\n");
    assert!(e.contains("expected the format version"), "{e}");
}
