//! A Language Server Protocol server for any language built on A0 (`lang lsp`).
//!
//! Everything comes from what a language already gives the driver: diagnostics from its compiler;
//! highlighting, the outline, go-to-definition and the declarations shown on hover from its grammar's
//! `@def` and `@scope` annotations ([`SyntaxIndex`]); and types on hover from the HIR its compiler
//! builds, when it builds one with spans. Diagnostics carry their codes and related locations, and
//! their suggested fixes are quick fixes (code actions). Completion, find-references and rename come
//! from the indexes of the program's files and the imports between them (`navigate.rs`). The server
//! speaks JSON-RPC on stdin and stdout and keeps whole documents (full sync). An edit only reindexes
//! the document; the program is analyzed again once edits pause, or when a request needs it, so
//! that typing in a large program stays responsive.

use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::time::Duration;

mod navigate;

use a0_diag::{Diagnostic, Fix, Severity, SourceId, SourceMap, Span};
use a0_hir::{HirModule, Item};
use a0_syntax::{Grammar, SymbolKind, SyntaxIndex, TokenClass};

use crate::json::Json;
use crate::{CompileError, Language, Program, program};

/// Semantic token types, in the order of the legend sent to the client.
const TOKEN_TYPES: [&str; 18] = [
    "keyword",
    "operator",
    "number",
    "string",
    "comment",
    "function",
    "method",
    "class",
    "struct",
    "enum",
    "enumMember",
    "property",
    "variable",
    "parameter",
    "typeParameter",
    "type",
    "namespace",
    "interface",
];

fn token_type(name: &str) -> u32 {
    TOKEN_TYPES.iter().position(|&t| t == name).unwrap() as u32
}

fn symbol_token_type(kind: SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Function => "function",
        SymbolKind::Method => "method",
        SymbolKind::Class => "class",
        SymbolKind::Struct => "struct",
        SymbolKind::Enum => "enum",
        SymbolKind::Interface => "interface",
        SymbolKind::Variant => "enumMember",
        SymbolKind::Field => "property",
        SymbolKind::Variable | SymbolKind::Constant => "variable",
        SymbolKind::Parameter => "parameter",
        SymbolKind::TypeParameter => "typeParameter",
        SymbolKind::Type => "type",
        SymbolKind::Module => "namespace",
    }
}

/// The protocol's `SymbolKind` numbers.
fn lsp_symbol_kind(kind: SymbolKind) -> u32 {
    match kind {
        SymbolKind::Module => 2,
        SymbolKind::Class | SymbolKind::Type => 5,
        SymbolKind::Method => 6,
        SymbolKind::Field => 8,
        SymbolKind::Enum => 10,
        SymbolKind::Interface => 11,
        SymbolKind::Function => 12,
        SymbolKind::Variable | SymbolKind::Parameter => 13,
        SymbolKind::Constant => 14,
        SymbolKind::Variant => 22,
        SymbolKind::Struct => 23,
        SymbolKind::TypeParameter => 26,
    }
}

/// Byte offsets of line starts, for converting to and from the protocol's positions (lines and
/// UTF-16 code units).
struct Lines {
    starts: Vec<u32>,
}

impl Lines {
    fn new(text: &str) -> Lines {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(i, _)| i as u32 + 1));
        Lines { starts }
    }

    fn position(&self, text: &str, offset: u32) -> (u32, u32) {
        let offset = offset.min(text.len() as u32);
        let line = self.starts.partition_point(|&s| s <= offset) - 1;
        let start = self.starts[line] as usize;
        let character = text[start..offset as usize]
            .chars()
            .map(char::len_utf16)
            .sum::<usize>();
        (line as u32, character as u32)
    }

    fn offset(&self, text: &str, line: u32, character: u32) -> u32 {
        let Some(&start) = self.starts.get(line as usize) else {
            return text.len() as u32;
        };
        let mut units = 0;
        for (i, c) in text[start as usize..].char_indices() {
            if units >= character as usize || c == '\n' {
                return start + i as u32;
            }
            units += c.len_utf16();
        }
        text.len() as u32
    }

    fn range(&self, text: &str, start: u32, end: u32) -> Json {
        let (sl, sc) = self.position(text, start);
        let (el, ec) = self.position(text, end);
        Json::object([
            (
                "start",
                Json::object([("line", sl.into()), ("character", sc.into())]),
            ),
            (
                "end",
                Json::object([("line", el.into()), ("character", ec.into())]),
            ),
        ])
    }
}

/// An open document and what is known about it.
struct Document {
    text: String,
    lines: Lines,
    index: Option<SyntaxIndex>,
    hir: Option<HirModule>,
    source: SourceId,
    /// The suggested fixes of its diagnostics: the diagnostic's range (byte offsets), the
    /// diagnostic as published, and the fix.
    fixes: Vec<(u32, u32, Json, Fix)>,
    /// The texts of the program's files and their URIs (the document's own is `source`).
    sources: SourceMap,
    files: Vec<(SourceId, String)>,
    /// Every file of the program as last analyzed, indexed, with its imports.
    program: Vec<FileInfo>,
    /// Whether all of the program's files loaded at the last analysis (for a language with
    /// modules; they do not when one has syntax errors).
    loaded: bool,
}

/// A file of a document's program, for navigating between files.
struct FileInfo {
    source: SourceId,
    uri: String,
    text: String,
    lines: Lines,
    index: SyntaxIndex,
    imports: Vec<FileImport>,
}

/// An import of a file: the name it uses for the module, the module's file (an index into the
/// program's files), and the names it imports unqualified.
type FileImport = (String, usize, Vec<(String, Span)>);

struct Server<'a> {
    lang: &'a Language,
    grammar: Option<Grammar>,
    docs: HashMap<String, Document>,
    /// Per document: the other files its program's last analysis published diagnostics for.
    reported: HashMap<String, Vec<String>>,
    /// Documents edited since they were last analyzed, in the order they were.
    dirty: Vec<String>,
    /// Syntax trees and indexes of files kept between analyses, by path: unchanged files are not
    /// parsed or indexed again (D82).
    trees: program::TreeCache,
    indexes: HashMap<PathBuf, (String, SyntaxIndex)>,
    out: &'a mut dyn Write,
}

/// How long edits must pause before the program is analyzed again.
const QUIET: Duration = Duration::from_millis(200);

