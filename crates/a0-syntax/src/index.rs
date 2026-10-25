//! Names in a syntax tree, from the grammar's `@def` and `@scope` annotations: what each token is
//! (for highlighting), the symbols a file declares (for an outline) and what each name refers to
//! (for go-to-definition). Editor support for a language needs nothing else from it.
//!
//! Resolution is lexical. A name refers to the closest declaration before it in the innermost scope
//! that declares the name; declarations that are visible in their whole scope (functions, types,
//! members, see [`SymbolKind::is_hoisted`]) and anything declared at file level also count when they
//! come later. A name after `.` or `::` refers to every member (method, field, variant) of that name,
//! since which one depends on types the grammar does not know.

use std::collections::HashMap;

use a0_diag::{SourceId, Span};

use crate::grammar::{FieldId, Grammar, KindId, SymbolKind};
use crate::tree::{NodeId, SyntaxTree};

/// What a token is, for highlighting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum TokenClass {
    /// A literal token that looks like a word (`while`, `fun`).
    Keyword,
    /// Any other literal token (`+`, `{`), or a named token that matches operator characters.
    Operator,
    Number,
    String,
    /// Skipped text other than whitespace.
    Comment,
    /// The token the grammar uses for identifiers.
    Name,
    Other,
}

/// A token of an indexed file. `start..end` are byte offsets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexedToken {
    pub start: u32,
    pub end: u32,
    pub class: TokenClass,
    /// The symbol whose name this token is, if it is one.
    pub declares: Option<u32>,
    /// Whether the token follows `.` or `::`.
    pub member: bool,
}

struct Symbol {
    name: String,
    kind: SymbolKind,
    name_span: Span,
    extent: Span,
    parent: Option<u32>,
    scope: Option<NodeId>,
    /// The text of the scope it is declared in (`None`: the file's).
    scope_span: Option<Span>,
}

/// The tokens, symbols and references of one file. Symbols are numbered in source order.
pub struct SyntaxIndex {
    tokens: Vec<IndexedToken>,
    targets: Vec<Vec<u32>>,
    symbols: Vec<Symbol>,
}

impl SyntaxIndex {
    /// Indexes `tree`, which `grammar` parsed. Works on trees with syntax errors too.
    pub fn new(grammar: &Grammar, tree: &SyntaxTree) -> SyntaxIndex {
        let text = tree.source_text();
        let classes = token_classes(grammar);
        let mut skipped = Vec::new();
        let (lexed, _) = grammar.lex_keeping(text, tree.source(), Some(&mut skipped));
        let mut tokens: Vec<IndexedToken> = lexed
            .iter()
            .filter(|t| t.kind != KindId::EOF)
            .map(|t| IndexedToken {
                start: t.start,
                end: t.end,
                class: classes[t.kind.0 as usize],
                declares: None,
                member: false,
            })
            .collect();
        let mut previous: Option<&str> = None;
        for t in &mut tokens {
            t.member = matches!(previous, Some("." | "::"));
            previous = Some(&text[t.start as usize..t.end as usize]);
        }
        for (start, end) in skipped {
            if !text[start as usize..end as usize].trim().is_empty() {
                tokens.push(IndexedToken {
                    start,
                    end,
                    class: TokenClass::Comment,
                    declares: None,
                    member: false,
                });
            }
        }
        tokens.sort_by_key(|t| t.start);

        let mut b = Indexer {
            grammar,
            tree,
            parent: vec![NodeId::NONE; tree.nodes.len()],
            symbol_of: HashMap::new(),
            symbols: Vec::new(),
            leaves: HashMap::new(),
        };
        b.walk();
        let declared: HashMap<u32, u32> = b
            .symbols
            .iter()
            .enumerate()
            .map(|(i, s)| (s.name_span.start, i as u32))
            .collect();
        let mut by_name: HashMap<(Option<NodeId>, &str), Vec<u32>> = HashMap::new();
        for (i, s) in b.symbols.iter().enumerate() {
            by_name
                .entry((s.scope, s.name.as_str()))
                .or_default()
                .push(i as u32);
        }
        let mut targets = vec![Vec::new(); tokens.len()];
        for (i, t) in tokens.iter_mut().enumerate() {
            if t.class != TokenClass::Name {
                continue;
            }
            if let Some(&s) = declared.get(&t.start) {
                t.declares = Some(s);
                targets[i] = vec![s];
                continue;
            }
            let name = &text[t.start as usize..t.end as usize];
            targets[i] = if t.member {
                (0..b.symbols.len() as u32)
                    .filter(|&s| {
                        let s = &b.symbols[s as usize];
                        s.kind.is_member() && s.name == name
                    })
                    .collect()
            } else {
                b.resolve(name, t.start, &by_name)
            };
        }
        SyntaxIndex {
            tokens,
            targets,
            symbols: b.symbols,
        }
    }

    pub fn token_count(&self) -> u32 {
        self.tokens.len() as u32
    }

