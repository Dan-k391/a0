//! Lexing and parsing with a compiled [`Grammar`].
//!
//! Parsing is ordered choice with backtracking (PEG). When parsing fails, the error is reported at
//! the furthest token any alternative reached. Among the things expected there, the first custom
//! message (`X ~ "message"`) wins; otherwise the message lists the expected tokens. Rules marked
//! `@recover(...)` never fail: they report the error, skip tokens in panic mode and produce an
//! `error` node, so one file can report several errors.

use std::sync::Arc;

use a0_diag::codes as code;
use a0_diag::{Diagnostic, Diagnostics, SourceId, Span};

use crate::grammar::{Assoc, Expr, ExprId, FieldId, Grammar, KindId, LexAction, RuleId};
use crate::tree::{NodeData, NodeId, SyntaxTree};

/// How deep a tree may nest, and how deep the parser recurses through rules (D83): every pass over
/// a tree recurses as deep as it nests, so a program nested deeper gets a diagnostic, not a
/// stack overflow. Real programs nest a few dozen levels.
pub const MAX_NESTING: usize = 10_000;

/// A token produced by the lexer. `start..end` are byte offsets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub kind: KindId,
    pub start: u32,
    pub end: u32,
}

impl Grammar {
    /// Splits `text` into tokens. Skipped text is dropped; error patterns and unknown characters are
    /// reported and dropped. The last token is always [`KindId::EOF`].
    pub fn lex(&self, text: &str, source: SourceId) -> (Vec<Token>, Diagnostics) {
        self.lex_keeping(text, source, None)
    }

    /// Like [`Grammar::lex`], also collecting the byte ranges of skipped text (whitespace,
    /// comments) into `skipped`.
    pub(crate) fn lex_keeping(
        &self,
        text: &str,
        source: SourceId,
        mut skipped: Option<&mut Vec<(u32, u32)>>,
    ) -> (Vec<Token>, Diagnostics) {
        let mut toks = Vec::new();
        let mut diags = Diagnostics::new();
        let mut pos = 0usize;
        while pos < text.len() {
            match self.matcher.longest_match(text, pos) {
                Some((end, id)) => {
                    match &self.actions[id as usize] {
                        LexAction::Token(kind) => toks.push(Token {
                            kind: *kind,
                            start: pos as u32,
                            end: end as u32,
                        }),
                        LexAction::Skip => {
                            if let Some(skipped) = skipped.as_deref_mut() {
                                skipped.push((pos as u32, end as u32));
                            }
                        }
                        LexAction::Error(msg) => diags.push(
                            Diagnostic::error(code::INVALID_TOKEN, msg)
                                .with_span(Span::new(source, pos as u32, end as u32)),
                        ),
                    }
                    pos = end;
                }
                None => {
                    let len = text[pos..].chars().next().map(char::len_utf8).unwrap_or(1);
                    diags.push(
                        Diagnostic::error(code::INVALID_TOKEN, &self.unknown_message)
                            .with_span(Span::new(source, pos as u32, (pos + len) as u32)),
                    );
                    pos += len;
                }
            }
        }
        toks.push(Token {
            kind: KindId::EOF,
            start: text.len() as u32,
            end: text.len() as u32,
        });
        (toks, diags)
    }

    /// Parses `text` with the grammar's first rule. Always returns a tree; syntax errors are reported
    /// as diagnostics and appear in the tree as `error` nodes.
    pub fn parse(&self, text: &str, source: SourceId) -> (SyntaxTree, Diagnostics) {
        let (toks, diags) = self.lex(text, source);
        let mut p = Parser {
            g: self,
            text,
            toks,
            pos: 0,
            source,
            nodes: Vec::new(),
            children: Vec::new(),
            pending: Vec::new(),
            furthest: 0,
            expected: Vec::new(),
            suppress: 0,
            diags,
            depth: 0,
            too_deep: None,
        };
        let ok = p.rule(self.start);
        let too_deep = match p.too_deep {
            Some(at) => Some(at),
            None if ok => p.deepest_node(),
            None => None,
        };
        if let Some(at) = too_deep {
            // What the parser reported while giving up is not the program's problem.
            let mut kept = Diagnostics::new();
            for d in p.diags.as_slice() {
                if d.code != code::SYNTAX {
                    kept.push(d.clone());
                }
            }
            p.diags = kept;
            p.diags.push(
                Diagnostic::error(
                    code::TOO_DEEP,
                    &format!("this is nested more than {MAX_NESTING} levels deep"),
                )
                .with_span(Span::new(source, at.0, at.1))
                .with_primary_label("too deep")
                .with_help("name intermediate results, or move inner parts into functions"),
            );
        }
        let root = if too_deep.is_none() && ok && p.at_eof() && p.pending.len() == 1 {
            p.pending.pop().unwrap().0
        } else {
            if too_deep.is_none() {
                if ok {
                    p.fail_token(p.pos, KindId::EOF);
                }
                p.report_furthest();
            }
            // Keep every token under one error node.
            p.pending.clear();
            p.pos = 0;
            while !p.at_eof() {
                p.push_token();
            }
            p.wrap(KindId::ERROR, 0);
            p.pending.pop().unwrap().0
        };
        let tree = SyntaxTree {
            text: text.to_string(),
            source,
            nodes: p.nodes,
            children: p.children,
            root,
            names: Arc::clone(&self.names),
        };
        (tree, p.diags)
    }
}