/// Serves the protocol until `exit` or the end of the input. Returns the process exit status: 0
/// after a `shutdown` request, 1 otherwise (as the protocol asks).
pub(crate) fn serve(
    lang: &Language,
    mut input: Box<dyn BufRead + Send>,
    output: &mut dyn Write,
) -> u8 {
    let grammar = lang
        .grammar
        .and_then(|text| match Grammar::from_str(text, SourceId(u32::MAX)) {
            Ok(g) => Some(g),
            Err(d) => {
                eprintln!("{}: the grammar does not load:\n{}", lang.name, d.summary());
                None
            }
        });
    let mut server = Server {
        lang,
        grammar,
        docs: HashMap::new(),
        reported: HashMap::new(),
        dirty: Vec::new(),
        trees: program::TreeCache::default(),
        indexes: HashMap::new(),
        out: output,
    };
    // Messages are read on a thread of their own, so that the server can tell when edits pause.
    let (to_server, messages) = channel();
    std::thread::spawn(move || {
        loop {
            let message = match read_message(&mut input) {
                Ok(Some(m)) => Some(m),
                Ok(None) => None,
                Err(e) => {
                    eprintln!("{e}");
                    None
                }
            };
            let end = message.is_none();
            if to_server.send(message).is_err() || end {
                break;
            }
        }
    });
    let mut shut_down = false;
    loop {
        let next = if server.dirty.is_empty() {
            messages.recv().ok().flatten()
        } else {
            match messages.recv_timeout(QUIET) {
                Ok(m) => m,
                Err(RecvTimeoutError::Timeout) => {
                    server.analyze_dirty();
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => None,
            }
        };
        let Some(message) = next else {
            server.analyze_dirty();
            return if shut_down { 0 } else { 1 };
        };
        let message = match Json::parse(&message) {
            Ok(m) => m,
            Err(e) => {
                server.error(&Json::Null, -32700, &format!("invalid JSON: {e}"));
                continue;
            }
        };
        let id = message.get("id").clone();
        let params = message.get("params");
        match message.get("method").as_str().unwrap_or("") {
            "initialize" => {
                let result = server.capabilities();
                server.respond(&id, result);
            }
            "shutdown" => {
                server.analyze_dirty();
                shut_down = true;
                server.respond(&id, Json::Null);
            }
            "exit" => return if shut_down { 0 } else { 1 },
            "textDocument/didOpen" => {
                let doc = params.get("textDocument");
                if let (Some(uri), Some(text)) = (doc.get("uri").as_str(), doc.get("text").as_str())
                {
                    server.edit(uri, text.to_string());
                }
            }
            "textDocument/didChange" => {
                let uri = params.get("textDocument").get("uri").as_str();
                let change = params.get("contentChanges").as_array().last();
                if let (Some(uri), Some(text)) = (uri, change.and_then(|c| c.get("text").as_str()))
                {
                    server.edit(uri, text.to_string());
                }
            }
            "textDocument/didClose" => {
                if let Some(uri) = params.get("textDocument").get("uri").as_str() {
                    server.docs.remove(uri);
                    server.dirty.retain(|d| d != uri);
                    server.publish(uri, Vec::new());
                }
            }
            "textDocument/semanticTokens/full" => {
                let result = server
                    .doc(params)
                    .map_or(Json::Null, |d| Json::object([("data", semantic_tokens(d))]));
                server.respond(&id, result);
            }
            "textDocument/documentSymbol" => {
                let result = server.doc(params).map_or(Json::Null, document_symbols);
                server.respond(&id, result);
            }
            "textDocument/definition" => {
                server.analyze_for(params);
                let result = server.definition(params);
                server.respond(&id, result);
            }
            "textDocument/hover" => {
                server.analyze_for(params);
                let result = server.hover(params);
                server.respond(&id, result);
            }
            "textDocument/codeAction" => {
                server.analyze_for(params);
                let result = server.code_actions(params);
                server.respond(&id, result);
            }
            "textDocument/completion" => {
                // Completion does not wait for the analysis of an edit (`p.` does not even parse):
                // the last analysis knows the types and the imports well enough.
                if server.doc(params).is_some_and(|d| d.program.is_empty()) {
                    server.analyze_for(params);
                }
                let result = server.completion(params);
                server.respond(&id, result);
            }
            "textDocument/references" => {
                server.analyze_for(params);
                let result = server.references(params);
                server.respond(&id, result);
            }
            "textDocument/prepareRename" => {
                server.analyze_for(params);
                match server.prepare_rename(params) {
                    Ok(result) => server.respond(&id, result),
                    Err(message) => server.error(&id, -32803, &message),
                }
            }
            "textDocument/rename" => {
                server.analyze_for(params);
                match server.rename(params) {
                    Ok(result) => server.respond(&id, result),
                    Err(message) => server.error(&id, -32803, &message),
                }
            }
            _ if !id.is_null() => server.error(&id, -32601, "method not supported"),
            _ => {}
        }
    }
}

/// A message framed for the wire (`Content-Length` header and body), for clients and tests.
pub fn frame(message: &Json) -> Vec<u8> {
    let body = message.to_string();
    format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
}

/// The messages in a stream of framed messages (a server's whole output, say). Stops at the first
/// one that is not valid.
pub fn messages(mut stream: &[u8]) -> Vec<Json> {
    let mut out = Vec::new();
    while let Ok(Some(text)) = read_message(&mut stream) {
        match Json::parse(&text) {
            Ok(m) => out.push(m),
            Err(_) => break,
        }
    }
    out
}

/// Runs a whole session against a language server (`server lsp`), for a language's tests: opens
/// `text` as `uri`, sends each request (method, 0-based line and UTF-16 character; responses get ids
/// 1, 2, ... in order), shuts down and exits. Returns everything the server said.
pub fn session(
    server: &std::path::Path,
    uri: &str,
    text: &str,
    requests: &[(&str, u32, u32)],
) -> io::Result<Vec<Json>> {
    session_with_args(server, &["lsp"], uri, text, requests)
}

/// Runs a session of any requests against a language server (`server <args>`), for tests: opens
/// each `(uri, text)` of `open`, then sends each `(method, params)`: requests get ids 1, 2, ... in
/// order, and `textDocument/did...` methods are sent as notifications. Then shuts down and exits.
/// Returns everything the server said.
pub fn session_requests(
    server: &std::path::Path,
    args: &[&str],
    open: &[(&str, &str)],
    requests: &[(&str, Json)],
) -> io::Result<Vec<Json>> {
    let message = |id: Option<u32>, method: &str, params: Json| {
        let mut m = vec![("jsonrpc".to_string(), "2.0".into())];
        if let Some(id) = id {
            m.push(("id".to_string(), id.into()));
        }
        m.push(("method".to_string(), method.into()));
        m.push(("params".to_string(), params));
        frame(&Json::Object(m))
    };
    let mut input = vec![message(Some(0), "initialize", Json::object([]))];
    for &(uri, text) in open {
        input.push(message(
            None,
            "textDocument/didOpen",
            Json::object([(
                "textDocument",
                Json::object([("uri", uri.into()), ("text", text.into())]),
            )]),
        ));
    }
    let mut id = 0;
    for (method, params) in requests {
        if method.starts_with("textDocument/did") {
            input.push(message(None, method, params.clone()));
        } else {
            id += 1;
            input.push(message(Some(id), method, params.clone()));
        }
    }
    input.push(message(Some(u32::MAX), "shutdown", Json::Null));
    input.push(message(None, "exit", Json::Null));
    let mut child = std::process::Command::new(server)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(&input.concat())?;
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        return Err(io::Error::other(format!(
            "the server exited with {}",
            out.status
        )));
    }
    Ok(messages(&out.stdout))
}