    pub fn token(&self, i: u32) -> IndexedToken {
        self.tokens[i as usize]
    }

    /// The token at byte `offset`. A name ending there wins over a token starting there, since a
    /// cursor right after a name (`len|;`) is on the name.
    pub fn token_at(&self, offset: u32) -> Option<u32> {
        let i = self.tokens.partition_point(|t| t.end <= offset);
        let here = self
            .tokens
            .get(i)
            .filter(|t| t.start <= offset)
            .map(|_| i as u32);
        let before = i
            .checked_sub(1)
            .filter(|&b| self.tokens[b].end == offset)
            .map(|b| b as u32);
        match (here, before) {
            (Some(h), Some(b))
                if self.token(h).class != TokenClass::Name
                    && self.token(b).class == TokenClass::Name =>
            {
                Some(b)
            }
            (Some(h), _) => Some(h),
            (None, b) => b,
        }
    }

    /// The symbols a token names: its own symbol for a declaration, what a reference resolves to
    /// (several for a member name), or nothing.
    pub fn targets(&self, token: u32) -> &[u32] {
        &self.targets[token as usize]
    }

    pub fn symbol_count(&self) -> u32 {
        self.symbols.len() as u32
    }

    pub fn symbol_name(&self, s: u32) -> &str {
        &self.symbols[s as usize].name
    }

    pub fn symbol_kind(&self, s: u32) -> SymbolKind {
        self.symbols[s as usize].kind
    }

    /// The symbol's name in its declaration.
    pub fn symbol_span(&self, s: u32) -> Span {
        self.symbols[s as usize].name_span
    }

    /// The whole declaration (the `@def` node; the name itself for `@def(kind: field)`).
    pub fn symbol_extent(&self, s: u32) -> Span {
        self.symbols[s as usize].extent
    }

    /// The declaration this one is inside (the class of a method, the function of a parameter).
    pub fn symbol_parent(&self, s: u32) -> Option<u32> {
        self.symbols[s as usize].parent
    }

    /// Whether the symbol is declared in the file's scope rather than in an `@scope` node.
    pub fn symbol_is_global(&self, s: u32) -> bool {
        self.symbols[s as usize].scope.is_none()
    }

    /// The symbols a name at `offset` could refer to, for completion: those of the scopes around
    /// it, declared before it unless they are hoisted (functions, types), members excepted; of
    /// several with one name, the innermost.
    pub fn visible_at(&self, offset: u32) -> Vec<u32> {
        let mut best: HashMap<&str, (u32, u32)> = HashMap::new();
        for (i, s) in self.symbols.iter().enumerate() {
            if s.kind.is_member() {
                continue;
            }
            let width = match s.scope_span {
                None => u32::MAX,
                Some(span) if span.start <= offset && offset <= span.end => span.end - span.start,
                Some(_) => continue,
            };
            let declared = s.name_span.start <= offset
                && !(s.extent.start <= offset && offset < s.extent.end && !s.kind.is_hoisted());
            if !declared && !(s.kind.is_hoisted() && s.scope.is_none()) {
                continue;
            }
            let entry = best.entry(s.name.as_str()).or_insert((i as u32, width));
            if width < entry.1
                || (width == entry.1
                    && s.name_span.start > self.symbols[entry.0 as usize].name_span.start
                    && s.name_span.start <= offset)
            {
                *entry = (i as u32, width);
            }
        }
        let mut out: Vec<u32> = best.into_values().map(|(s, _)| s).collect();
        out.sort();
        out
    }
}

/// Classifies every token kind. Named tokens are classified by what they match: the grammar's own
/// lexer is run on a few sample words, numbers, strings and operators.
fn token_classes(grammar: &Grammar) -> Vec<TokenClass> {
    let mut classes = vec![TokenClass::Other; grammar.kind_count() as usize];
    for k in 0..grammar.kind_count() {
        let kind = KindId(k);
        if let Some(text) = grammar.literal_text(kind) {
            let word = text.chars().all(|c| c.is_alphanumeric() || c == '_')
                && text.chars().next().is_some_and(|c| !c.is_ascii_digit());
            classes[k as usize] = if word {
                TokenClass::Keyword
            } else {
                TokenClass::Operator
            };
        }
    }
    let samples: [(TokenClass, &[&str]); 4] = [
        (TokenClass::Name, &["x", "abc", "foo_bar", "x1", "X"]),
        (TokenClass::Number, &["0", "42", "1.5"]),
        (TokenClass::String, &["\"s\"", "\"\"", "'s'"]),
        (TokenClass::Operator, &["+", "-", "*", "<", "="]),
    ];
    for (class, texts) in samples {
        for text in texts {
            let (toks, diags) = grammar.lex(text, SourceId(u32::MAX));
            if let [t, eof] = toks.as_slice()
                && diags.is_empty()
                && eof.kind == KindId::EOF
                && t.end as usize == text.len()
                && grammar.literal_text(t.kind).is_none()
                && classes[t.kind.0 as usize] == TokenClass::Other
            {
                classes[t.kind.0 as usize] = class;
            }
        }
    }
    classes
}