/// One thing that would have been accepted at the furthest failure position.
struct Expected<'g> {
    token: Option<KindId>,
    message: Option<&'g str>,
}

#[derive(Clone, Copy)]
struct Mark {
    pos: usize,
    nodes: usize,
    children: usize,
    pending: usize,
}

struct Parser<'g> {
    g: &'g Grammar,
    text: &'g str,
    toks: Vec<Token>,
    pos: usize,
    source: SourceId,
    nodes: Vec<NodeData>,
    children: Vec<(NodeId, FieldId)>,
    /// Children of the nodes currently being built, innermost last.
    pending: Vec<(NodeId, FieldId)>,
    furthest: usize,
    expected: Vec<Expected<'g>>,
    /// Inside a lookahead: failures are not recorded as expectations.
    suppress: u32,
    diags: Diagnostics,
    /// How many rules are being parsed, one inside another.
    depth: usize,
    /// Where the input nested too deeply, if it did: parsing then gives up.
    too_deep: Option<(u32, u32)>,
}

impl<'g> Parser<'g> {
    fn kind(&self) -> KindId {
        self.toks[self.pos].kind
    }

    fn at_eof(&self) -> bool {
        self.kind() == KindId::EOF
    }

    fn mark(&self) -> Mark {
        Mark {
            pos: self.pos,
            nodes: self.nodes.len(),
            children: self.children.len(),
            pending: self.pending.len(),
        }
    }

    fn reset(&mut self, m: Mark) {
        self.pos = m.pos;
        self.nodes.truncate(m.nodes);
        self.children.truncate(m.children);
        self.pending.truncate(m.pending);
    }

    fn push_token(&mut self) {
        let t = self.toks[self.pos];
        let id = NodeId(self.nodes.len() as u32);
        self.nodes.push(NodeData {
            kind: t.kind,
            start: t.start,
            end: t.end,
            children_start: 0,
            children_len: 0,
            is_token: true,
        });
        self.pending.push((id, FieldId::NONE));
        self.pos += 1;
    }

    /// Turns `pending[from..]` into the children of a new node, which takes their place.
    fn wrap(&mut self, kind: KindId, from: usize) {
        let kids: Vec<(NodeId, FieldId)> = self.pending.drain(from..).collect();
        let (start, end) = match (kids.first(), kids.last()) {
            (Some(first), Some(last)) => (
                self.nodes[first.0.0 as usize].start,
                self.nodes[last.0.0 as usize].end,
            ),
            _ => {
                let s = self.toks[self.pos].start;
                (s, s)
            }
        };
        let children_start = self.children.len() as u32;
        let children_len = kids.len() as u32;
        self.children.extend(kids);
        let id = NodeId(self.nodes.len() as u32);
        self.nodes.push(NodeData {
            kind,
            start,
            end,
            children_start,
            children_len,
            is_token: false,
        });
        self.pending.push((id, FieldId::NONE));
    }

    fn label_from(&mut self, from: usize, field: FieldId) {
        for entry in &mut self.pending[from..] {
            entry.1 = field;
        }
    }

    fn label_last(&mut self, field: FieldId) {
        if let Some(last) = self.pending.last_mut() {
            last.1 = field;
        }
    }

    // ---------------------------------------------------------------- failure tracking

