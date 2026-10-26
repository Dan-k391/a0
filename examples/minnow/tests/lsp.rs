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

#[test]
fn programs_of_several_files_are_analyzed_together() {
    let dir = std::env::temp_dir().join(format!("minnow-lsp-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("util.mn"),
        "pub fn twice(x: int) -> int {\n  return x * 2;\n}\n\npub fn broken() -> int {\n  return \"no\";\n}\n",
    )
    .unwrap();
    let main = "import util;\n\nfn main() {\n  print(util.twice(21));\n}\n";
    let uri = a0_driver::lsp::path_to_uri(&dir.join("main.mn"));
    let minnow = Path::new(env!("CARGO_BIN_EXE_minnow"));
    let messages = session(minnow, &uri, main, &[("textDocument/definition", 3, 14)]).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    // The imported file's error is published for that file, and the main file has none.
    let published: Vec<(&str, usize)> = messages
        .iter()
        .filter(|m| m.get("method").as_str() == Some("textDocument/publishDiagnostics"))
        .map(|m| {
            let p = m.get("params");
            (
                p.get("uri").as_str().unwrap(),
                p.get("diagnostics").as_array().len(),
            )
        })
        .collect();
    assert!(published.contains(&(uri.as_str(), 0)), "{published:?}");
    assert!(
        published
            .iter()
            .any(|(u, n)| u.ends_with("/util.mn") && *n == 1),
        "{published:?}"
    );
    // `util.twice` goes to the function in util.mn.
    let target = &result(&messages, 1).as_array()[0];
    assert!(
        target.get("uri").as_str().unwrap().ends_with("/util.mn"),
        "{target}"
    );
    assert_eq!(start(target), (0, 7));
}

#[test]
fn the_standard_library_is_known_to_the_server() {
    let main = "import std.str as s;\n\nfn main() {\n  print(s.find(\"abc\", \"b\"));\n  print(s.nope());\n}\n";
    let minnow = Path::new(env!("CARGO_BIN_EXE_minnow"));
    let dir = std::env::temp_dir().join(format!("minnow-lsp-std-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let uri = a0_driver::lsp::path_to_uri(&dir.join("main.mn"));
    let requests = [
        ("textDocument/hover", 3, 11),
        ("textDocument/definition", 3, 11),
    ];
    let messages = session(minnow, &uri, main, &requests).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let diagnostics: Vec<String> = messages
        .iter()
        .filter(|m| m.get("method").as_str() == Some("textDocument/publishDiagnostics"))
        .flat_map(|m| m.get("params").get("diagnostics").as_array().to_vec())
        .map(|d| d.get("message").as_str().unwrap().to_string())
        .collect();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert!(diagnostics[0].starts_with("module `std.str` has no public `nope`"));
    let hover = result(&messages, 1).get("contents").get("value");
    assert!(
        hover
            .as_str()
            .unwrap()
            .contains("fn std.str.find(s: string, part: string) -> int"),
        "{hover:?}"
    );
    // The library's source is written where the editor can open it.
    let target = &result(&messages, 2).as_array()[0];
    let file = target.get("uri").as_str().unwrap();
    assert!(file.ends_with("/std/str.hir"), "{target}");
    let line = start(target).0 as usize;
    let text = std::fs::read_to_string(a0_driver::lsp::uri_to_path(file).unwrap()).unwrap();
    assert!(
        text.lines().nth(line).unwrap().starts_with("fn find("),
        "{target}"
    );
}

const UTIL: &str = "pub struct Point { x: int, y: int }

impl Point {
  fn sum(self) -> int { return self.x + self.y; }
}

pub fn twice(x: int) -> int {
  return x * 2;
}
";

const MAIN: &str = "import util;

fn main() {
  let p = new util.Point { x: 1, y: 2 };
  let total = util.twice(p.sum());
  print(total + util.twice(3));
  print(p.x);
}
";

fn position(uri: &str, line: u32, character: u32) -> Json {
    Json::object([
        ("textDocument", Json::object([("uri", uri.into())])),
        (
            "position",
            Json::object([("line", line.into()), ("character", character.into())]),
        ),
    ])
}

/// The labels of a completion result.
fn labels(result: &Json) -> Vec<String> {
    result
        .get("items")
        .as_array()
        .iter()
        .map(|i| i.get("label").as_str().unwrap().to_string())
        .collect()
}

/// `(file name, line, character)` of each location.
fn places(result: &Json) -> Vec<(String, u32, u32)> {
    let mut out: Vec<(String, u32, u32)> = result
        .as_array()
        .iter()
        .map(|l| {
            let uri = l.get("uri").as_str().unwrap();
            let name = uri.rsplit('/').next().unwrap().to_string();
            let (line, character) = start(l);
            (name, line, character)
        })
        .collect();
    out.sort();
    out
}

#[test]
fn completion_references_and_rename_work_across_files() {
    let dir = std::env::temp_dir().join(format!("minnow-lsp-nav-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("util.mn"), UTIL).unwrap();
    let main = a0_driver::lsp::path_to_uri(&dir.join("main.mn"));
    let minnow = Path::new(env!("CARGO_BIN_EXE_minnow"));
    let with = |p: Json, extra: (&str, Json)| match p {
        Json::Object(mut m) => {
            m.push((extra.0.to_string(), extra.1));
            Json::Object(m)
        }
        other => other,
    };
    let requests = [
        // Names in scope at the start of line 6, and the language's keywords.
        ("textDocument/completion", position(&main, 6, 2)),
        // After `p.`: the struct's fields and its method.
        ("textDocument/completion", position(&main, 6, 10)),
        // After `util.`: the module's items.
        ("textDocument/completion", position(&main, 5, 21)),
        // `twice`, used twice in main.mn and declared in util.mn.
        ("textDocument/references", position(&main, 4, 20)),
        // `total`, a local.
        ("textDocument/references", position(&main, 5, 9)),
        ("textDocument/prepareRename", position(&main, 4, 20)),
        (
            "textDocument/rename",
            with(position(&main, 4, 20), ("newName", "double".into())),
        ),
        (
            "textDocument/rename",
            with(position(&main, 4, 20), ("newName", "fn".into())),
        ),
    ];
    let messages =
        a0_driver::lsp::session_requests(minnow, &["lsp"], &[(&main, MAIN)], &requests).unwrap();
    std::fs::remove_dir_all(&dir).ok();

    let names = labels(result(&messages, 1));
    for wanted in ["p", "total", "main", "util", "let", "while"] {
        assert!(names.contains(&wanted.to_string()), "{wanted} in {names:?}");
    }
    assert_eq!(labels(result(&messages, 2)), ["sum", "x", "y"]);
    let items = labels(result(&messages, 3));
    assert!(
        items.contains(&"twice".to_string()) && items.contains(&"Point".to_string()),
        "{items:?}"
    );

    assert_eq!(
        places(result(&messages, 4)),
        [
            ("main.mn".to_string(), 4, 19),
            ("main.mn".to_string(), 5, 21),
            ("util.mn".to_string(), 6, 7),
        ]
    );
    assert_eq!(
        places(result(&messages, 5)),
        [("main.mn".to_string(), 4, 6), ("main.mn".to_string(), 5, 8)]
    );
    assert_eq!(
        result(&messages, 6).get("placeholder").as_str(),
        Some("twice")
    );
    // The rename edits both files.
    let changes = result(&messages, 7).get("changes");
    let Json::Object(files) = changes else {
        panic!("{changes}")
    };
    let mut edited: Vec<(String, usize)> = files
        .iter()
        .map(|(uri, edits)| {
            let name = uri.rsplit('/').next().unwrap().to_string();
            (name, edits.as_array().len())
        })
        .collect();
    edited.sort();
    assert_eq!(
        edited,
        [("main.mn".to_string(), 2), ("util.mn".to_string(), 1)]
    );
    // A keyword is not a name.
    let refused = messages
        .iter()
        .find(|m| m.get("id").as_u32() == Some(8))
        .unwrap();
    assert!(!refused.get("error").is_null(), "{refused}");
}

#[test]
fn edits_are_analyzed_once_they_pause() {
    let uri = "file:///edits.mn";
    let minnow = Path::new(env!("CARGO_BIN_EXE_minnow"));
    // Fifty edits in a row, then a request: the program is analyzed for the request, not fifty
    // times on the way.
    let mut requests: Vec<(&str, Json)> = Vec::new();
    for i in 0..50 {
        let text = format!("fn main() {{\n  let x = {i};\n  print(x);\n}}\n");
        requests.push((
            "textDocument/didChange",
            Json::object([
                ("textDocument", Json::object([("uri", uri.into())])),
                (
                    "contentChanges",
                    vec![Json::object([("text", text.into())])].into(),
                ),
            ]),
        ));
    }
    requests.push(("textDocument/hover", position(uri, 1, 6)));
    let messages =
        a0_driver::lsp::session_requests(minnow, &["lsp"], &[(uri, "fn main() {}\n")], &requests)
            .unwrap();
    let analyses = messages
        .iter()
        .filter(|m| m.get("method").as_str() == Some("textDocument/publishDiagnostics"))
        .count();
    assert!(analyses <= 2, "{analyses} analyses");
    let hover = result(&messages, 1).get("contents").get("value");
    assert!(hover.as_str().unwrap().contains("let x = 49"), "{hover}");
}
