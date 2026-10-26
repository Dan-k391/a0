//! Renders diagnostics the way rustc does.
//!
//! A diagnostic shows each source file it points into once: the primary span and every label in
//! that file share one snippet, with the primary underlined `^^^` and the others `---`. Spans over
//! several lines are drawn with a track in the left margin; lines between annotations are shown
//! when only one is missing and elided with `...` otherwise; very long lines are cut around what
//! they point at. Notes and helps follow, then suggested fixes, shown inline when they replace the
//! primary span and as a corrected snippet otherwise. Colors are ANSI escapes, for terminals.

use std::collections::BTreeSet;

use crate::{Diagnostic, Fix, Severity, SourceId, SourceMap, Span};

/// How diagnostics are rendered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderOptions {
    /// Color with ANSI escapes, for terminals.
    pub color: bool,
    /// Show codes, as in `error[E0308]`.
    pub codes: bool,
    /// Source lines wider than this many columns are cut around what the diagnostic points at.
    pub width: u32,
}

impl Default for RenderOptions {
    fn default() -> Self {
        RenderOptions {
            color: false,
            codes: true,
            width: 140,
        }
    }
}

impl RenderOptions {
    /// Options for writing to stderr: colored when it is a terminal, unless `NO_COLOR` is set;
    /// `CLICOLOR_FORCE=1` colors even when it is not (both as on <https://no-color.org>).
    pub fn stderr() -> RenderOptions {
        use std::io::IsTerminal;
        let set = |name: &str| std::env::var_os(name).is_some_and(|v| !v.is_empty());
        let color = if set("NO_COLOR") {
            false
        } else if std::env::var_os("CLICOLOR_FORCE").is_some_and(|v| v != "0") {
            true
        } else {
            std::io::stderr().is_terminal() && enable_ansi()
        };
        RenderOptions {
            color,
            ..RenderOptions::default()
        }
    }
}

/// Makes the console interpret ANSI escapes, which Windows consoles do only when asked.
#[cfg(windows)]
fn enable_ansi() -> bool {
    type Handle = *mut core::ffi::c_void;
    const STD_ERROR_HANDLE: u32 = -12i32 as u32;
    const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;
    unsafe extern "system" {
        fn GetStdHandle(which: u32) -> Handle;
        fn GetConsoleMode(console: Handle, mode: *mut u32) -> i32;
        fn SetConsoleMode(console: Handle, mode: u32) -> i32;
    }
    // SAFETY: plain Win32 calls on the process's own stderr handle; `mode` outlives the call.
    unsafe {
        let handle = GetStdHandle(STD_ERROR_HANDLE);
        let mut mode = 0;
        if GetConsoleMode(handle, &mut mode) == 0 {
            return false;
        }
        mode & ENABLE_VIRTUAL_TERMINAL_PROCESSING != 0
            || SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING) != 0
    }
}

#[cfg(not(windows))]
fn enable_ansi() -> bool {
    std::env::var_os("TERM").is_none_or(|t| t != "dumb")
}

/// Columns a tab takes in a snippet.
const TAB: usize = 4;
/// A multi-line span over more lines than this shows only its first and last two.
const LONG_SPAN: u32 = 6;
/// A replacement this short, of the primary span, is shown in the primary label.
const INLINE_FIX: usize = 30;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Style {
    Plain,
    /// The severity word and the primary markers and label.
    Level(Severity),
    /// The message after the severity.
    Message,
    /// Secondary markers and labels, line numbers, `|`, `-->`, `=`.
    Accent,
    /// `help:` and the markers of suggested edits.
    Help,
    /// Text a suggestion inserts.
    Insert,
}

fn ansi(style: Style) -> &'static str {
    match style {
        Style::Plain => "",
        Style::Level(Severity::Error) => "\x1b[1;91m",
        Style::Level(Severity::Warning) => "\x1b[1;93m",
        Style::Level(Severity::Note) => "\x1b[1;92m",
        Style::Message => "\x1b[1m",
        Style::Accent => "\x1b[1;94m",
        Style::Help => "\x1b[1;96m",
        Style::Insert => "\x1b[92m",
    }
}

struct Out {
    text: String,
    color: bool,
}