struct Indexer<'a> {
    grammar: &'a Grammar,
    tree: &'a SyntaxTree,
    parent: Vec<NodeId>,
    /// The symbol each `@def(kind)` node declares.
    symbol_of: HashMap<NodeId, u32>,
    symbols: Vec<Symbol>,
    /// Token leaves by start offset.
    leaves: HashMap<u32, NodeId>,
}

impl Indexer<'_> {
    /// Visits the tree in source order, recording parents, leaves and declarations.
    fn walk(&mut self) {
        let tree = self.tree;
        let mut stack = vec![tree.root()];
        while let Some(n) = stack.pop() {
            if tree.is_token(n) {
                self.leaves.insert(tree.span(n).start, n);
                continue;
            }
            self.declare(n);
            let children = tree.children(n);
            for &c in &children {
                self.parent[c.0 as usize] = n;
            }
            stack.extend(children.into_iter().rev());
        }
    }

    fn declare(&mut self, n: NodeId) {
        let tree = self.tree;
        let Some(def) = self.grammar.definition(tree.kind(n)) else {
            return;
        };
        let parent = self.enclosing_symbol(n);
        if def.each {
            // The names are visible in the innermost scope around them, this node included.
            let scope = if self.grammar.is_scope(tree.kind(n)) {
                Some(n)
            } else {
                self.scope_above(n)
            };
            for t in tree.fields(n, def.field) {
                if tree.is_token(t) {
                    self.push(t, tree.span(t), def.kind, parent, scope);
                }
            }
        } else if let Some(t) = self.name_token(n, def.field) {
            let scope = self.scope_above(n);
            let s = self.push(t, tree.span(n), def.kind, parent, scope);
            self.symbol_of.insert(n, s);
        }
    }

    fn push(
        &mut self,
        name: NodeId,
        extent: Span,
        kind: SymbolKind,
        parent: Option<u32>,
        scope: Option<NodeId>,
    ) -> u32 {
        self.symbols.push(Symbol {
            name: self.tree.text(name).to_string(),
            kind,
            name_span: self.tree.span(name),
            extent,
            parent,
            scope,
            scope_span: scope.map(|s| self.tree.span(s)),
        });
        self.symbols.len() as u32 - 1
    }

    /// The first token in `field`, among the children of `n` or, failing that, inside child nodes
    /// that declare nothing themselves (`definition` naming itself through its `prototype`).
    fn name_token(&self, n: NodeId, field: FieldId) -> Option<NodeId> {
        let tree = self.tree;
        if let Some(t) = tree.field(n, field)
            && tree.is_token(t)
        {
            return Some(t);
        }
        tree.children(n).into_iter().find_map(|c| {
            if tree.is_token(c) || self.grammar.definition(tree.kind(c)).is_some() {
                None
            } else {
                self.name_token(c, field)
            }
        })
    }

    /// The innermost `@scope` node strictly around `n`; `None` is the file.
    fn scope_above(&self, n: NodeId) -> Option<NodeId> {
        let mut p = self.parent[n.0 as usize];
        while !p.is_none() {
            if self.grammar.is_scope(self.tree.kind(p)) {
                return Some(p);
            }
            p = self.parent[p.0 as usize];
        }
        None
    }

    fn enclosing_symbol(&self, n: NodeId) -> Option<u32> {
        let mut p = self.parent[n.0 as usize];
        while !p.is_none() {
            if let Some(&s) = self.symbol_of.get(&p) {
                return Some(s);
            }
            p = self.parent[p.0 as usize];
        }
        None
    }

    fn resolve(
        &self,
        name: &str,
        at: u32,
        by_name: &HashMap<(Option<NodeId>, &str), Vec<u32>>,
    ) -> Vec<u32> {
        let mut scope = match self.leaves.get(&at) {
            Some(&leaf) => self.scope_above(leaf),
            None => None,
        };
        loop {
            if let Some(candidates) = by_name.get(&(scope, name)) {
                // A variable is not visible in its own declaration (`var a = a + 1` reads an outer
                // `a`), unlike a function in its body.
                let before = candidates.iter().rev().find(|&&s| {
                    let s = &self.symbols[s as usize];
                    s.name_span.start <= at
                        && !(matches!(s.kind, SymbolKind::Variable | SymbolKind::Constant)
                            && s.extent.start <= at
                            && at < s.extent.end)
                });
                let later = candidates
                    .iter()
                    .find(|&&s| scope.is_none() || self.symbols[s as usize].kind.is_hoisted());
                if let Some(&s) = before.or(later) {
                    return vec![s];
                }
            }
            match scope {
                Some(s) => scope = self.scope_above(s),
                None => return Vec::new(),
            }
        }
    }
}