/// [`session`] with a server started by other arguments (`a0 lang calc.a0g lsp`).
pub fn session_with_args(
    server: &std::path::Path,
    args: &[&str],
    uri: &str,
    text: &str,
    requests: &[(&str, u32, u32)],
) -> io::Result<Vec<Json>> {
    let message = |id: Option<u32>, method: &str, params: Json| {
        let mut m = vec![("jsonrpc".to_string(), "2.0".into())];
        if let Some(id) = id {
            m.push(("id".to_string(), id.into()));
        }
        m.push(("method".to_string(), method.into()));
        m.push(("params".to_string(), params));
        frame(&Json::Object(m))
    };
    let doc = || Json::object([("uri", uri.into())]);
    let mut input = vec![
        message(Some(0), "initialize", Json::object([])),
        message(
            None,
            "textDocument/didOpen",
            Json::object([(
                "textDocument",
                Json::object([("uri", uri.into()), ("text", text.into())]),
            )]),
        ),
    ];
    for (i, &(method, line, character)) in requests.iter().enumerate() {
        let position = Json::object([("line", line.into()), ("character", character.into())]);
        // Code actions are asked for a range; the position stands for an empty one.
        let params = if method == "textDocument/codeAction" {
            let range = Json::object([("start", position.clone()), ("end", position)]);
            Json::object([
                ("textDocument", doc()),
                ("range", range),
                (
                    "context",
                    Json::object([("diagnostics", Json::Array(Vec::new()))]),
                ),
            ])
        } else {
            Json::object([("textDocument", doc()), ("position", position)])
        };
        input.push(message(Some(i as u32 + 1), method, params));
    }
    input.push(message(Some(u32::MAX), "shutdown", Json::Null));
    input.push(message(None, "exit", Json::Null));
    let mut child = std::process::Command::new(server)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(&input.concat())?;
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        return Err(io::Error::other(format!(
            "the server exited with {}",
            out.status
        )));
    }
    Ok(messages(&out.stdout))
}

/// Reads one `Content-Length`-framed message; `None` at the end of the input.
pub(crate) fn read_message(input: &mut dyn BufRead) -> io::Result<Option<String>> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse::<usize>().ok();
        }
    }
    let Some(length) = length else {
        return Err(io::Error::other("a message without Content-Length"));
    };
    let mut body = vec![0; length];
    input.read_exact(&mut body)?;
    String::from_utf8(body)
        .map(Some)
        .map_err(|_| io::Error::other("a message that is not UTF-8"))
}