impl Out {
    fn put(&mut self, style: Style, s: &str) {
        if self.color && style != Style::Plain && !s.is_empty() {
            self.text.push_str(ansi(style));
            self.text.push_str(s);
            self.text.push_str("\x1b[0m");
        } else {
            self.text.push_str(s);
        }
    }

    fn line(&mut self) {
        self.text.push('\n');
    }

    /// `   |` with the bar after a gutter of `gw` columns.
    fn bar(&mut self, gw: usize) {
        self.put(Style::Plain, &" ".repeat(gw + 1));
        self.put(Style::Accent, "|");
        self.line();
    }
}

/// Characters of one output row, each with a style; columns are display columns.
#[derive(Default)]
struct Row {
    cells: Vec<(char, Style)>,
}

impl Row {
    fn put(&mut self, col: usize, s: &str, style: Style) {
        for (i, c) in s.chars().enumerate() {
            let at = col + i;
            if self.cells.len() <= at {
                self.cells.resize(at + 1, (' ', Style::Plain));
            }
            self.cells[at] = (c, style);
        }
    }

    fn emit(&self, out: &mut Out) {
        let end = self
            .cells
            .iter()
            .rposition(|&(c, _)| c != ' ')
            .map_or(0, |i| i + 1);
        let mut i = 0;
        while i < end {
            let style = self.cells[i].1;
            let mut run = String::new();
            while i < end && self.cells[i].1 == style {
                run.push(self.cells[i].0);
                i += 1;
            }
            out.put(style, &run);
        }
    }
}

/// A span as drawn: display columns, 0-based, on 1-based lines.
#[derive(Clone, Debug)]
struct Ann {
    start_line: u32,
    start_col: usize,
    end_line: u32,
    /// Exclusive on a single line; on the last line of a multi-line span, one past its last
    /// character.
    end_col: usize,
    primary: bool,
    message: String,
    /// The label is a suggestion (`help: ...`).
    help: bool,
}

impl Ann {
    fn multi(&self) -> bool {
        self.start_line != self.end_line
    }
}

/// The text of a line as shown: tabs expanded.
fn shown(text: &str) -> String {
    text.replace('\t', &" ".repeat(TAB))
}

/// The display column of byte `offset` within `line`.
fn display_col(line: &str, offset: usize) -> usize {
    line[..line.floor_char_boundary(offset)]
        .chars()
        .map(|c| if c == '\t' { TAB } else { 1 })
        .sum()
}

/// Line number and display column of a byte offset.
fn position(sources: &SourceMap, id: SourceId, offset: u32) -> (u32, usize) {
    let line = sources.line(id, offset);
    let start = sources.file(id).line_starts[line as usize - 1];
    let text = sources.line_text(id, line);
    (line, display_col(text, (offset - start) as usize))
}

fn annotation(sources: &SourceMap, span: Span, primary: bool, message: &str) -> Ann {
    let text = sources.text(span.source);
    // Spans from a language's code may cut through a character; draw the whole character.
    let start = text.floor_char_boundary(span.start as usize) as u32;
    let mut end = text.ceil_char_boundary((span.end as usize).max(start as usize)) as u32;
    // A span that ends with a line break ends on the line it breaks.
    while end > start && matches!(text.as_bytes()[end as usize - 1], b'\n' | b'\r') {
        end -= 1;
    }
    let (start_line, start_col) = position(sources, span.source, start);
    let (end_line, end_col) = if end > start {
        let last_start = text.floor_char_boundary(end as usize - 1);
        let (l, c) = position(sources, span.source, last_start as u32);
        // One past the last character, which may be a tab.
        let last = text[last_start..].chars().next();
        (l, c + if last == Some('\t') { TAB } else { 1 })
    } else {
        (start_line, start_col + 1)
    };
    Ann {
        start_line,
        start_col,
        end_line,
        end_col,
        primary,
        message: message.to_string(),
        help: false,
    }
}

/// Where a snippet's columns start, so that long lines fit in `width` columns.
struct Window {
    left: usize,
    width: usize,
}

impl Window {
    fn new(anns: &[Ann], lines: &[String], width: usize) -> Window {
        let longest = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
        if longest <= width {
            return Window {
                left: 0,
                width: usize::MAX,
            };
        }
        let lo = anns
            .iter()
            .map(|a| a.start_col.min(a.end_col))
            .min()
            .unwrap_or(0);
        let hi = anns
            .iter()
            .map(|a| a.start_col.max(a.end_col))
            .max()
            .unwrap_or(0);
        let left = if hi + 4 <= width {
            0
        } else {
            lo.saturating_sub(8)
        };
        Window { left, width }
    }

