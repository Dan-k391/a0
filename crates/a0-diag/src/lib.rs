//! Source files, spans and diagnostics shared by every A0 layer.
//!
//! Every layer reports problems as [`Diagnostic`]s collected in a [`Diagnostics`] list instead of
//! panicking or returning ad-hoc error strings. Source text is registered in a [`SourceMap`]; a
//! [`Span`] is a byte range inside one registered source. Diagnostics render the way rustc's do
//! (see [`RenderOptions`]).

pub mod codes;
mod render;
mod similar;
pub mod time;

use std::fmt::Write as _;
use std::sync::RwLock;

pub use render::RenderOptions;
pub use similar::similar_name;

/// Identifies a source registered in a [`SourceMap`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct SourceId(pub u32);

/// A byte range `start..end` in one source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Span {
    pub source: SourceId,
    pub start: u32,
    pub end: u32,
}

impl Span {
    /// A span from `start` to `end`; an `end` before `start` (a host's mistake through the C API)
    /// is taken as `start`.
    pub fn new(source: SourceId, start: u32, end: u32) -> Span {
        Span {
            source,
            start,
            end: end.max(start),
        }
    }

    /// The smallest span covering both `self` and `other` (which must share a source).
    pub fn to(self, other: Span) -> Span {
        Span {
            source: self.source,
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    pub fn len(self) -> u32 {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(self) -> bool {
        self.start == self.end
    }
}

#[derive(Clone)]
struct SourceFile {
    name: String,
    text: String,
    /// Byte offset of the first character of every line.
    line_starts: Vec<u32>,
}

impl SourceFile {
    fn new(name: &str, text: &str) -> SourceFile {
        let mut line_starts = vec![0];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i as u32 + 1);
            }
        }
        SourceFile {
            name: name.to_string(),
            text: text.to_string(),
            line_starts,
        }
    }
}

/// The first id of the sources built into the tools rather than read from files (the standard
/// library's modules, `a0-hir`): every [`SourceMap`] knows them, from [`register_builtin_source`].
pub const BUILTIN_SOURCES: u32 = 0xf000_0000;

static BUILTINS: RwLock<Vec<Option<&'static SourceFile>>> = RwLock::new(Vec::new());

/// Registers the built-in source number `index` (its id is `BUILTIN_SOURCES + index`), unless it
/// is registered already, and returns its id. Built-in sources are the same for the whole process,
/// so they are kept once, outside any map.
pub fn register_builtin_source(index: u32, name: &str, text: &str) -> SourceId {
    let id = SourceId(BUILTIN_SOURCES + index);
    let mut builtins = BUILTINS.write().unwrap_or_else(|e| e.into_inner());
    let i = index as usize;
    if builtins.len() <= i {
        builtins.resize(i + 1, None);
    }
    if builtins[i].is_none() {
        builtins[i] = Some(Box::leak(Box::new(SourceFile::new(name, text))));
    }
    id
}

fn builtin(id: SourceId) -> Option<&'static SourceFile> {
    let index = id.0.checked_sub(BUILTIN_SOURCES)? as usize;
    let builtins = BUILTINS.read().unwrap_or_else(|e| e.into_inner());
    builtins.get(index).copied().flatten()
}

/// Owns the text of every source that spans can point into, besides the built-in ones.
#[derive(Clone, Default)]
pub struct SourceMap {
    files: Vec<SourceFile>,
}

impl SourceMap {
    pub fn new() -> SourceMap {
        SourceMap::default()
    }

    /// Registers a source and returns its id. `name` is used in rendered diagnostics.
    pub fn add(&mut self, name: &str, text: &str) -> SourceId {
        self.files.push(SourceFile::new(name, text));
        SourceId(self.files.len() as u32 - 1)
    }

    /// The number of sources added to this map (the built-in ones not counted).
    pub fn len(&self) -> u32 {
        self.files.len() as u32
    }

    /// Whether spans in `id` can be shown: a source of this map or a built-in one.
    pub fn contains(&self, id: SourceId) -> bool {
        id.0 < self.len() || builtin(id).is_some()
    }

    /// Whether `id` is a built-in source ([`register_builtin_source`]).
    pub fn is_builtin(id: SourceId) -> bool {
        builtin(id).is_some()
    }

    /// The built-in source named `name`, if one is registered.
    pub fn builtin_named(name: &str) -> Option<SourceId> {
        let builtins = BUILTINS.read().unwrap_or_else(|e| e.into_inner());
        let index = builtins
            .iter()
            .position(|f| f.is_some_and(|f| f.name == name))?;
        Some(SourceId(BUILTIN_SOURCES + index as u32))
    }

