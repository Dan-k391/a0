//! `kaleidoscope lsp`: the language server for a language that compiles to the core IR.

use std::path::Path;

use a0_driver::json::Json;
use a0_driver::lsp::session;

const PROGRAM: &str = "\
extern printd(x);
# 'var' initializers see the outer binding.
def shadow(a)
  var a = a + 1 in
    var a = a * 2, b = a in b;
printd(shadow(1));
";

fn result(messages: &[Json], id: u32) -> &Json {
    messages
        .iter()
        .find(|m| m.get("id").as_u32() == Some(id))
        .map(|m| m.get("result"))
        .unwrap_or_else(|| panic!("no response {id}"))
}

fn start(location: &Json) -> (u32, u32) {
    let s = location.get("range").get("start");
    (
        s.get("line").as_u32().unwrap(),
        s.get("character").as_u32().unwrap(),
    )
}

#[test]
fn names_resolve_through_nested_scopes() {
    let kaleidoscope = Path::new(env!("CARGO_BIN_EXE_kaleidoscope"));
    let messages = session(
        kaleidoscope,
        "file:///demo.k",
        PROGRAM,
        &[
            ("textDocument/documentSymbol", 0, 0),
            // The `a` read by each initializer, `a` in `b = a`, `b` in the body, `shadow`.
            ("textDocument/definition", 3, 10),
            ("textDocument/definition", 4, 12),
            ("textDocument/definition", 4, 23),
            ("textDocument/definition", 4, 28),
            ("textDocument/definition", 5, 8),
            ("textDocument/hover", 5, 1),
        ],
    )
    .unwrap();
    let names: Vec<&str> = result(&messages, 1)
        .as_array()
        .iter()
        .map(|s| s.get("name").as_str().unwrap())
        .collect();
    assert_eq!(names, ["printd", "shadow"]);
    assert_eq!(start(&result(&messages, 2).as_array()[0]), (2, 11));
    assert_eq!(start(&result(&messages, 3).as_array()[0]), (3, 6));
    assert_eq!(start(&result(&messages, 4).as_array()[0]), (4, 8));
    assert_eq!(start(&result(&messages, 5).as_array()[0]), (4, 19));
    assert_eq!(start(&result(&messages, 6).as_array()[0]), (2, 4));
    let hover = result(&messages, 7).get("contents").get("value");
    assert_eq!(
        hover.as_str(),
        Some("```kaleidoscope\nextern printd(x)\n```\nfunction `printd`")
    );
}