    /// The part of a line shown, with `...` where it is cut.
    fn cut(&self, line: &str) -> String {
        let chars: Vec<char> = line.chars().collect();
        if self.width == usize::MAX {
            return line.to_string();
        }
        let end = (self.left + self.width).min(chars.len());
        let mut s: String = chars[self.left.min(end)..end].iter().collect();
        if self.left > 0 {
            s = format!("...{}", s.chars().skip(3).collect::<String>());
        }
        if chars.len() > self.left + self.width {
            let keep = s.chars().count().saturating_sub(3);
            s = format!("{}...", s.chars().take(keep).collect::<String>());
        }
        s
    }

    /// A column of the line as a column of the cut line.
    fn col(&self, col: usize) -> usize {
        if self.width == usize::MAX || self.left == 0 {
            col
        } else {
            col.saturating_sub(self.left).max(3)
        }
    }
}

/// The lines a snippet shows: every annotated line, the inside of short multi-line spans, and a
/// single line between two shown ones. `None` stands for `...`.
fn lines_to_show(anns: &[Ann]) -> Vec<Option<u32>> {
    let mut set = BTreeSet::new();
    for a in anns {
        if a.multi() && a.end_line - a.start_line > LONG_SPAN {
            set.extend([a.start_line, a.start_line + 1, a.end_line - 1, a.end_line]);
        } else {
            set.extend(a.start_line..=a.end_line);
        }
    }
    let mut out = Vec::new();
    let mut prev: Option<u32> = None;
    for line in set {
        if let Some(p) = prev {
            if line == p + 2 {
                out.push(Some(p + 1));
            } else if line > p + 2 {
                out.push(None);
            }
        }
        out.push(Some(line));
        prev = Some(line);
    }
    out
}

/// The style of an annotation's markers.
fn mark_style(a: &Ann, level: Severity) -> Style {
    if a.primary {
        Style::Level(level)
    } else {
        Style::Accent
    }
}

/// The style of an annotation's label.
fn ann_style(a: &Ann, level: Severity) -> Style {
    if a.help {
        Style::Help
    } else {
        mark_style(a, level)
    }
}

