//! `tin lsp`: the language server, with types on hover from the HIR Tin builds.

use std::path::Path;

use a0_driver::json::Json;
use a0_driver::lsp::session;

const PROGRAM: &str = "\
struct Vec2 { x: f64, y: f64 }
extern fn sqrt(x: f64) -> f64;

fn length(v: *Vec2) -> f64 {
  return sqrt(v->x * v->x + v->y * v->y);
}

fn main() -> i32 {
  var p = (Vec2){ .x = 3.0, .y = 4.0 };
  var size = length(&p);
  println(size);
  return 0;
}
";

fn run(text: &str, requests: &[(&str, u32, u32)]) -> Vec<Json> {
    let tin = Path::new(env!("CARGO_BIN_EXE_tin"));
    session(tin, "file:///demo.tin", text, requests).unwrap()
}

fn result(messages: &[Json], id: u32) -> &Json {
    messages
        .iter()
        .find(|m| m.get("id").as_u32() == Some(id))
        .map(|m| m.get("result"))
        .unwrap_or_else(|| panic!("no response {id}"))
}

fn names(symbols: &Json) -> Vec<String> {
    symbols
        .as_array()
        .iter()
        .map(|s| {
            let children = names(s.get("children"));
            let name = s.get("name").as_str().unwrap();
            if children.is_empty() {
                name.to_string()
            } else {
                format!("{name}({})", children.join(" "))
            }
        })
        .collect()
}

fn hover(messages: &[Json], id: u32) -> &str {
    result(messages, id)
        .get("contents")
        .get("value")
        .as_str()
        .unwrap()
}

fn start(location: &Json) -> (u32, u32) {
    let s = location.get("range").get("start");
    (
        s.get("line").as_u32().unwrap(),
        s.get("character").as_u32().unwrap(),
    )
}

fn diagnostics(messages: &[Json]) -> Vec<Json> {
    messages
        .iter()
        .find(|m| m.get("method").as_str() == Some("textDocument/publishDiagnostics"))
        .unwrap()
        .get("params")
        .get("diagnostics")
        .as_array()
        .to_vec()
}

#[test]
fn outline_hovers_and_definitions() {
    let messages = run(
        PROGRAM,
        &[
            ("textDocument/documentSymbol", 0, 0),
            // `size` (inferred), `v` (a pointer parameter), `x` in `v->x`.
            ("textDocument/hover", 9, 7),
            ("textDocument/hover", 3, 10),
            ("textDocument/hover", 4, 17),
            // `length` in its call, and `sqrt`.
            ("textDocument/definition", 9, 13),
            ("textDocument/definition", 4, 10),
        ],
    );
    assert_eq!(
        names(result(&messages, 1)),
        ["Vec2(x y)", "sqrt", "length", "main"]
    );
    assert_eq!(
        hover(&messages, 2),
        "```tin\nvar size = length(&p);\n```\nvariable `size`: `f64`"
    );
    assert!(
        hover(&messages, 3).ends_with("`*Vec2`"),
        "{}",
        hover(&messages, 3)
    );
    assert_eq!(hover(&messages, 4), "```tin\nx: f64\n```\nfield `x`: `f64`");
    assert_eq!(start(&result(&messages, 5).as_array()[0]), (3, 3));
    assert_eq!(start(&result(&messages, 6).as_array()[0]), (1, 10));
}

#[test]
fn type_errors_are_diagnostics() {
    let messages = run(
        "fn main() -> i32 {\n  var p: *i32 = null;\n  var q: *u8 = p;\n  return 0;\n}\n",
        &[],
    );
    let d = &diagnostics(&messages)[0];
    assert!(
        d.get("message")
            .as_str()
            .unwrap()
            .contains("expected `*u8`, found `*i32`"),
        "{d}"
    );
    assert_eq!(start(d), (2, 15));
}

#[test]
fn misspelled_names_get_a_quick_fix() {
    let text = "fn main() -> i32 {\n  var count = 1;\n  println(cuont);\n  return 0;\n}\n";
    let messages = run(text, &[("textDocument/codeAction", 2, 11)]);
    let d = &diagnostics(&messages)[0];
    assert_eq!(d.get("code").as_str(), Some("E1001"), "{d}");
    let action = &result(&messages, 1).as_array()[0];
    assert_eq!(
        action.get("title").as_str(),
        Some("a variable with a similar name exists: `count`")
    );
    let edit = &action
        .get("edit")
        .get("changes")
        .get("file:///demo.tin")
        .as_array()[0];
    assert_eq!(edit.get("newText").as_str(), Some("count"));
    assert_eq!(start(edit), (2, 10));
}
