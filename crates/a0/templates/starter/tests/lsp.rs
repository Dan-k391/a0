//! `{{name}} lsp`, the language server: the outline of a small program and a diagnostic.

use a0_driver::json::Json;
use a0_driver::lsp::session;

const PROGRAM: &str = "fn twice(x: int) -> int {\n  return x * 2;\n}\n\nfn main() {\n  print(twice(21));\n  print(thrice(1));\n}\n";

#[test]
fn the_language_server_knows_the_program() {
    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_{{name}}"));
    let messages = session(
        exe,
        "file:///demo.{{ext}}",
        PROGRAM,
        &[("textDocument/documentSymbol", 0, 0)],
    )
    .unwrap();
    let outline = messages
        .iter()
        .find(|m| m.get("id").as_u32() == Some(1))
        .map(|m| m.get("result"))
        .unwrap();
    let names: Vec<&str> = outline
        .as_array()
        .iter()
        .map(|s| s.get("name").as_str().unwrap())
        .collect();
    assert_eq!(names, ["twice", "main"]);
    let diagnostics: Vec<&Json> = messages
        .iter()
        .filter(|m| m.get("method").as_str() == Some("textDocument/publishDiagnostics"))
        .flat_map(|m| m.get("params").get("diagnostics").as_array())
        .collect();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    let message = diagnostics[0].get("message").as_str().unwrap();
    assert!(message.contains("thrice"), "{message}");
}