/// The snippet of one file: its source lines and the rows that annotate them.
fn snippet(
    out: &mut Out,
    sources: &SourceMap,
    id: SourceId,
    anns: &[Ann],
    level: Severity,
    gw: usize,
    width: u32,
) {
    let lines = lines_to_show(anns);
    let texts: Vec<String> = lines
        .iter()
        .flatten()
        .map(|&l| shown(sources.line_text(id, l)))
        .collect();
    // Multi-line spans each get a track in the margin; a span that starts where its line's code
    // starts begins with `/` instead of a row of its own.
    let multis: Vec<&Ann> = anns.iter().filter(|a| a.multi()).collect();
    let tracks = multis.len();
    let margin = if tracks > 0 { tracks + 1 } else { 0 };
    let avail = (width as usize).saturating_sub(gw + 3 + margin).max(20);
    let window = Window::new(anns, &texts, avail);
    let slash: Vec<bool> = multis
        .iter()
        .map(|m| {
            let text = shown(sources.line_text(id, m.start_line));
            let indent = text.len() - text.trim_start().len();
            m.start_col <= indent
        })
        .collect();
    let code_col = |c: usize| margin + window.col(c);
    let emit = |out: &mut Out, number: Option<u32>, row: &Row| {
        match number {
            Some(n) => out.put(Style::Accent, &format!("{n:>gw$} |")),
            None => {
                out.put(Style::Plain, &" ".repeat(gw + 1));
                out.put(Style::Accent, "|");
            }
        }
        if !row.cells.is_empty() {
            out.put(Style::Plain, " ");
        }
        row.emit(out);
        out.line();
    };
    // The tracks drawn on rows below `line`, before any span ending there is closed.
    let active_below = |row: &mut Row, line: u32, closed: &[usize]| {
        for (t, m) in multis.iter().enumerate() {
            let started = m.start_line < line || (m.start_line == line && slash[t]);
            if started && m.end_line >= line && !closed.contains(&t) {
                row.put(t, "|", ann_style(m, level));
            }
        }
    };
    for line in lines {
        let Some(line) = line else {
            out.put(Style::Accent, "...");
            out.line();
            continue;
        };
        // The source line.
        let mut row = Row::default();
        for (t, m) in multis.iter().enumerate() {
            if m.start_line < line && m.end_line >= line {
                row.put(t, "|", ann_style(m, level));
            } else if m.start_line == line && slash[t] {
                row.put(t, "/", ann_style(m, level));
            }
        }
        row.put(
            margin,
            &window.cut(&shown(sources.line_text(id, line))),
            Style::Plain,
        );
        emit(out, Some(line), &row);

        // Single-line annotations: markers, then the labels that do not fit after them.
        let mut singles: Vec<&Ann> = anns
            .iter()
            .filter(|a| !a.multi() && a.start_line == line)
            .collect();
        singles.sort_by_key(|a| (a.start_col, a.primary));
        if !singles.is_empty() {
            let mut row = Row::default();
            active_below(&mut row, line, &[]);
            for pass in [false, true] {
                for a in singles.iter().filter(|a| a.primary == pass) {
                    let mark = if a.primary { "^" } else { "-" };
                    let width = a.end_col.saturating_sub(a.start_col).max(1);
                    row.put(
                        code_col(a.start_col),
                        &mark.repeat(width),
                        mark_style(a, level),
                    );
                }
            }
            let rightmost = singles
                .iter()
                .max_by_key(|a| (a.start_col, a.end_col))
                .unwrap();
            let inline = !rightmost.message.is_empty()
                && singles
                    .iter()
                    .all(|a| std::ptr::eq(*a, *rightmost) || a.end_col <= rightmost.start_col);
            if inline {
                let at = code_col(rightmost.end_col.max(rightmost.start_col + 1)) + 1;
                row.put(at, &rightmost.message, ann_style(rightmost, level));
            }
            emit(out, None, &row);
            let pending: Vec<&&Ann> = singles
                .iter()
                .filter(|a| !a.message.is_empty() && !(inline && std::ptr::eq(**a, *rightmost)))
                .collect();
            if !pending.is_empty() {
                let mut row = Row::default();
                active_below(&mut row, line, &[]);
                for a in &pending {
                    row.put(code_col(a.start_col), "|", ann_style(a, level));
                }
                emit(out, None, &row);
                for i in (0..pending.len()).rev() {
                    let mut row = Row::default();
                    active_below(&mut row, line, &[]);
                    for a in &pending[..i] {
                        row.put(code_col(a.start_col), "|", ann_style(a, level));
                    }
                    let a = pending[i];
                    row.put(code_col(a.start_col), &a.message, ann_style(a, level));
                    emit(out, None, &row);
                }
            }
        }

        // Multi-line spans that start here: `____^` from the track to the first character.
        for (t, m) in multis.iter().enumerate() {
            if m.start_line != line || slash[t] {
                continue;
            }
            let mut row = Row::default();
            active_below(&mut row, line, &[t]);
            let style = ann_style(m, level);
            let at = code_col(m.start_col);
            row.put(t + 1, &"_".repeat(at.saturating_sub(t + 1)), style);
            row.put(at, if m.primary { "^" } else { "-" }, style);
            emit(out, None, &row);
        }
        // Multi-line spans that end here: `|____^ label` from the track to the last character.
        let mut closed = Vec::new();
        for (t, m) in multis.iter().enumerate() {
            if m.end_line != line {
                continue;
            }
            let mut row = Row::default();
            active_below(&mut row, line, &closed);
            let style = ann_style(m, level);
            let at = code_col(m.end_col.saturating_sub(1));
            row.put(t, "|", style);
            row.put(t + 1, &"_".repeat(at.saturating_sub(t + 1)), style);
            row.put(at, if m.primary { "^" } else { "-" }, style);
            if !m.message.is_empty() {
                row.put(at + 2, &m.message, style);
            }
            emit(out, None, &row);
            closed.push(t);
        }
    }
}

/// The lines a fix changes as they read after it: the file, and for each line its number, its
/// text and the changed columns (start, end, whether inserted).
type FixedLines = (SourceId, Vec<(u32, String, Vec<(usize, usize, bool)>)>);