impl Server<'_> {
    fn send(&mut self, message: Json) {
        let body = message.to_string();
        let _ = write!(self.out, "Content-Length: {}\r\n\r\n{body}", body.len());
        let _ = self.out.flush();
    }

    fn respond(&mut self, id: &Json, result: Json) {
        self.send(Json::object([
            ("jsonrpc", "2.0".into()),
            ("id", id.clone()),
            ("result", result),
        ]));
    }

    fn error(&mut self, id: &Json, code: i32, message: &str) {
        self.send(Json::object([
            ("jsonrpc", "2.0".into()),
            ("id", id.clone()),
            (
                "error",
                Json::object([
                    ("code", Json::Number(code as f64)),
                    ("message", message.into()),
                ]),
            ),
        ]));
    }

    fn capabilities(&self) -> Json {
        let mut caps = vec![
            (
                "textDocumentSync".to_string(),
                Json::object([("openClose", true.into()), ("change", 1u32.into())]),
            ),
            ("positionEncoding".to_string(), "utf-16".into()),
            ("codeActionProvider".to_string(), true.into()),
        ];
        if self.grammar.is_some() {
            let types = TOKEN_TYPES.iter().map(|&t| t.into()).collect::<Vec<Json>>();
            caps.extend([
                (
                    "semanticTokensProvider".to_string(),
                    Json::object([
                        (
                            "legend",
                            Json::object([
                                ("tokenTypes", types.into()),
                                ("tokenModifiers", vec!["declaration".into()].into()),
                            ]),
                        ),
                        ("full", true.into()),
                    ]),
                ),
                ("documentSymbolProvider".to_string(), true.into()),
                ("definitionProvider".to_string(), true.into()),
                ("hoverProvider".to_string(), true.into()),
                (
                    "completionProvider".to_string(),
                    Json::object([("triggerCharacters", vec![".".into()].into())]),
                ),
                ("referencesProvider".to_string(), true.into()),
                (
                    "renameProvider".to_string(),
                    Json::object([("prepareProvider", true.into())]),
                ),
            ]);
        }
        Json::object([
            ("capabilities", Json::Object(caps)),
            (
                "serverInfo",
                Json::object([
                    ("name", format!("{} (A0)", self.lang.name).into()),
                    ("version", env!("CARGO_PKG_VERSION").into()),
                ]),
            ),
        ])
    }

    fn doc(&self, params: &Json) -> Option<&Document> {
        let uri = params.get("textDocument").get("uri").as_str()?;
        self.docs.get(uri)
    }

    /// The document and byte offset of a `TextDocumentPositionParams`.
    fn position(&self, params: &Json) -> Option<(&Document, u32)> {
        let doc = self.doc(params)?;
        let pos = params.get("position");
        let offset = doc.lines.offset(
            &doc.text,
            pos.get("line").as_u32()?,
            pos.get("character").as_u32()?,
        );
        Some((doc, offset))
    }

    /// Analyzes a new version of a document and publishes its diagnostics. For a language with
    /// modules the document is the root of a program: the files it imports are read from open
    /// documents or from disk, and their diagnostics are published too.
    /// A new version of a document: reindexed now, analyzed later (`analyze_dirty`).
    fn edit(&mut self, uri: &str, text: String) {
        let lines = Lines::new(&text);
        let index = self.grammar.as_ref().map(|g| {
            let (tree, _) = g.parse(&text, SourceId(0));
            SyntaxIndex::new(g, &tree)
        });
        match self.docs.get_mut(uri) {
            Some(doc) => {
                doc.text = text;
                doc.lines = lines;
                doc.index = index;
            }
            None => {
                let mut sources = SourceMap::new();
                let source = sources.add(uri, &text);
                self.docs.insert(
                    uri.to_string(),
                    Document {
                        text,
                        lines,
                        index,
                        hir: None,
                        source,
                        fixes: Vec::new(),
                        sources,
                        files: Vec::new(),
                        program: Vec::new(),
                        loaded: false,
                    },
                );
            }
        }
        if !self.dirty.iter().any(|d| d == uri) {
            self.dirty.push(uri.to_string());
        }
    }

    /// Analyzes every edited document.
    fn analyze_dirty(&mut self) {
        for uri in std::mem::take(&mut self.dirty) {
            if let Some(text) = self.docs.get(&uri).map(|d| d.text.clone()) {
                a0_diag::time::stage("lsp: update", || self.update(&uri, text));
            }
        }
    }

    /// Analyzes the document a request is about if it was edited since it was last analyzed.
    fn analyze_for(&mut self, params: &Json) {
        let Some(uri) = params.get("textDocument").get("uri").as_str() else {
            return;
        };
        if let Some(i) = self.dirty.iter().position(|d| d == uri) {
            let uri = self.dirty.remove(i);
            if let Some(text) = self.docs.get(&uri).map(|d| d.text.clone()) {
                a0_diag::time::stage("lsp: update", || self.update(&uri, text));
            }
        }
    }

    fn update(&mut self, uri: &str, text: String) {
        let mut sources = SourceMap::new();
        // The edited file's source: the first, unless manifests come before it.
        let mut source = SourceId(0);
        let mut files: Vec<(SourceId, String)> = Vec::new();
        // The manifests of its package and its dependencies, whose errors go to them.
        let mut manifests: Vec<(SourceId, PathBuf)> = Vec::new();
        let mut program_files: Vec<FileInfo> = Vec::new();
        // What is known about a program with errors still helps (types, definitions).
        let mut partial: Option<HirModule> = None;
        // Whether the program's files loaded (they do not with syntax errors).
        let mut loaded_all = self.lang.modules.is_none();
        // A compiler bug must not take the editor's language support down with it.
        let compiled = match (self.lang.modules, self.grammar.as_ref(), uri_to_path(uri)) {
            (Some(modules), Some(grammar), Some(path)) => {
                let open: HashMap<PathBuf, String> = self
                    .docs
                    .iter()
                    .filter_map(|(u, d)| {
                        Some((program::normalize(&uri_to_path(u)?), d.text.clone()))
                    })
                    .collect();
                let root = program::normalize(&path);
                let mut read = |p: &Path| {
                    if *p == root {
                        Some(text.clone())
                    } else {
                        open.get(p)
                            .cloned()
                            .or_else(|| std::fs::read_to_string(p).ok())
                    }
                };
                let root_name = path.to_string_lossy().into_owned();
                catch_unwind(AssertUnwindSafe(|| -> Result<Program, CompileError> {
                    // Imports resolve through the file's package (D81); git dependencies come
                    // from the cache `build` fetched them into.
                    let packages = match self.lang.packages_of(&path, &mut sources, &mut manifests)
                    {
                        Ok(p) => p,
                        Err(d) => {
                            source = sources.add(&root_name, &text);
                            return Err(CompileError::Diagnostics(d));
                        }
                    };
                    source = SourceId(sources.len());
                    let loaded = program::load_cached(
                        grammar,
                        &modules,
                        &root_name,
                        packages.as_ref(),
                        &mut read,
                        &mut sources,
                        Some(&mut self.trees),
                    )
                    .map_err(CompileError::Diagnostics)?;
                    files = loaded
                        .files
                        .iter()
                        .map(|f| {
                            let file_uri = if program::normalize(&f.path) == root {
                                uri.to_string()
                            } else {
                                path_to_uri(&f.path)
                            };
                            (f.source, file_uri)
                        })
                        .collect();
                    let indexes = &mut self.indexes;
                    program_files = loaded
                        .files
                        .iter()
                        .zip(&files)
                        .map(|(f, (_, file_uri))| {
                            let text = sources.text(f.source).to_string();
                            // An unchanged file's index is kept from the last analysis (D82).
                            let index = match indexes.get(&f.path) {
                                Some((old, index)) if *old == text => {
                                    let mut index = index.clone();
                                    index.set_source(f.source);
                                    index
                                }
                                _ => {
                                    let index = SyntaxIndex::new(grammar, &f.tree);
                                    indexes.insert(f.path.clone(), (text.clone(), index.clone()));
                                    index
                                }
                            };
                            FileInfo {
                                source: f.source,
                                uri: file_uri.clone(),
                                lines: Lines::new(&text),
                                text,
                                index,
                                imports: f
                                    .imports
                                    .iter()
                                    .map(|(import, file)| {
                                        (import.alias.clone(), *file, import.names.clone())
                                    })
                                    .collect(),
                            }
                        })
                        .collect();
                    loaded_all = true;
                    let (m, diags) = program::analyze(&loaded, &modules, self.lang.name);
                    if diags.has_errors() {
                        partial = Some(m);
                        return Err(CompileError::Diagnostics(diags));
                    }
                    Ok(Program::Hir(m))
                }))
            }
            _ => {
                sources.add(uri, &text);
                let compile = self.lang.compile;
                catch_unwind(AssertUnwindSafe(|| compile(&text, source)))
            }
        };
        if sources.is_empty() {
            sources.add(uri, &text);
        }
        if !files.iter().any(|(id, _)| *id == source) {
            files.push((source, uri.to_string()));
        }
        files.extend(manifests.iter().map(|(id, p)| (*id, path_to_uri(p))));
        let lines = Lines::new(&text);
        let index = self.grammar.as_ref().map(|g| {
            let (tree, _) = g.parse(&text, source);
            SyntaxIndex::new(g, &tree)
        });
        // Per file of the program: its diagnostics.
        let mut published: HashMap<String, Vec<Json>> = HashMap::new();
        published.insert(uri.to_string(), Vec::new());
        let mut fixes = Vec::new();
        let mut hir;
        let file_of = |id: SourceId| files.iter().find(|(s, _)| *s == id).map(|(_, u)| u.clone());
        let range_of = |id: SourceId, start: u32, end: u32| {
            let text = sources.text(id);
            Lines::new(text).range(text, start, end)
        };
        let diagnostic = |range: Json, severity: u32, message: &str| {
            Json::object([
                ("range", range),
                ("severity", severity.into()),
                ("source", self.lang.name.into()),
                ("message", message.into()),
            ])
        };
        hir = partial;
        let (ds, messages) = match compiled {
            Ok(Ok(Program::Hir(m))) => {
                let warnings = m.warnings().clone();
                hir = Some(m);
                (Some(warnings), None)
            }
            Ok(Ok(Program::Ir(_))) => (None, None),
            Ok(Err(CompileError::Diagnostics(ds))) => (Some(ds), None),
            Ok(Err(CompileError::Messages(ms))) => (None, Some(Ok(ms))),
            Err(panic) => (None, Some(Err(panic))),
        };
        if let Some(ds) = ds {
            for d in self.lang.codes.apply(&ds).as_slice() {
                let (file, span) = match d.span {
                    Some(s) => match file_of(s.source) {
                        Some(f) => (f, Some(s)),
                        None => continue,
                    },
                    None => (uri.to_string(), None),
                };
                let severity = match d.severity {
                    Severity::Error => 1,
                    Severity::Warning => 2,
                    Severity::Note => 3,
                };
                let range = match span {
                    Some(s) => range_of(s.source, s.start, s.end),
                    None => lines.range(&text, 0, 0),
                };
                let mut json = diagnostic(range, severity, &full_message(d));
                if let Json::Object(fields) = &mut json {
                    if !d.code.is_empty() {
                        fields.push(("code".to_string(), d.code.as_str().into()));
                    }
                    let related: Vec<Json> = d
                        .labels
                        .iter()
                        .filter_map(|l| {
                            let file = file_of(l.span.source)?;
                            let range = range_of(l.span.source, l.span.start, l.span.end);
                            Some(Json::object([
                                (
                                    "location",
                                    Json::object([("uri", file.into()), ("range", range)]),
                                ),
                                ("message", l.message.as_str().into()),
                            ]))
                        })
                        .collect();
                    if !related.is_empty() {
                        fields.push(("relatedInformation".to_string(), related.into()));
                    }
                }
                if span.is_none_or(|s| s.source == source) {
                    let (start, end) = span.map_or((0, 0), |s| (s.start, s.end));
                    for fix in &d.fixes {
                        if fix.edits.iter().all(|e| e.span.source == source) {
                            fixes.push((start, end, json.clone(), fix.clone()));
                        }
                    }
                }
                published.entry(file).or_default().push(json);
            }
        }
        let mine = published.entry(uri.to_string()).or_default();
        match messages {
            None => {}
            Some(Ok(ms)) => {
                for m in ms {
                    let (start, end) = m.span.map_or((0, 0), |s| (s.start, s.end));
                    mine.push(diagnostic(lines.range(&text, start, end), 1, &m.text));
                }
            }
            Some(Err(panic)) => {
                let what = panic
                    .downcast_ref::<String>()
                    .map(String::as_str)
                    .or_else(|| panic.downcast_ref::<&str>().copied())
                    .unwrap_or("a panic");
                mine.push(diagnostic(
                    lines.range(&text, 0, 0),
                    1,
                    &format!("internal error in the {} compiler: {what}", self.lang.name),
                ));
            }
        }
        // The last types known help completion while the program does not compile.
        if hir.is_none() {
            hir = self.docs.get_mut(uri).and_then(|d| d.hir.take());
        }
        // A single file, or a program that did not load: the document alone.
        if program_files.is_empty()
            && let Some(g) = self.grammar.as_ref()
        {
            let (tree, _) = g.parse(&text, source);
            program_files.push(FileInfo {
                source,
                uri: uri.to_string(),
                text: text.clone(),
                lines: Lines::new(&text),
                index: SyntaxIndex::new(g, &tree),
                imports: Vec::new(),
            });
        }
        self.docs.insert(
            uri.to_string(),
            Document {
                text,
                lines,
                index,
                hir,
                source,
                fixes,
                sources,
                files,
                program: program_files,
                loaded: loaded_all,
            },
        );
        // Files this program's last analysis reported on and this one does not are now clean.
        let before = self.reported.remove(uri).unwrap_or_default();
        for old in before {
            published.entry(old).or_default();
        }
        let others: Vec<String> = published.keys().filter(|u| *u != uri).cloned().collect();
        self.reported.insert(uri.to_string(), others);
        let mut published: Vec<(String, Vec<Json>)> = published.into_iter().collect();
        published.sort_by(|a, b| a.0.cmp(&b.0));
        for (file, diagnostics) in published {
            // An open document of its own publishes its own diagnostics.
            if file != uri && self.docs.contains_key(&file) {
                continue;
            }
            self.publish(&file, diagnostics);
        }
    }

    /// Quick fixes for the diagnostics in the requested range: their suggested edits.
    fn code_actions(&self, params: &Json) -> Json {
        let Some(doc) = self.doc(params) else {
            return Json::Null;
        };
        let Some(uri) = params.get("textDocument").get("uri").as_str() else {
            return Json::Null;
        };
        let range = params.get("range");
        let offset = |p: &Json| {
            doc.lines.offset(
                &doc.text,
                p.get("line").as_u32().unwrap_or(0),
                p.get("character").as_u32().unwrap_or(0),
            )
        };
        let (from, to) = (offset(range.get("start")), offset(range.get("end")));
        let mut actions = Vec::new();
        for (start, end, diagnostic, fix) in &doc.fixes {
            if *start > to || *end < from {
                continue;
            }
            let edits: Vec<Json> = fix
                .edits
                .iter()
                .map(|e| {
                    Json::object([
                        (
                            "range",
                            doc.lines.range(&doc.text, e.span.start, e.span.end),
                        ),
                        ("newText", e.text.as_str().into()),
                    ])
                })
                .collect();
            let title = match fix.edits.as_slice() {
                [one] if !one.text.is_empty() => format!("{}: `{}`", fix.message, one.text),
                _ => fix.message.clone(),
            };
            actions.push(Json::object([
                ("title", title.into()),
                ("kind", "quickfix".into()),
                ("diagnostics", vec![diagnostic.clone()].into()),
                ("isPreferred", true.into()),
                (
                    "edit",
                    Json::object([(
                        "changes",
                        Json::Object(vec![(uri.to_string(), edits.into())]),
                    )]),
                ),
            ]));
        }
        actions.into()
    }

    fn publish(&mut self, uri: &str, diagnostics: Vec<Json>) {
        self.send(Json::object([
            ("jsonrpc", "2.0".into()),
            ("method", "textDocument/publishDiagnostics".into()),
            (
                "params",
                Json::object([("uri", uri.into()), ("diagnostics", diagnostics.into())]),
            ),
        ]));
    }

    fn definition(&self, params: &Json) -> Json {
        let Some((doc, offset)) = self.position(params) else {
            return Json::Null;
        };
        let Some(index) = &doc.index else {
            return Json::Null;
        };
        let Some(t) = index.token_at(offset) else {
            return Json::Null;
        };
        let uri = params.get("textDocument").get("uri").clone();
        let mut locations: Vec<Json> = index
            .targets(t)
            .iter()
            .map(|&s| {
                let span = index.symbol_span(s);
                Json::object([
                    ("uri", uri.clone()),
                    ("range", doc.lines.range(&doc.text, span.start, span.end)),
                ])
            })
            .collect();
        // Names the grammar cannot resolve, such as another module's items: the HIR knows what the
        // innermost expression around them refers to.
        if locations.is_empty()
            && let Some(m) = &doc.hir
            && let Some(span) = referenced_item_span(m, doc.source, offset)
            && let Some(file) = doc
                .files
                .iter()
                .find(|(id, _)| *id == span.source)
                .map(|(_, f)| f.clone())
                .or_else(|| builtin_file_uri(&doc.sources, span.source))
        {
            let text = doc.sources.text(span.source);
            let range = Lines::new(text).range(text, span.start, span.end);
            locations.push(Json::object([
                ("uri", file.as_str().into()),
                ("range", range),
            ]));
        }
        if locations.is_empty() {
            Json::Null
        } else {
            locations.into()
        }
    }

    fn hover(&self, params: &Json) -> Json {
        let Some((doc, offset)) = self.position(params) else {
            return Json::Null;
        };
        let Some(index) = &doc.index else {
            return Json::Null;
        };
        let Some(t) = index.token_at(offset) else {
            return Json::Null;
        };
        let token = index.token(t);
        let Some(&s) = index.targets(t).first() else {
            // Not declared in this file: another module's item, or the standard library's.
            let item = doc
                .hir
                .as_ref()
                .and_then(|m| Some((m, referenced_item(m, doc.source, offset)?)));
            let Some((m, item)) = item else {
                return Json::Null;
            };
            let value = format!("```{}\n{}\n```", self.lang.name, m.item_signature(item));
            return Json::object([
                (
                    "contents",
                    Json::object([("kind", "markdown".into()), ("value", value.into())]),
                ),
                ("range", doc.lines.range(&doc.text, token.start, token.end)),
            ]);
        };
        let name = index.symbol_name(s);
        let kind = index.symbol_kind(s);
        let mut value = String::new();
        let header = declaration_header(&doc.text, index.symbol_extent(s), kind);
        if header != name {
            value.push_str(&format!("```{}\n{header}\n```\n", self.lang.name));
        }
        value.push_str(&format!("{} `{name}`", kind_words(kind)));
        if let Some(ty) = doc.hir.as_ref().and_then(|m| hir_type(m, doc, index, s)) {
            value.push_str(&format!(": `{ty}`"));
        }
        Json::object([
            (
                "contents",
                Json::object([("kind", "markdown".into()), ("value", value.into())]),
            ),
            ("range", doc.lines.range(&doc.text, token.start, token.end)),
        ])
    }
}

