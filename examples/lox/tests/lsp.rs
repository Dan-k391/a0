//! `lox lsp`: the language server the driver derives from Lox's grammar and compiler.

use std::path::Path;

use a0_driver::json::Json;
use a0_driver::lsp::session;

const PROGRAM: &str = "\
class Point {
  init(x, y) { this.x = x; this.y = y; }
  sum() { return this.x + this.y; }
}

fun fib(n) {
  if (n < 2) return n;
  return fib(n - 2) + fib(n - 1);
}

var p = Point(1, 2);
print p.sum() + fib(10);
";

fn run(text: &str, requests: &[(&str, u32, u32)]) -> Vec<Json> {
    let lox = Path::new(env!("CARGO_BIN_EXE_lox"));
    session(lox, "file:///demo.lox", text, requests).unwrap()
}

fn result(messages: &[Json], id: u32) -> &Json {
    messages
        .iter()
        .find(|m| m.get("id").as_u32() == Some(id))
        .map(|m| m.get("result"))
        .unwrap_or_else(|| panic!("no response {id}"))
}

fn outline(symbols: &Json) -> Vec<String> {
    symbols
        .as_array()
        .iter()
        .map(|s| {
            let children = outline(s.get("children"));
            let name = s.get("name").as_str().unwrap();
            if children.is_empty() {
                name.to_string()
            } else {
                format!("{name}({})", children.join(" "))
            }
        })
        .collect()
}

fn start(location: &Json) -> (u32, u32) {
    let s = location.get("range").get("start");
    (
        s.get("line").as_u32().unwrap(),
        s.get("character").as_u32().unwrap(),
    )
}

fn hover(messages: &[Json], id: u32) -> &str {
    let contents = result(messages, id).get("contents");
    contents.get("value").as_str().unwrap()
}

#[test]
fn outline_definitions_and_hovers_come_from_the_grammar() {
    let messages = run(
        PROGRAM,
        &[
            ("textDocument/documentSymbol", 0, 0),
            // `fib` in `fib(n - 2)`, `n` in `n < 2`, `sum` in `p.sum()`.
            ("textDocument/definition", 7, 10),
            ("textDocument/definition", 6, 6),
            ("textDocument/definition", 11, 9),
            // `Point` in `Point(1, 2)`, `sum` again.
            ("textDocument/hover", 10, 9),
            ("textDocument/hover", 11, 9),
        ],
    );
    assert_eq!(
        outline(result(&messages, 1)),
        ["Point(init sum)", "fib", "p"]
    );
    assert_eq!(start(&result(&messages, 2).as_array()[0]), (5, 4));
    assert_eq!(start(&result(&messages, 3).as_array()[0]), (5, 8));
    assert_eq!(start(&result(&messages, 4).as_array()[0]), (2, 2));
    assert_eq!(
        hover(&messages, 5),
        "```lox\nclass Point\n```\nclass `Point`"
    );
    assert_eq!(hover(&messages, 6), "```lox\nsum()\n```\nmethod `sum`");
}

#[test]
fn compile_errors_are_diagnostics_at_their_token() {
    let messages = run("var a = 1;\nprint a +;\n", &[]);
    let diags = messages
        .iter()
        .find(|m| m.get("method").as_str() == Some("textDocument/publishDiagnostics"))
        .unwrap()
        .get("params")
        .get("diagnostics");
    let d = &diags.as_array()[0];
    assert_eq!(
        d.get("message").as_str(),
        Some("[line 2] Error at ';': Expect expression.")
    );
    assert_eq!(start(d), (1, 9));
}