    pub(crate) fn file(&self, id: SourceId) -> &SourceFile {
        match self.files.get(id.0 as usize) {
            Some(f) => f,
            None => builtin(id).unwrap_or_else(|| panic!("no source {}", id.0)),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn name(&self, id: SourceId) -> &str {
        &self.file(id).name
    }

    pub fn text(&self, id: SourceId) -> &str {
        &self.file(id).text
    }

    /// The text covered by `span`.
    pub fn slice(&self, span: Span) -> &str {
        let text = self.text(span.source);
        let start = text.floor_char_boundary(span.start as usize);
        &text[start..text.ceil_char_boundary((span.end as usize).max(start))]
    }

    /// 1-based line number of a byte offset.
    pub fn line(&self, id: SourceId, offset: u32) -> u32 {
        let starts = &self.file(id).line_starts;
        match starts.binary_search(&offset) {
            Ok(i) => i as u32 + 1,
            Err(i) => i as u32,
        }
    }

    /// 1-based line and column (counted in characters) of a byte offset.
    pub fn line_col(&self, id: SourceId, offset: u32) -> (u32, u32) {
        let line = self.line(id, offset);
        let start = self.file(id).line_starts[line as usize - 1];
        let text = self.text(id);
        let text = &text[start as usize..text.floor_char_boundary(offset as usize)];
        (line, text.chars().count() as u32 + 1)
    }

    pub fn line_count(&self, id: SourceId) -> u32 {
        self.file(id).line_starts.len() as u32
    }

    /// The text of a 1-based line, without its line terminator.
    pub fn line_text(&self, id: SourceId, line: u32) -> &str {
        let file = self.file(id);
        let start = file.line_starts[line as usize - 1] as usize;
        let end = file
            .line_starts
            .get(line as usize)
            .map(|&e| e as usize)
            .unwrap_or(file.text.len());
        file.text[start..end].trim_end_matches(['\n', '\r'])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Severity {
    Error,
    Warning,
    Note,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Note => "note",
        }
    }
}

/// A secondary span with its own message: a related location, such as an earlier definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Label {
    pub span: Span,
    pub message: String,
}

/// A change to the source: replace `span` with `text` (insert when the span is empty).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub span: Span,
    pub text: String,
}

/// A suggested fix: edits that together resolve a diagnostic, which editors offer as a quick fix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fix {
    /// What the fix does, as in "a local with a similar name exists".
    pub message: String,
    pub edits: Vec<Edit>,
}

/// One problem report.
///
/// `code` is a short, stable identifier that tools can match on; `message` is the human-readable
/// text, and `label` says what is wrong at `span` itself. `labels` point at related locations,
/// `notes` and `helps` add explanations, and `fixes` suggest edits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: String,
    pub message: String,
    pub span: Option<Span>,
    /// The text under the primary span.
    pub label: String,
    pub labels: Vec<Label>,
    pub notes: Vec<String>,
    pub helps: Vec<String>,
    pub fixes: Vec<Fix>,
}

impl Diagnostic {
    pub fn new(severity: Severity, code: &str, message: &str) -> Diagnostic {
        Diagnostic {
            severity,
            code: code.to_string(),
            message: message.to_string(),
            span: None,
            label: String::new(),
            labels: Vec::new(),
            notes: Vec::new(),
            helps: Vec::new(),
            fixes: Vec::new(),
        }
    }

    pub fn error(code: &str, message: &str) -> Diagnostic {
        Diagnostic::new(Severity::Error, code, message)
    }

    pub fn warning(code: &str, message: &str) -> Diagnostic {
        Diagnostic::new(Severity::Warning, code, message)
    }

    pub fn with_span(mut self, span: Span) -> Diagnostic {
        self.span = Some(span);
        self
    }

    pub fn with_span_opt(mut self, span: Option<Span>) -> Diagnostic {
        self.span = span;
        self
    }

    /// Says what is wrong at the primary span.
    pub fn with_primary_label(mut self, message: &str) -> Diagnostic {
        self.label = message.to_string();
        self
    }

    /// Points at a related location, if there is one.
    pub fn with_label_opt(self, span: Option<Span>, message: &str) -> Diagnostic {
        match span {
            Some(s) => self.with_label(s, message),
            None => self,
        }
    }

    /// Points at a related location.
    pub fn with_label(mut self, span: Span, message: &str) -> Diagnostic {
        self.labels.push(Label {
            span,
            message: message.to_string(),
        });
        self
    }

    pub fn with_note(mut self, note: &str) -> Diagnostic {
        self.notes.push(note.to_string());
        self
    }

    pub fn with_help(mut self, help: &str) -> Diagnostic {
        self.helps.push(help.to_string());
        self
    }