/// Where the item that the innermost expression around `offset` (in file `source`) refers to is
/// declared: the function it calls, the struct it builds, the global it reads.
fn referenced_item_span(m: &HirModule, source: SourceId, offset: u32) -> Option<Span> {
    m.item_span(referenced_item(m, source, offset)?)
}

/// The item the innermost expression around `offset` (in file `source`) refers to.
fn referenced_item(m: &HirModule, source: SourceId, offset: u32) -> Option<Item> {
    let mut best: Option<(u32, Item)> = None;
    for e in 0..m.expr_count() {
        let e = a0_hir::ExprId(e);
        let Some(span) = m.expr_span(e) else { continue };
        if span.source != source || offset < span.start || offset > span.end {
            continue;
        }
        let Some(item) = m.expr_item(e) else { continue };
        if best.is_none_or(|(len, _)| span.len() < len) {
            best = Some((span.len(), item));
        }
    }
    best.map(|(_, item)| item)
}

/// The file a `file:` URI names (`file:///D:/a0/x.mn` is `D:/a0/x.mn`, `file:///home/x.mn` is
/// `/home/x.mn`), percent-escapes decoded.
/// A built-in source (a module of the standard library, `<std>/fs.hir`) as a file an editor can
/// open: written, the first time it is needed, to a directory of this version of A0 in the system's
/// temporary directory.
fn builtin_file_uri(sources: &SourceMap, id: SourceId) -> Option<String> {
    builtin_file(sources, id).map(|p| path_to_uri(&p))
}