fn fixed_lines(sources: &SourceMap, fix: &Fix) -> Option<FixedLines> {
    let first = fix.edits.first()?;
    let id = first.span.source;
    let mut by_line: Vec<(u32, Vec<&crate::Edit>)> = Vec::new();
    for e in &fix.edits {
        let (l1, _) = sources.line_col(e.span.source, e.span.start);
        let (l2, _) = sources.line_col(e.span.source, e.span.end);
        if e.span.source != id || l1 != l2 || e.text.contains('\n') {
            return None;
        }
        match by_line.iter_mut().find(|(l, _)| *l == l1) {
            Some((_, v)) => v.push(e),
            None => by_line.push((l1, vec![e])),
        }
    }
    by_line.sort_by_key(|(l, _)| *l);
    let mut out = Vec::new();
    for (line, mut edits) in by_line {
        edits.sort_by_key(|e| e.span.start);
        let start = sources.file(id).line_starts[line as usize - 1] as usize;
        let text = sources.line_text(id, line);
        let mut fixed = String::new();
        let mut marks = Vec::new();
        let mut at = 0;
        for e in edits {
            let (s, t) = (e.span.start as usize - start, e.span.end as usize - start);
            if s < at || t > text.len() || !text.is_char_boundary(s) || !text.is_char_boundary(t) {
                return None;
            }
            fixed.push_str(&text[at..s]);
            let col = display_col(&fixed, fixed.len());
            let width = shown(&e.text).chars().count();
            marks.push((col, col + width.max(1), e.span.is_empty()));
            fixed.push_str(&e.text);
            at = t;
        }
        fixed.push_str(&text[at..]);
        out.push((line, shown(&fixed), marks));
    }
    Some((id, out))
}

/// Whether `fix` replaces the primary span of `d` with a short text, so it can be its label.
fn inline_fix(d: &Diagnostic) -> Option<&Fix> {
    let [fix] = d.fixes.as_slice() else {
        return None;
    };
    let [edit] = fix.edits.as_slice() else {
        return None;
    };
    (d.label.is_empty()
        && Some(edit.span) == d.span
        && !edit.span.is_empty()
        && !edit.text.contains('\n')
        && edit.text.chars().count() <= INLINE_FIX)
        .then_some(fix)
}