    fn note(&mut self, pos: usize) -> bool {
        if self.suppress > 0 {
            return false;
        }
        if pos > self.furthest {
            self.furthest = pos;
            self.expected.clear();
        }
        pos == self.furthest
    }

    fn fail_token(&mut self, pos: usize, kind: KindId) {
        if self.note(pos) {
            self.expected.push(Expected {
                token: Some(kind),
                message: None,
            });
        }
    }

    fn describe_kind(&self, kind: KindId) -> String {
        if kind == KindId::EOF {
            return "end of input".to_string();
        }
        match &self.g.names.literal_text[kind.0 as usize] {
            Some(text) => format!("'{text}'"),
            None => self.g.names.kinds[kind.0 as usize].clone(),
        }
    }

    /// The span of the first node nested deeper than [`MAX_NESTING`] in the parsed tree, whose
    /// nodes come before their parents (operators chained in a loop nest without recursing).
    fn deepest_node(&self) -> Option<(u32, u32)> {
        let mut depth = vec![0usize; self.nodes.len()];
        for (i, node) in self.nodes.iter().enumerate().rev() {
            let children = &self.children
                [node.children_start as usize..(node.children_start + node.children_len) as usize];
            for &(child, _) in children {
                depth[child.0 as usize] = depth[i] + 1;
                if depth[i] + 1 > MAX_NESTING {
                    let c = &self.nodes[child.0 as usize];
                    return Some((c.start, c.end));
                }
            }
        }
        None
    }

    fn report_furthest(&mut self) {
        let tok = self.toks[self.furthest];
        let span = Span::new(self.source, tok.start, tok.end);
        let found = if tok.kind == KindId::EOF {
            "end of input".to_string()
        } else {
            format!("'{}'", &self.text[tok.start as usize..tok.end as usize])
        };
        let diag = match self.expected.iter().find_map(|e| e.message) {
            Some(m) => Diagnostic::error(code::SYNTAX, m),
            None => {
                let mut labels: Vec<String> = Vec::new();
                for e in &self.expected {
                    if let Some(k) = e.token {
                        let d = self.describe_kind(k);
                        if !labels.contains(&d) {
                            labels.push(d);
                        }
                    }
                }
                let msg = match labels.len() {
                    0 => format!("unexpected {found}"),
                    1 => format!("expected {}, found {found}", labels[0]),
                    _ => {
                        let last = labels.pop().unwrap();
                        format!("expected {} or {last}, found {found}", labels.join(", "))
                    }
                };
                Diagnostic::error(code::SYNTAX, &msg)
            }
        };
        self.diags.push(diag.with_span(span));
    }

    // ---------------------------------------------------------------- rules and expressions

    fn rule(&mut self, r: RuleId) -> bool {
        if self.too_deep.is_some() {
            return false;
        }
        if self.depth >= MAX_NESTING {
            let tok = self.toks[self.pos];
            self.too_deep = Some((tok.start, tok.end));
            return false;
        }
        self.depth += 1;
        let ok = self.rule_body(r);
        self.depth -= 1;
        ok
    }

    fn rule_body(&mut self, r: RuleId) -> bool {
        let rule = &self.g.rules[r];
        if rule.recover.is_some() {
            return self.recovering_rule(r);
        }
        let mark = self.mark();
        if !self.expr(rule.body) {
            self.reset(mark);
            return false;
        }
        if let Some(kind) = rule.kind {
            self.wrap(kind, mark.pending);
        }
        true
    }

    fn recovering_rule(&mut self, r: RuleId) -> bool {
        let g = self.g;
        let rule = &g.rules[r];
        let recover = rule.recover.as_ref().unwrap();
        let mark = self.mark();
        // The furthest failure is deliberately not reset here: if an earlier construct backtracked out
        // of a branch it had committed to (an `else` whose statement is broken), the failure inside that
        // branch is further along and is the error a hand-written parser would report.
        if self.expr(rule.body) {
            if let Some(kind) = rule.kind {
                self.wrap(kind, mark.pending);
            }
            return true;
        }

        self.report_furthest();
        let error_pos = self.furthest.max(mark.pos);
        self.reset(mark);
        // Panic mode: skip the offending token, then stop after an `after` token or before a `before`
        // token (or at the end of input).
        self.pos = error_pos;
        if !self.at_eof() {
            self.pos += 1;
            while !self.at_eof() {
                let prev = self.toks[self.pos - 1].kind;
                if recover.after.contains(&prev) || recover.before.contains(&self.kind()) {
                    break;
                }
                self.pos += 1;
            }
        }
        let end = self.pos;
        self.pos = mark.pos;
        while self.pos < end {
            self.push_token();
        }
        self.wrap(KindId::ERROR, mark.pending);
        self.furthest = self.pos;
        self.expected.clear();
        true
    }