/// The file [`builtin_file_uri`] names, written if need be (the debugger shows it too).
pub(crate) fn builtin_file(sources: &SourceMap, id: SourceId) -> Option<PathBuf> {
    if !SourceMap::is_builtin(id) {
        return None;
    }
    let name = sources.name(id).replace(['<', '>'], "");
    let path = std::env::temp_dir()
        .join(format!("a0-{}", env!("CARGO_PKG_VERSION")))
        .join(name);
    let text = sources.text(id);
    if std::fs::read_to_string(&path).ok().as_deref() != Some(text) {
        std::fs::create_dir_all(path.parent()?).ok()?;
        std::fs::write(&path, text).ok()?;
    }
    Some(path)
}

pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let mut bytes = Vec::new();
    let raw = rest.as_bytes();
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%'
            && i + 2 < raw.len()
            && let Ok(b) = u8::from_str_radix(&rest[i + 1..i + 3], 16)
        {
            bytes.push(b);
            i += 3;
            continue;
        }
        bytes.push(raw[i]);
        i += 1;
    }
    let path = String::from_utf8(bytes).ok()?;
    // `/D:/a0/x.mn`: a drive letter after the slash is a Windows path.
    let path = match path.as_bytes() {
        [b'/', d, b':', ..] if d.is_ascii_alphabetic() => path[1..].to_string(),
        _ => path,
    };
    Some(PathBuf::from(path))
}

/// The `file:` URI of a path, the way editors write it.
pub fn path_to_uri(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let mut out = String::from("file://");
    if !text.starts_with('/') {
        out.push('/');
    }
    for c in text.chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '/' | '-' | '_' | '.' | '~' | ':' => out.push(c),
            c => {
                let mut buf = [0; 4];
                for b in c.encode_utf8(&mut buf).bytes() {
                    out.push_str(&format!("%{b:02X}"));
                }
            }
        }
    }
    out
}

/// A diagnostic's message as an editor shows it: the message, what is wrong at its span, then its
/// notes, helps and fixes.
fn full_message(d: &Diagnostic) -> String {
    let mut message = d.message.clone();
    if !d.label.is_empty() {
        message.push('\n');
        message.push_str(&d.label);
    }
    for note in &d.notes {
        message.push_str("\nnote: ");
        message.push_str(note);
    }
    for help in &d.helps {
        message.push_str("\nhelp: ");
        message.push_str(help);
    }
    for fix in &d.fixes {
        message.push_str("\nhelp: ");
        message.push_str(&fix.message);
        if let [one] = fix.edits.as_slice() {
            message.push_str(&format!(": `{}`", one.text));
        }
    }
    message
}

fn kind_words(kind: SymbolKind) -> &'static str {
    match kind {
        SymbolKind::TypeParameter => "type parameter",
        k => k.name(),
    }
}

/// The first line of a declaration, up to the `{` of its body for functions and types:
/// `fun fib(n)`, `let p = new Point { x: 1 };`.
fn declaration_header(text: &str, extent: Span, kind: SymbolKind) -> String {
    let decl = &text[extent.start as usize..extent.end as usize];
    let line = decl.lines().next().unwrap_or("");
    let has_body = !matches!(
        kind,
        SymbolKind::Variable | SymbolKind::Constant | SymbolKind::Field | SymbolKind::Parameter
    );
    let header = match line.find('{') {
        Some(i) if has_body => &line[..i],
        _ => line.strip_suffix('{').unwrap_or(line),
    }
    .trim();
    match header.char_indices().nth(100) {
        Some((i, _)) => format!("{}…", &header[..i]),
        None => header.to_string(),
    }
}