pub(crate) fn render(d: &Diagnostic, sources: &SourceMap, opts: &RenderOptions) -> String {
    let mut out = Out {
        text: String::new(),
        color: opts.color,
    };
    let level = d.severity;
    let code = match opts.codes && !d.code.is_empty() {
        true => format!("[{}]", d.code),
        false => String::new(),
    };
    out.put(Style::Level(level), &format!("{}{code}", level.as_str()));
    out.put(Style::Message, &format!(": {}", d.message));
    out.line();

    let valid = |s: &Span| sources.contains(s.source);
    let inline = inline_fix(d);
    // Every annotation, grouped by file, the primary's file first.
    let mut files: Vec<(SourceId, Vec<Ann>)> = Vec::new();
    let mut add = |id: SourceId, a: Ann| match files.iter_mut().find(|(f, _)| *f == id) {
        Some((_, v)) => v.push(a),
        None => files.push((id, vec![a])),
    };
    if let Some(span) = d.span.filter(valid) {
        let mut a = annotation(sources, span, true, &d.label);
        if let Some(fix) = inline {
            a.message = format!("help: {}: `{}`", fix.message, fix.edits[0].text);
            a.help = true;
        }
        add(span.source, a);
    }
    for l in d.labels.iter().filter(|l| valid(&l.span)) {
        add(
            l.span.source,
            annotation(sources, l.span, false, &l.message),
        );
    }
    let fixes: Vec<(&Fix, Option<FixedLines>)> = d
        .fixes
        .iter()
        .filter(|f| inline.is_none_or(|i| !std::ptr::eq(*f, i)))
        .map(|f| {
            (
                f,
                fixed_lines(sources, f).filter(|(id, _)| sources.contains(*id)),
            )
        })
        .collect();
    let gw = files
        .iter()
        .flat_map(|(_, anns)| anns.iter().map(|a| a.end_line))
        .chain(
            fixes
                .iter()
                .flat_map(|(_, f)| f.iter().flat_map(|(_, ls)| ls.iter().map(|l| l.0))),
        )
        .max()
        .map_or(1, |n| n.to_string().len());

    for (i, (id, anns)) in files.iter().enumerate() {
        let first = anns.iter().find(|a| a.primary).unwrap_or(&anns[0]);
        let (line, col) = if first.primary {
            sources.line_col(*id, d.span.unwrap().start)
        } else {
            let l = d.labels.iter().find(|l| l.span.source == *id).unwrap();
            sources.line_col(*id, l.span.start)
        };
        out.put(Style::Plain, &" ".repeat(gw));
        out.put(Style::Accent, if i == 0 { "--> " } else { "::: " });
        out.put(Style::Plain, &format!("{}:{line}:{col}", sources.name(*id)));
        out.line();
        out.bar(gw);
        snippet(&mut out, sources, *id, anns, level, gw, opts.width);
        // rustc separates the next file, or the notes, with a bare bar.
        if i + 1 < files.len() {
            out.bar(gw);
        }
    }
    let notes = d
        .notes
        .iter()
        .map(|n| ("note", n))
        .chain(d.helps.iter().map(|h| ("help", h)));
    let mut first_note = true;
    for (kind, text) in notes {
        if first_note && !files.is_empty() {
            out.bar(gw);
        }
        first_note = false;
        out.put(Style::Plain, &" ".repeat(gw + 1));
        out.put(Style::Accent, "=");
        out.put(Style::Message, &format!(" {kind}:"));
        let indent = " ".repeat(gw + kind.len() + 5);
        for (j, part) in text.lines().enumerate() {
            if j > 0 {
                out.line();
                out.put(Style::Plain, &indent);
                out.put(Style::Plain, part);
            } else {
                out.put(Style::Plain, &format!(" {part}"));
            }
        }
        out.line();
    }
    let primary_file = files.first().map(|(id, _)| *id);
    for (fix, lines) in fixes {
        out.put(Style::Help, "help");
        out.put(Style::Message, &format!(": {}", fix.message));
        out.line();
        let Some((id, lines)) = lines else {
            continue;
        };
        if Some(id) != primary_file {
            let (line, col) = sources.line_col(id, fix.edits[0].span.start);
            out.put(Style::Plain, &" ".repeat(gw));
            out.put(Style::Accent, "--> ");
            out.put(Style::Plain, &format!("{}:{line}:{col}", sources.name(id)));
            out.line();
        }
        out.bar(gw);
        for (line, text, marks) in lines {
            let mut row = Row::default();
            row.put(0, &text, Style::Plain);
            for &(s, e, _) in &marks {
                let inserted: String = text.chars().skip(s).take(e - s).collect();
                row.put(s, &inserted, Style::Insert);
            }
            out.put(Style::Accent, &format!("{line:>gw$} |"));
            out.put(Style::Plain, " ");
            row.emit(&mut out);
            out.line();
            let mut row = Row::default();
            for &(s, e, insert) in &marks {
                row.put(
                    s,
                    &(if insert { "+" } else { "~" }).repeat(e - s),
                    Style::Help,
                );
            }
            out.put(Style::Plain, &" ".repeat(gw + 1));
            out.put(Style::Accent, "|");
            out.put(Style::Plain, " ");
            row.emit(&mut out);
            out.line();
        }
    }
    out.text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Diagnostic, SourceMap, Span};

    fn sm(text: &str) -> (SourceMap, SourceId) {
        let mut sm = SourceMap::new();
        let id = sm.add("main.mn", text);
        (sm, id)
    }

    fn at(text: &str, id: SourceId, needle: &str) -> Span {
        let s = text.find(needle).unwrap() as u32;
        Span::new(id, s, s + needle.len() as u32)
    }

    #[test]
    fn labels_in_one_file_share_a_snippet() {
        let text = "fn f(x: int) -> int {\n  let y: int = 1;\n  y = \"a\";\n  x\n}\n";
        let (sm, id) = sm(text);
        let int = at(text, id, "int = 1");
        let d = Diagnostic::error("E0308", "mismatched types")
            .with_span(at(text, id, "\"a\""))
            .with_primary_label("expected `int`, found `string`")
            .with_label(
                Span::new(id, int.start, int.start + 3),
                "expected because of this",
            );
        assert_eq!(
            d.render(&sm),
            "error[E0308]: mismatched types
 --> main.mn:3:7
  |
2 |   let y: int = 1;
  |          --- expected because of this
3 |   y = \"a\";
  |       ^^^ expected `int`, found `string`
"
        );
    }

    #[test]
    fn labels_on_one_line_hang_below_it() {
        let text = "print(add(1, \"two\"))\n";
        let (sm, id) = sm(text);
        let d = Diagnostic::error("E0308", "mismatched types")
            .with_span(at(text, id, "\"two\""))
            .with_primary_label("expected `int`")
            .with_label(
                at(text, id, "add"),
                "arguments to this function are incorrect",
            );
        assert_eq!(
            d.render(&sm),
            "error[E0308]: mismatched types
 --> main.mn:1:14
  |
1 | print(add(1, \"two\"))
  |       ---    ^^^^^ expected `int`
  |       |
  |       arguments to this function are incorrect
"
        );
    }

    #[test]
    fn multi_line_spans_have_a_track() {
        let text = "fn main() {\n  let x = if c {\n    1\n  } else {\n    \"a\"\n  };\n}\n";
        let (sm, id) = sm(text);
        let start = text.find("if c").unwrap() as u32;
        let end = text.find("};").unwrap() as u32 + 1;
        let d = Diagnostic::error("E0308", "`if` and `else` have incompatible types")
            .with_span(at(text, id, "\"a\""))
            .with_primary_label("expected `int`, found `string`")
            .with_label(Span::new(id, start, end), "`if` and `else` differ")
            .with_note("both branches must have the same type");
        assert_eq!(
            d.render(&sm),
            "error[E0308]: `if` and `else` have incompatible types
 --> main.mn:5:5
  |
2 |     let x = if c {
  |  ___________-
3 | |     1
4 | |   } else {
5 | |     \"a\"
  | |     ^^^ expected `int`, found `string`
6 | |   };
  | |___- `if` and `else` differ
  |
  = note: both branches must have the same type
"
        );
    }

    #[test]
    fn a_span_from_the_start_of_its_code_begins_with_a_slash() {
        let text = "a\n  {\n    b\n  }\nz\n";
        let (sm, id) = sm(text);
        let s = text.find('{').unwrap() as u32;
        let e = text.rfind('}').unwrap() as u32 + 1;
        let d = Diagnostic::error("E1", "block").with_span(Span::new(id, s, e));
        assert_eq!(
            d.render(&sm),
            "error[E1]: block
 --> main.mn:2:3
  |
2 | /   {
3 | |     b
4 | |   }
  | |___^
"
        );
    }

    #[test]
    fn distant_lines_are_elided_and_long_spans_shortened() {
        let mut text = String::from("start\n");
        for i in 0..20 {
            text += &format!("line {i}\n");
        }
        text += "end\n";
        let (sm, id) = sm(&text);
        let d = Diagnostic::error("E1", "far apart")
            .with_span(at(&text, id, "end"))
            .with_label(at(&text, id, "start"), "here");
        let r = d.render(&sm);
        assert!(
            r.contains(" 1 | start\n   | ----- here\n...\n22 | end\n   | ^^^\n"),
            "{r}"
        );
    }

    #[test]
    fn labels_in_other_files_get_their_own_snippet() {
        let mut sm = SourceMap::new();
        let a = sm.add("a.mn", "f(1)\n");
        let b = sm.add("lib.mn", "fn f(s: string) {}\n");
        let d = Diagnostic::error("E0308", "mismatched types")
            .with_span(Span::new(a, 2, 3))
            .with_label(Span::new(b, 5, 14), "parameter declared here");
        assert_eq!(
            d.render(&sm),
            "error[E0308]: mismatched types
 --> a.mn:1:3
  |
1 | f(1)
  |   ^
  |
 ::: lib.mn:1:6
  |
1 | fn f(s: string) {}
  |      --------- parameter declared here
"
        );
    }

    #[test]
    fn short_fixes_of_the_primary_span_are_its_label() {
        let text = "let count = 1;\nprint(cuont);\n";
        let (sm, id) = sm(text);
        let span = at(text, id, "cuont");
        let d = Diagnostic::error("E0425", "unknown name `cuont`")
            .with_span(span)
            .with_fix("a local with a similar name exists", span, "count");
        assert_eq!(
            d.render(&sm),
            "error[E0425]: unknown name `cuont`
 --> main.mn:2:7
  |
2 | print(cuont);
  |       ^^^^^ help: a local with a similar name exists: `count`
"
        );
    }

    #[test]
    fn other_fixes_show_the_corrected_line() {
        let text = "let x = f(1);\n";
        let (sm, id) = sm(text);
        let d = Diagnostic::error("E0061", "this function takes 2 arguments")
            .with_span(at(text, id, "f(1)"))
            .with_primary_label("1 argument given")
            .with_fix("add the missing argument", Span::new(id, 11, 11), ", y");
        assert_eq!(
            d.render(&sm),
            "error[E0061]: this function takes 2 arguments
 --> main.mn:1:9
  |
1 | let x = f(1);
  |         ^^^^ 1 argument given
help: add the missing argument
  |
1 | let x = f(1, y);
  |            +++
"
        );
    }

    #[test]
    fn long_lines_are_cut_around_the_span() {
        let text = format!("{}target{}\n", "a".repeat(300), "b".repeat(300));
        let (sm, id) = sm(&text);
        let d = Diagnostic::error("E1", "here").with_span(at(&text, id, "target"));
        let r = d.render(&sm);
        let line = r.lines().nth(3).unwrap();
        assert!(line.starts_with("1 | ...") && line.ends_with("..."), "{r}");
        assert!(line.len() <= 140, "{}", line.len());
        let marks = r.lines().nth(4).unwrap();
        assert_eq!(marks.find('^'), line.find("target"), "{r}");
    }

    #[test]
    fn colors_and_hidden_codes() {
        let (sm, id) = sm("x\n");
        let d = Diagnostic::warning("unused-variable", "unused variable `x`")
            .with_span(Span::new(id, 0, 1));
        let plain = d.render_with(
            &sm,
            &RenderOptions {
                codes: false,
                ..RenderOptions::default()
            },
        );
        assert!(
            plain.starts_with("warning: unused variable `x`\n"),
            "{plain}"
        );
        let colored = d.render_with(
            &sm,
            &RenderOptions {
                color: true,
                ..RenderOptions::default()
            },
        );
        assert!(
            colored.starts_with(
                "\x1b[1;93mwarning[unused-variable]\x1b[0m\x1b[1m: unused variable `x`\x1b[0m\n"
            ),
            "{colored:?}"
        );
        assert!(colored.contains("\x1b[1;93m^\x1b[0m"), "{colored:?}");
    }

    #[test]
    fn tabs_are_expanded() {
        let text = "\tx = 1\n";
        let (sm, id) = sm(text);
        let d = Diagnostic::error("E1", "tab").with_span(at(text, id, "x"));
        assert_eq!(
            d.render(&sm),
            "error[E1]: tab\n --> main.mn:1:2\n  |\n1 |     x = 1\n  |     ^\n"
        );
    }

    #[test]
    fn spans_at_the_end_of_the_text_point_after_it() {
        let text = "let x =";
        let (sm, id) = sm(text);
        let d = Diagnostic::error("E1", "expected an expression")
            .with_span(Span::new(id, 7, 7))
            .with_primary_label("expected an expression");
        assert_eq!(
            d.render(&sm),
            "error[E1]: expected an expression\n --> main.mn:1:8\n  |\n1 | let x =\n  |        ^ expected an expression\n"
        );
    }

    #[test]
    fn spans_through_a_character_draw_all_of_it() {
        // A language's span may end, or start, inside a character (found by fuzzing).
        let text = "x = \u{e9}\u{e9};\n";
        let (sm, id) = sm(text);
        for (start, end) in [(4, 5), (5, 6), (5, 7), (4, 7)] {
            let d = Diagnostic::error("E1", "here").with_span(Span::new(id, start, end));
            let caret = if end > 6 { "^^" } else { "^" };
            assert_eq!(
                d.render(&sm),
                format!(
                    "error[E1]: here\n --> main.mn:1:5\n  |\n1 | x = \u{e9}\u{e9};\n  |     {caret}\n"
                ),
                "{start}..{end}"
            );
        }
    }
}