    /// Suggests replacing `span` with `text`.
    pub fn with_fix(self, message: &str, span: Span, text: &str) -> Diagnostic {
        self.with_fix_edits(
            message,
            vec![Edit {
                span,
                text: text.to_string(),
            }],
        )
    }

    /// Suggests several edits made together.
    pub fn with_fix_edits(mut self, message: &str, edits: Vec<Edit>) -> Diagnostic {
        self.fixes.push(Fix {
            message: message.to_string(),
            edits,
        });
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }

    /// Renders this diagnostic the way rustc does, without colors.
    pub fn render(&self, sources: &SourceMap) -> String {
        self.render_with(sources, &RenderOptions::default())
    }

    pub fn render_with(&self, sources: &SourceMap, options: &RenderOptions) -> String {
        render::render(self, sources, options)
    }
}

/// An ordered list of diagnostics.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diagnostics {
    items: Vec<Diagnostic>,
}

impl Diagnostics {
    pub fn new() -> Diagnostics {
        Diagnostics::default()
    }

    pub fn push(&mut self, d: Diagnostic) {
        self.items.push(d);
    }

    /// Shorthand for pushing an error with an optional span.
    pub fn error(&mut self, code: &str, message: &str, span: Option<Span>) {
        self.push(Diagnostic::error(code, message).with_span_opt(span));
    }

    pub fn extend(&mut self, other: Diagnostics) {
        self.items.extend(other.items);
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn get(&self, index: usize) -> &Diagnostic {
        &self.items[index]
    }

    /// The diagnostic pushed last, to add labels, notes or fixes to it.
    pub fn last_mut(&mut self) -> Option<&mut Diagnostic> {
        self.items.last_mut()
    }

    pub fn as_slice(&self) -> &[Diagnostic] {
        &self.items
    }

    pub fn into_vec(self) -> Vec<Diagnostic> {
        self.items
    }

    pub fn has_errors(&self) -> bool {
        self.items.iter().any(Diagnostic::is_error)
    }

    pub fn error_count(&self) -> usize {
        self.items.iter().filter(|d| d.is_error()).count()
    }

    /// Sorts by source position; diagnostics without a span keep their order and go last.
    pub fn sort_by_position(&mut self) {
        self.items.sort_by_key(|d| {
            d.span
                .map(|s| (0, s.source, s.start))
                .unwrap_or((1, SourceId(0), 0))
        });
    }

    /// Renders all diagnostics, separated by blank lines.
    pub fn render(&self, sources: &SourceMap) -> String {
        self.render_with(sources, &RenderOptions::default())
    }

    pub fn render_with(&self, sources: &SourceMap, options: &RenderOptions) -> String {
        let mut out = String::new();
        for (i, d) in self.items.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(&d.render_with(sources, options));
        }
        out
    }

    /// Renders without source snippets: one `code: message` line per diagnostic.
    pub fn summary(&self) -> String {
        let mut out = String::new();
        for d in &self.items {
            let _ = writeln!(out, "{}: {}", d.severity.as_str(), d.message);
        }
        out
    }
}

impl From<Diagnostic> for Diagnostics {
    fn from(d: Diagnostic) -> Diagnostics {
        Diagnostics { items: vec![d] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_and_column() {
        let mut sm = SourceMap::new();
        let id = sm.add("t.txt", "ab\ncdé\nf");
        assert_eq!(sm.line_col(id, 0), (1, 1));
        assert_eq!(sm.line_col(id, 2), (1, 3));
        assert_eq!(sm.line_col(id, 3), (2, 1));
        // 'é' is two bytes; the column of the following newline counts characters.
        assert_eq!(sm.line_col(id, 7), (2, 4));
        assert_eq!(sm.line_col(id, 8), (3, 1));
        assert_eq!(sm.line_text(id, 2), "cdé");
        assert_eq!(sm.line_count(id), 3);
    }

    #[test]
    fn render_points_at_span() {
        let mut sm = SourceMap::new();
        let id = sm.add("main.lox", "print 1 +;\n");
        let d = Diagnostic::error("syntax.expected", "expected expression")
            .with_span(Span::new(id, 9, 10))
            .with_note("an operand must follow '+'");
        let text = d.render(&sm);
        assert_eq!(
            text,
            "error[syntax.expected]: expected expression\n --> main.lox:1:10\n  |\n1 | print 1 +;\n  |          ^\n  |\n  = note: an operand must follow '+'\n"
        );
    }

    #[test]
    fn counts_errors() {
        let mut ds = Diagnostics::new();
        ds.push(Diagnostic::warning("w", "careful"));
        assert!(!ds.has_errors());
        ds.error("e", "broken", None);
        assert!(ds.has_errors());
        assert_eq!(ds.error_count(), 1);
        assert_eq!(ds.summary(), "warning: careful\nerror: broken\n");
    }
}