/// The type of a variable, parameter or field, from the HIR when the compiler recorded where its
/// declarations are. (A function's declaration already shows its signature.)
fn hir_type(m: &HirModule, doc: &Document, index: &SyntaxIndex, s: u32) -> Option<String> {
    let name = index.symbol_name(s);
    let span = index.symbol_span(s);
    let covers = |outer: Option<Span>| {
        outer.filter(|o| o.source == doc.source && o.start <= span.start && span.end <= o.end)
    };
    match index.symbol_kind(s) {
        SymbolKind::Field => {
            let owner = index.symbol_name(index.symbol_parent(s)?);
            let Some(Item::Struct(st)) = m.lookup_type(owner) else {
                return None;
            };
            let i = m.struct_field_index(st, name)?;
            Some(m.display_type(m.struct_field(st, i).1))
        }
        SymbolKind::Variable | SymbolKind::Parameter | SymbolKind::Constant => {
            // Locals may have been renamed to stay unique (`x_1`); the innermost one wins.
            let same_name = |local: &str| {
                local == name
                    || local
                        .strip_prefix(name)
                        .and_then(|rest| rest.strip_prefix('_'))
                        .is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()))
            };
            (0..m.local_count())
                .map(a0_hir::LocalId)
                .filter_map(|l| {
                    let outer = covers(m.local_span(l))?;
                    same_name(m.local_name(l)).then_some((outer.end - outer.start, l))
                })
                .min_by_key(|&(size, _)| size)
                .map(|(_, l)| m.display_type(m.local_type(l)))
        }
        _ => None,
    }
}

/// The document's tokens, encoded as the protocol's relative five-number groups.
fn semantic_tokens(doc: &Document) -> Json {
    let Some(index) = &doc.index else {
        return Vec::new().into();
    };
    let mut data = Vec::new();
    let (mut last_line, mut last_char) = (0, 0);
    for i in 0..index.token_count() {
        let t = index.token(i);
        let (kind, declaration) = match t.class {
            TokenClass::Keyword => ("keyword", false),
            TokenClass::Operator => ("operator", false),
            TokenClass::Number => ("number", false),
            TokenClass::String => ("string", false),
            TokenClass::Comment => ("comment", false),
            TokenClass::Name => match index.targets(i).first() {
                Some(&s) => (
                    symbol_token_type(index.symbol_kind(s)),
                    t.declares.is_some(),
                ),
                None if t.member => ("property", false),
                None => ("variable", false),
            },
            TokenClass::Other => continue,
        };
        // Tokens may not span lines: a multi-line string or comment becomes one token per line.
        let mut start = t.start;
        for piece in doc.text[t.start as usize..t.end as usize].split_inclusive('\n') {
            let text = piece.trim_end_matches(['\n', '\r']);
            let (line, character) = doc.lines.position(&doc.text, start);
            let length = text.chars().map(char::len_utf16).sum::<usize>() as u32;
            start += piece.len() as u32;
            if length == 0 {
                continue;
            }
            let delta_char = if line == last_line {
                character - last_char
            } else {
                character
            };
            data.extend([
                line - last_line,
                delta_char,
                length,
                token_type(kind),
                declaration as u32,
            ]);
            (last_line, last_char) = (line, character);
        }
    }
    data.into_iter().map(Json::from).collect::<Vec<_>>().into()
}

