//! `minnow lsp`: the language server, with types on hover from the HIR Minnow builds.

use std::path::Path;

use a0_driver::json::Json;
use a0_driver::lsp::session;

const PROGRAM: &str = "\
struct Point { x: int, y: int }
enum Shape { Circle(float), Square(float) }

fn area(s: Shape) -> float {
  return match s {
    Circle(r) => 3.14 * r * r,
    Square(w) => w * w,
  };
}

fn main() {
  let p = new Point { x: 1, y: 2 };
  let total = p.x + p.y;
  print(str(area(Shape::Circle(1.0))));
  print(str(total));
}
";

fn run(text: &str, requests: &[(&str, u32, u32)]) -> Vec<Json> {
    let minnow = Path::new(env!("CARGO_BIN_EXE_minnow"));
    session(minnow, "file:///demo.mn", text, requests).unwrap()
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
    let contents = result(messages, id).get("contents");
    contents.get("value").as_str().unwrap()
}

fn start(location: &Json) -> (u32, u32) {
    let s = location.get("range").get("start");
    (
        s.get("line").as_u32().unwrap(),
        s.get("character").as_u32().unwrap(),
    )
}

#[test]
fn hovers_show_inferred_types() {
    let messages = run(
        PROGRAM,
        &[
            ("textDocument/documentSymbol", 0, 0),
            // `total` (inferred), `y` in `p.y`, `r` (a pattern binding), `p`.
            ("textDocument/hover", 12, 7),
            ("textDocument/hover", 12, 22),
            ("textDocument/hover", 5, 11),
            ("textDocument/hover", 11, 6),
            // `Circle` in `Shape::Circle(1.0)` and `area` in its call.
            ("textDocument/definition", 13, 24),
            ("textDocument/definition", 13, 12),
        ],
    );
    assert_eq!(
        names(result(&messages, 1)),
        ["Point(x y)", "Shape(Circle Square)", "area", "main"]
    );
    assert_eq!(
        hover(&messages, 2),
        "```minnow\nlet total = p.x + p.y;\n```\nvariable `total`: `int`"
    );
    assert_eq!(
        hover(&messages, 3),
        "```minnow\ny: int\n```\nfield `y`: `int`"
    );
    assert_eq!(hover(&messages, 4), "variable `r`: `float`");
    assert_eq!(
        hover(&messages, 5),
        "```minnow\nlet p = new Point { x: 1, y: 2 };\n```\nvariable `p`: `Point`"
    );
    assert_eq!(start(&result(&messages, 6).as_array()[0]), (1, 13));
    assert_eq!(start(&result(&messages, 7).as_array()[0]), (3, 3));
}

#[test]
fn type_errors_are_diagnostics() {
    let messages = run("fn main() {\n  let x: int = \"no\";\n}\n", &[]);
    let diags = messages
        .iter()
        .find(|m| m.get("method").as_str() == Some("textDocument/publishDiagnostics"))
        .unwrap()
        .get("params")
        .get("diagnostics");
    let d = &diags.as_array()[0];
    assert!(
        d.get("message")
            .as_str()
            .unwrap()
            .contains("mismatched types"),
        "{d}"
    );
    assert_eq!(start(d).0, 1);
}

#[test]
fn misspelled_names_get_a_quick_fix() {
    let text = "fn main() {\n  let count = 1;\n  print(cuont);\n}\n";
    let messages = run(text, &[("textDocument/codeAction", 2, 9)]);
    let d = &messages
        .iter()
        .find(|m| m.get("method").as_str() == Some("textDocument/publishDiagnostics"))
        .unwrap()
        .get("params")
        .get("diagnostics")
        .as_array()[0];
    assert_eq!(d.get("code").as_str(), Some("E1001"), "{d}");
    let action = &result(&messages, 1).as_array()[0];
    assert_eq!(
        action.get("title").as_str(),
        Some("a variable with a similar name exists: `count`")
    );
    assert_eq!(action.get("kind").as_str(), Some("quickfix"));
    let edit = &action
        .get("edit")
        .get("changes")
        .get("file:///demo.mn")
        .as_array()[0];
    assert_eq!(edit.get("newText").as_str(), Some("count"));
    assert_eq!(start(edit), (2, 8));
}

#[test]
fn related_locations_are_related_information() {
    let text = "fn f(a: int) {}\nfn main() {\n  f(\"no\");\n}\n";
    let messages = run(text, &[]);
    let d = &messages
        .iter()
        .find(|m| m.get("method").as_str() == Some("textDocument/publishDiagnostics"))
        .unwrap()
        .get("params")
        .get("diagnostics")
        .as_array()[0];
    let related = &d.get("relatedInformation").as_array()[0];
    assert_eq!(
        related.get("message").as_str(),
        Some("parameter declared here")
    );
    assert_eq!(start(related.get("location")), (0, 5));
    assert!(
        d.get("message")
            .as_str()
            .unwrap()
            .contains("expected `int`, found `string`"),
        "{d}"
    );
}

#[test]
fn warnings_are_diagnostics_too() {
    let messages = run("fn main() {\n  let x = 1;\n}\n", &[]);
    let d = &messages
        .iter()
        .find(|m| m.get("method").as_str() == Some("textDocument/publishDiagnostics"))
        .unwrap()
        .get("params")
        .get("diagnostics")
        .as_array()[0];
    assert_eq!(d.get("severity").as_u32(), Some(2), "{d}");
    assert_eq!(d.get("code").as_str(), Some("W0001"));
    assert_eq!(start(d), (1, 6));
}