    fn expr(&mut self, e: ExprId) -> bool {
        let g = self.g;
        match &g.exprs[e] {
            Expr::Token(k) => {
                if self.kind() == *k {
                    self.push_token();
                    true
                } else {
                    self.fail_token(self.pos, *k);
                    false
                }
            }
            Expr::Eof => {
                if self.at_eof() {
                    true
                } else {
                    self.fail_token(self.pos, KindId::EOF);
                    false
                }
            }
            Expr::Rule(r) => self.rule(*r),
            Expr::Seq(items) => items.iter().all(|&i| self.expr(i)),
            Expr::Alt(options) => {
                for &o in options {
                    let mark = self.mark();
                    if self.expr(o) {
                        return true;
                    }
                    self.reset(mark);
                }
                false
            }
            &Expr::Repeat {
                expr,
                min,
                at_most_once,
            } => {
                let mut count = 0;
                loop {
                    if at_most_once && count == 1 {
                        break;
                    }
                    let mark = self.mark();
                    if !self.expr(expr) {
                        self.reset(mark);
                        break;
                    }
                    count += 1;
                    if self.pos == mark.pos {
                        break;
                    }
                }
                count >= min
            }
            Expr::Not(inner) | Expr::And(inner) => {
                let mark = self.mark();
                self.suppress += 1;
                let ok = self.expr(*inner);
                self.suppress -= 1;
                self.reset(mark);
                if matches!(g.exprs[e], Expr::Not(_)) {
                    !ok
                } else {
                    ok
                }
            }
            Expr::Field(field, inner) => {
                let from = self.pending.len();
                let ok = self.expr(*inner);
                if ok {
                    self.label_from(from, *field);
                }
                ok
            }
            Expr::Expect(inner, message) => {
                let start = self.pos;
                let before = if self.furthest == start {
                    self.expected.len()
                } else {
                    0
                };
                let ok = self.expr(*inner);
                if !ok && self.suppress == 0 {
                    if self.furthest < start {
                        self.furthest = start;
                        self.expected.clear();
                    }
                    if self.furthest == start {
                        // Prefer this message over anything `inner` itself expected here, but keep
                        // messages recorded earlier by siblings first.
                        let at = before.min(self.expected.len());
                        self.expected.insert(
                            at,
                            Expected {
                                token: None,
                                message: Some(message.as_str()),
                            },
                        );
                    }
                }
                ok
            }
            Expr::Prec(p) => self.prec(*p, 1),
        }
    }

    /// Pratt parsing of an operator expression whose operators bind at `min_level` or tighter.
    fn prec(&mut self, p: usize, min_level: u32) -> bool {
        let g = self.g;
        let prec = &g.precs[p];
        let std = g.std_fields;
        let start = self.pending.len();
        if let Some(&(level, node_kind)) = prec.prefix.get(&self.kind()) {
            self.push_token();
            self.label_last(std.op);
            if !self.prec(p, level) {
                return false;
            }
            self.label_last(std.operand);
            self.wrap(node_kind, start);
        } else if !self.expr(prec.atom) {
            return false;
        }
        loop {
            let k = self.kind();
            if let Some(&(level, assoc, node_kind)) = prec.binary.get(&k) {
                if level < min_level {
                    break;
                }
                let left = self.pending.len() - 1;
                self.label_last(std.left);
                self.push_token();
                self.label_last(std.op);
                let next = if assoc == Assoc::Left {
                    level + 1
                } else {
                    level
                };
                if !self.prec(p, next) {
                    return false;
                }
                self.label_last(std.right);
                self.wrap(node_kind, left);
            } else if let Some(&(level, rest, node_kind)) = prec.postfix.get(&k) {
                if level < min_level {
                    break;
                }
                let operand = self.pending.len() - 1;
                self.label_last(std.operand);
                self.push_token();
                if !self.expr(rest) {
                    return false;
                }
                self.wrap(node_kind, operand);
            } else {
                break;
            }
        }
        true
    }
}