/// The outline: declarations nested in the declarations around them, without parameters and
/// without variables other than global ones.
fn document_symbols(doc: &Document) -> Json {
    let Some(index) = &doc.index else {
        return Vec::new().into();
    };
    let n = index.symbol_count();
    let shown: Vec<bool> = (0..n)
        .map(|s| match index.symbol_kind(s) {
            SymbolKind::Parameter | SymbolKind::TypeParameter => false,
            SymbolKind::Variable | SymbolKind::Constant => index.symbol_is_global(s),
            _ => true,
        })
        .collect();
    let mut children: Vec<Vec<u32>> = vec![Vec::new(); n as usize];
    let mut roots = Vec::new();
    for s in (0..n).filter(|&s| shown[s as usize]) {
        let mut p = index.symbol_parent(s);
        while let Some(q) = p.filter(|&q| !shown[q as usize]) {
            p = index.symbol_parent(q);
        }
        match p {
            Some(p) => children[p as usize].push(s),
            None => roots.push(s),
        }
    }
    fn symbol(doc: &Document, index: &SyntaxIndex, children: &[Vec<u32>], s: u32) -> Json {
        let extent = index.symbol_extent(s);
        let name = index.symbol_span(s);
        let kids: Vec<Json> = children[s as usize]
            .iter()
            .map(|&c| symbol(doc, index, children, c))
            .collect();
        Json::object([
            ("name", index.symbol_name(s).into()),
            ("kind", lsp_symbol_kind(index.symbol_kind(s)).into()),
            (
                "range",
                doc.lines.range(&doc.text, extent.start, extent.end),
            ),
            (
                "selectionRange",
                doc.lines.range(&doc.text, name.start, name.end),
            ),
            ("children", kids.into()),
        ])
    }
    roots
        .iter()
        .map(|&s| symbol(doc, index, &children, s))
        .collect::<Vec<_>>()
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ExitCodes;
    use a0_diag::Diagnostics;

    const GRAMMAR: &str = r#"
        grammar demo;
        skip /[ \t\r\n]+/;
        skip /\/\/[^\n]*/;
        token NUM /[0-9]+/;
        token STR /"[^"]*"/;
        token ID /[A-Za-z_][A-Za-z0-9_]*/;
        program = (!EOF item)* EOF;
        item @inline @recover(before "fn" "let") = fn_decl | let_stmt;
        fn_decl @def(function) @scope = "fn" name:ID "(" params? ")" "{" (!"}" !EOF use)* "}";
        params @def(parameter: param) = param:ID ("," param:ID)*;
        let_stmt @def(variable) = "let" name:ID "=" value:atom ";";
        use = value:atom ";";
        atom = NUM | STR | ID;
    "#;

    fn demo() -> Language {
        Language {
            name: "demo",
            compile: |text, source| {
                if text.contains("panic") {
                    panic!("boom");
                }
                if let Some(i) = text.find("oops") {
                    let mut d = Diagnostics::new();
                    let span = Span::new(source, i as u32, i as u32 + 4);
                    d.error("demo.oops", "no oops allowed", Some(span));
                    return Err(CompileError::Diagnostics(d));
                }
                let m = a0_hir::parse("module \"m\";\nfn main() {}\n", SourceId(u32::MAX));
                Ok(Program::Hir(m.unwrap()))
            },
            exit_codes: ExitCodes::default(),
            run_by_default: false,
            host_symbols: None,
            libraries: None,
            grammar: Some(GRAMMAR),
            codes: crate::Codes::A0,
            modules: None,
            repl_main: None,
        }
    }

    const URI: &str = "file:///demo.d";

    fn request(id: u32, method: &str, params: Json) -> Vec<u8> {
        frame(&Json::object([
            ("jsonrpc", "2.0".into()),
            ("id", id.into()),
            ("method", method.into()),
            ("params", params),
        ]))
    }

    fn notify(method: &str, params: Json) -> Vec<u8> {
        frame(&Json::object([
            ("jsonrpc", "2.0".into()),
            ("method", method.into()),
            ("params", params),
        ]))
    }

    fn at(id: u32, method: &str, line: u32, character: u32) -> Vec<u8> {
        request(
            id,
            method,
            Json::object([
                ("textDocument", Json::object([("uri", URI.into())])),
                (
                    "position",
                    Json::object([("line", line.into()), ("character", character.into())]),
                ),
            ]),
        )
    }

    fn open(text: &str) -> Vec<u8> {
        notify(
            "textDocument/didOpen",
            Json::object([(
                "textDocument",
                Json::object([
                    ("uri", URI.into()),
                    ("languageId", "demo".into()),
                    ("version", 1u32.into()),
                    ("text", text.into()),
                ]),
            )]),
        )
    }

    fn run(input: Vec<Vec<u8>>) -> (Vec<Json>, u8) {
        let input = input.concat();
        let mut output = Vec::new();
        let code = demo().serve_lsp(Box::new(std::io::Cursor::new(input)), &mut output);
        (messages(&output), code)
    }

    fn response(messages: &[Json], id: u32) -> &Json {
        messages
            .iter()
            .find(|m| m.get("id").as_u32() == Some(id))
            .unwrap_or_else(|| panic!("no response {id}"))
    }

    fn diagnostics(messages: &[Json]) -> Vec<&Json> {
        messages
            .iter()
            .filter(|m| m.get("method").as_str() == Some("textDocument/publishDiagnostics"))
            .map(|m| m.get("params").get("diagnostics"))
            .collect()
    }

    const TEXT: &str = "// é comment\nlet x = \"a\nb\";\nfn f(a, b) {\n  a;\n  x;\n}\nlet s = \"😀\"; let t = s;\n";

    #[test]
    fn a_session_answers_every_request() {
        let td = || Json::object([("textDocument", Json::object([("uri", URI.into())]))]);
        let change = notify(
            "textDocument/didChange",
            Json::object([
                (
                    "textDocument",
                    Json::object([("uri", URI.into()), ("version", 2u32.into())]),
                ),
                (
                    "contentChanges",
                    vec![Json::object([("text", TEXT.into())])].into(),
                ),
            ]),
        );
        let (messages, code) = run(vec![
            request(
                1,
                "initialize",
                Json::object([("capabilities", Json::object([]))]),
            ),
            notify("initialized", Json::object([])),
            open("let x = oops;"),
            // Edits are analyzed when a request needs it (or when they pause).
            at(9, "textDocument/hover", 0, 4),
            change,
            request(2, "textDocument/semanticTokens/full", td()),
            request(3, "textDocument/documentSymbol", td()),
            // `s` in `let t = s;`, after a character that is two UTF-16 units.
            at(4, "textDocument/definition", 7, 22),
            at(5, "textDocument/hover", 4, 2),
            at(6, "textDocument/hover", 5, 3),
            request(7, "textDocument/formatting", td()),
            b"Content-Length: 5\r\n\r\n{nope".to_vec(),
            request(8, "shutdown", Json::Null),
            notify("exit", Json::Null),
        ]);
        assert_eq!(code, 0);

        let caps = response(&messages, 1).get("result").get("capabilities");
        assert_eq!(caps.get("hoverProvider"), &Json::Bool(true));
        let legend = caps.get("semanticTokensProvider").get("legend");
        assert_eq!(legend.get("tokenTypes").as_array().len(), TOKEN_TYPES.len());

        // The compiler's error, then nothing once it is fixed.
        let diags = diagnostics(&messages);
        assert_eq!(diags.len(), 2);
        let error = &diags[0].as_array()[0];
        assert_eq!(error.get("message").as_str(), Some("no oops allowed"));
        let range = error.get("range");
        assert_eq!(range.get("start").get("character").as_u32(), Some(8));
        assert_eq!(range.get("end").get("character").as_u32(), Some(12));
        assert!(diags[1].as_array().is_empty());

        // Tokens: (line, character, length, type, declaration), one per line for a split string.
        let data: Vec<u32> = response(&messages, 2)
            .get("result")
            .get("data")
            .as_array()
            .iter()
            .map(|n| n.as_u32().unwrap())
            .collect();
        let mut tokens = Vec::new();
        let (mut line, mut character) = (0, 0);
        for t in data.chunks(5) {
            line += t[0];
            character = if t[0] == 0 { character + t[1] } else { t[1] };
            tokens.push((line, character, t[2], TOKEN_TYPES[t[3] as usize], t[4]));
        }
        for expected in [
            (0, 0, 12, "comment", 0),
            (1, 0, 3, "keyword", 0),
            (1, 4, 1, "variable", 1),
            (1, 8, 2, "string", 0),
            (2, 0, 2, "string", 0),
            (3, 3, 1, "function", 1),
            (3, 5, 1, "parameter", 1),
            (4, 2, 1, "parameter", 0),
            (5, 2, 1, "variable", 0),
            (7, 8, 4, "string", 0),
            (7, 22, 1, "variable", 0),
        ] {
            assert!(tokens.contains(&expected), "{expected:?} not in {tokens:?}");
        }

        // The outline has the global variables and the function, not its parameters.
        let names: Vec<&str> = response(&messages, 3)
            .get("result")
            .as_array()
            .iter()
            .map(|s| s.get("name").as_str().unwrap())
            .collect();
        assert_eq!(names, ["x", "f", "s", "t"]);

        let def = &response(&messages, 4).get("result").as_array()[0];
        assert_eq!(def.get("uri").as_str(), Some(URI));
        let start = def.get("range").get("start");
        let start = (start.get("line").as_u32(), start.get("character").as_u32());
        assert_eq!(start, (Some(7), Some(4)));

        let hover = |id| {
            response(&messages, id)
                .get("result")
                .get("contents")
                .get("value")
                .as_str()
                .unwrap()
                .to_string()
        };
        assert_eq!(hover(5), "parameter `a`");
        assert_eq!(hover(6), "```demo\nlet x = \"a\n```\nvariable `x`");

        let rename = response(&messages, 7).get("error");
        assert_eq!(rename.get("code").as_f64(), Some(-32601.0));
        let bad = messages
            .iter()
            .find(|m| m.get("error").get("code").as_f64() == Some(-32700.0));
        assert!(bad.is_some(), "{messages:?}");
        assert!(response(&messages, 8).get("result").is_null());
    }

    #[test]
    fn exit_without_shutdown_fails_and_compiler_panics_are_reported() {
        let (messages, code) = run(vec![
            open("let x = 1; // panic\n"),
            at(2, "textDocument/hover", 0, 4),
            notify("exit", Json::Null),
        ]);
        assert_eq!(code, 1);
        let diags = diagnostics(&messages);
        let message = diags[0].as_array()[0].get("message").as_str().unwrap();
        assert_eq!(message, "internal error in the demo compiler: boom");
        // The end of the input also ends the session.
        assert_eq!(run(Vec::new()).1, 1);
    }

    #[test]
    fn positions_count_utf16_units() {
        let text = "a😀b\né\n";
        let lines = Lines::new(text);
        assert_eq!(lines.position(text, 5), (0, 3));
        assert_eq!(lines.offset(text, 0, 3), 5);
        assert_eq!(lines.offset(text, 1, 1), 9);
        assert_eq!(lines.offset(text, 1, 99), 9);
        assert_eq!(lines.offset(text, 9, 0), text.len() as u32);
    }
}
