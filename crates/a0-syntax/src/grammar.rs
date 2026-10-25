//! The grammar DSL: parsing grammar files into a compiled [`Grammar`] and validating them.

use std::collections::HashMap;
use std::sync::Arc;

use a0_diag::{Diagnostic, Diagnostics, SourceId, Span};

use crate::regex::{self, Matcher};

/// A token or node kind: a literal token, a named token, a rule, a precedence node kind, or one of
/// the built-in kinds [`KindId::EOF`] and [`KindId::ERROR`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct KindId(pub u32);

impl KindId {
    /// The end-of-input token.
    pub const EOF: KindId = KindId(0);
    /// A node covering text that could not be parsed (produced by error recovery).
    pub const ERROR: KindId = KindId(1);
}

/// A field label (`cond:expr` labels a child with the field `cond`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct FieldId(pub u32);

impl FieldId {
    pub const NONE: FieldId = FieldId(u32::MAX);
}

/// What a grammar's `@def` rules declare, for editors (outline, highlighting, navigation).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum SymbolKind {
    Function,
    Method,
    Class,
    Struct,
    Enum,
    Interface,
    Variant,
    Field,
    Variable,
    Constant,
    Parameter,
    TypeParameter,
    Type,
    Module,
}

impl SymbolKind {
    pub const ALL: [SymbolKind; 14] = [
        SymbolKind::Function,
        SymbolKind::Method,
        SymbolKind::Class,
        SymbolKind::Struct,
        SymbolKind::Enum,
        SymbolKind::Interface,
        SymbolKind::Variant,
        SymbolKind::Field,
        SymbolKind::Variable,
        SymbolKind::Constant,
        SymbolKind::Parameter,
        SymbolKind::TypeParameter,
        SymbolKind::Type,
        SymbolKind::Module,
    ];

    /// The name used in `@def(...)`.
    pub fn name(self) -> &'static str {
        match self {
            SymbolKind::Function => "function",
            SymbolKind::Method => "method",
            SymbolKind::Class => "class",
            SymbolKind::Struct => "struct",
            SymbolKind::Enum => "enum",
            SymbolKind::Interface => "interface",
            SymbolKind::Variant => "variant",
            SymbolKind::Field => "field",
            SymbolKind::Variable => "variable",
            SymbolKind::Constant => "constant",
            SymbolKind::Parameter => "parameter",
            SymbolKind::TypeParameter => "type_parameter",
            SymbolKind::Type => "type",
            SymbolKind::Module => "module",
        }
    }

    pub fn from_name(name: &str) -> Option<SymbolKind> {
        SymbolKind::ALL.into_iter().find(|k| k.name() == name)
    }

    /// Whether a symbol of this kind is visible in its whole scope, not only after its declaration.
    pub fn is_hoisted(self) -> bool {
        !matches!(
            self,
            SymbolKind::Variable | SymbolKind::Constant | SymbolKind::Parameter
        )
    }

    /// Whether the symbol is reached with `.` or `::` from a value or type (`p.x`, `Shape::Circle`).
    pub fn is_member(self) -> bool {
        matches!(
            self,
            SymbolKind::Method | SymbolKind::Field | SymbolKind::Variant
        )
    }
}

/// A rule marked `@def`: its nodes declare names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Definition {
    pub kind: SymbolKind,
    /// The field that holds the name.
    pub field: FieldId,
    /// `@def(kind: field)`: every token in the field is a declaration of its own. Otherwise
    /// (`@def(kind)`) the node is one declaration, named by the first token in its `name` field.
    pub each: bool,
}

pub(crate) type ExprId = usize;
pub(crate) type RuleId = usize;

#[derive(Clone, Debug)]
pub(crate) enum Expr {
    Token(KindId),
    Eof,
    Rule(RuleId),
    Seq(Vec<ExprId>),
    Alt(Vec<ExprId>),
    /// `e*` (min 0), `e+` (min 1), `e?` (min 0, at most once).
    Repeat {
        expr: ExprId,
        min: u32,
        at_most_once: bool,
    },
    Not(ExprId),
    And(ExprId),
    Field(FieldId, ExprId),
    /// `e ~ "message"`: if `e` cannot even start here, report `message`.
    Expect(ExprId, String),
    Prec(usize),
}

#[derive(Clone, Debug)]
pub(crate) struct Recover {
    pub after: Vec<KindId>,
    pub before: Vec<KindId>,
}

#[derive(Clone, Debug)]
pub(crate) struct Rule {
    pub name: String,
    /// Node kind for this rule, or `None` for `@inline` rules (their children go to the parent).
    pub kind: Option<KindId>,
    pub body: ExprId,
    pub recover: Option<Recover>,
    pub span: Span,
}

/// `@def(kind)` or `@def(kind: field)` as written, before the field is checked.
#[derive(Clone, Debug)]
struct DefAttr {
    kind: String,
    field: Option<String>,
    span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Assoc {
    Left,
    Right,
}

#[derive(Clone, Debug)]
pub(crate) struct Prec {
    /// The operand expression (with its `~` message folded in).
    pub atom: ExprId,
    /// Binary operators: token kind -> (level, associativity, node kind).
    pub binary: HashMap<KindId, (u32, Assoc, KindId)>,
    /// Prefix operators: token kind -> (level, node kind).
    pub prefix: HashMap<KindId, (u32, KindId)>,
    /// Postfix forms: first token kind -> (level, rest of the sequence, node kind).
    pub postfix: HashMap<KindId, (u32, ExprId, KindId)>,
}

#[derive(Clone, Debug)]
pub(crate) enum LexAction {
    Token(KindId),
    Skip,
    Error(String),
}

/// Names of kinds and fields, shared with every tree parsed by a grammar.
#[derive(Debug)]
pub(crate) struct Names {
    pub kinds: Vec<String>,
    /// Whether each kind is a token (leaf) kind.
    pub is_token: Vec<bool>,
    /// For literal token kinds, the literal text.
    pub literal_text: Vec<Option<String>>,
    pub fields: Vec<String>,
}

/// A compiled grammar: a lexer and a parser for one language.
pub struct Grammar {
    pub(crate) name: String,
    pub(crate) matcher: Matcher,
    pub(crate) actions: Vec<LexAction>,
    pub(crate) unknown_message: String,
    pub(crate) names: Arc<Names>,
    pub(crate) kind_map: HashMap<String, KindId>,
    pub(crate) literal_map: HashMap<String, KindId>,
    pub(crate) field_map: HashMap<String, FieldId>,
    pub(crate) exprs: Vec<Expr>,
    pub(crate) rules: Vec<Rule>,
    pub(crate) precs: Vec<Prec>,
    pub(crate) start: RuleId,
    pub(crate) std_fields: StdFields,
    /// Per kind: whether its nodes are `@scope`s, and what they declare (`@def`).
    pub(crate) scopes: Vec<bool>,
    pub(crate) definitions: Vec<Option<Definition>>,
    /// Per kind: whether its nodes import a module (`@import`).
    pub(crate) imports: Vec<bool>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct StdFields {
    pub left: FieldId,
    pub op: FieldId,
    pub right: FieldId,
    pub operand: FieldId,
}

impl Grammar {
    /// Parses and validates a grammar. `source` is the id of `text` in the caller's `SourceMap`.
    pub fn from_str(text: &str, source: SourceId) -> Result<Grammar, Diagnostics> {
        let items = DslParser::new(text, source)?.file()?;
        Builder::new(source).build(items)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The kind with this name: a rule, a named token, a precedence node kind, `EOF` or `error`.
    /// Literal tokens are found with [`Grammar::literal`].
    pub fn kind(&self, name: &str) -> Option<KindId> {
        self.kind_map.get(name).copied()
    }

    /// The token kind of a literal used in the grammar, such as `"+"` or `"while"` (without quotes).
    pub fn literal(&self, text: &str) -> Option<KindId> {
        self.literal_map.get(text).copied()
    }

    pub fn field(&self, name: &str) -> Option<FieldId> {
        self.field_map.get(name).copied()
    }

    pub fn kind_name(&self, kind: KindId) -> &str {
        &self.names.kinds[kind.0 as usize]
    }

    pub fn field_name(&self, field: FieldId) -> &str {
        &self.names.fields[field.0 as usize]
    }

    pub fn kind_count(&self) -> u32 {
        self.names.kinds.len() as u32
    }

    pub fn is_token_kind(&self, kind: KindId) -> bool {
        self.names.is_token[kind.0 as usize]
    }

    /// The text of a literal token kind (`while` for the kind of `"while"`).
    pub fn literal_text(&self, kind: KindId) -> Option<&str> {
        self.names.literal_text[kind.0 as usize].as_deref()
    }

    /// Whether nodes of this kind open a scope for the names declared in them (`@scope`).
    pub fn is_scope(&self, kind: KindId) -> bool {
        self.scopes.get(kind.0 as usize).copied().unwrap_or(false)
    }

    /// What nodes of this kind declare (`@def`).
    pub fn definition(&self, kind: KindId) -> Option<Definition> {
        self.definitions.get(kind.0 as usize).copied().flatten()
    }

    /// Whether nodes of this kind import a module (`@import`).
    pub fn is_import(&self, kind: KindId) -> bool {
        self.imports.get(kind.0 as usize).copied().unwrap_or(false)
    }
}

// ------------------------------------------------------------------------------------------ DSL lexer

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ident(String),
    Str(String),
    Regex(String),
    Attr(String),
    Punct(&'static str),
    Eof,
}

#[derive(Clone, Debug)]
struct DslToken {
    tok: Tok,
    span: Span,
}

const PUNCT: [&str; 15] = [
    "=>", "=", ";", "|", "(", ")", "*", "+", "?", "!", "&", ":", "~", "{", "}",
];

fn lex_dsl(text: &str, source: SourceId) -> Result<Vec<DslToken>, Diagnostics> {
    let bytes = text.as_bytes();
    let mut toks = Vec::new();
    let mut i = 0;
    let span = |a: usize, b: usize| Span::new(source, a as u32, b as u32);
    let err = |a: usize, b: usize, m: &str| -> Diagnostics {
        Diagnostic::error("grammar.syntax", m)
            .with_span(span(a, b))
            .into()
    };
    'outer: while i < bytes.len() {
        let c = bytes[i];
        let start = i;
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if text[i..].starts_with("//") {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if text[i..].starts_with("/*") {
            match text[i + 2..].find("*/") {
                Some(end) => i += end + 4,
                None => return Err(err(i, i + 2, "unterminated comment")),
            }
            continue;
        }
        if c == b'/' {
            i += 1;
            let mut pat = String::new();
            loop {
                match bytes.get(i) {
                    None | Some(b'\n') => return Err(err(start, i, "unterminated pattern")),
                    Some(b'/') => {
                        i += 1;
                        break;
                    }
                    Some(b'\\') if bytes.get(i + 1) == Some(&b'/') => {
                        pat.push_str("\\/");
                        i += 2;
                    }
                    Some(b'\\') => {
                        let ch_len = text[i + 1..]
                            .chars()
                            .next()
                            .map(char::len_utf8)
                            .unwrap_or(0);
                        pat.push_str(&text[i..i + 1 + ch_len]);
                        i += 1 + ch_len;
                    }
                    Some(_) => {
                        let ch = text[i..].chars().next().unwrap();
                        pat.push(ch);
                        i += ch.len_utf8();
                    }
                }
            }
            toks.push(DslToken {
                tok: Tok::Regex(pat),
                span: span(start, i),
            });
            continue;
        }
        if c == b'"' {
            i += 1;
            let mut s = String::new();
            loop {
                match bytes.get(i) {
                    None | Some(b'\n') => return Err(err(start, i, "unterminated string")),
                    Some(b'"') => {
                        i += 1;
                        break;
                    }
                    Some(b'\\') => {
                        let e = bytes.get(i + 1).copied();
                        s.push(match e {
                            Some(b'"') => '"',
                            Some(b'\\') => '\\',
                            Some(b'n') => '\n',
                            Some(b't') => '\t',
                            Some(b'r') => '\r',
                            _ => return Err(err(i, i + 2, "unknown escape in string")),
                        });
                        i += 2;
                    }
                    Some(_) => {
                        let ch = text[i..].chars().next().unwrap();
                        s.push(ch);
                        i += ch.len_utf8();
                    }
                }
            }
            toks.push(DslToken {
                tok: Tok::Str(s),
                span: span(start, i),
            });
            continue;
        }
        if c == b'@' || c.is_ascii_alphabetic() || c == b'_' {
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = text[start..i].to_string();
            let tok = if c == b'@' {
                Tok::Attr(word[1..].to_string())
            } else {
                Tok::Ident(word)
            };
            toks.push(DslToken {
                tok,
                span: span(start, i),
            });
            continue;
        }
        for p in PUNCT {
            if text[i..].starts_with(p) {
                i += p.len();
                toks.push(DslToken {
                    tok: Tok::Punct(p),
                    span: span(start, i),
                });
                continue 'outer;
            }
        }
        let len = text[i..].chars().next().map(char::len_utf8).unwrap_or(1);
        return Err(err(i, i + len, "unexpected character in grammar"));
    }
    toks.push(DslToken {
        tok: Tok::Eof,
        span: span(bytes.len(), bytes.len()),
    });
    Ok(toks)
}

// ------------------------------------------------------------------------------------------ DSL syntax

/// Grammar syntax before name resolution.
#[derive(Clone, Debug)]
enum Ast {
    Lit(String),
    Name(String, Span),
    Eof,
    Seq(Vec<Ast>),
    Alt(Vec<Ast>),
    Repeat(Box<Ast>, u32, bool),
    Not(Box<Ast>),
    And(Box<Ast>),
    Field(String, Box<Ast>),
    Expect(Box<Ast>, String),
}

#[derive(Clone, Debug)]
enum PrecLine {
    Infix(Assoc, Vec<(String, Span)>, String),
    Prefix(Vec<(String, Span)>, String),
    Postfix(Vec<Ast>, String, Span),
}

#[derive(Clone, Debug)]
enum RuleBody {
    Plain(Ast),
    Prec { atom: Ast, lines: Vec<PrecLine> },
}

/// Literals listed in `@recover(after ... before ...)`, with their spans.
type SyncTokens = Vec<(String, Span)>;

#[derive(Clone, Debug)]
enum Item {
    Name(String),
    Skip(String, Span),
    Token(String, String, Span),
    Error(String, String, Span),
    Unknown(String),
    Rule {
        name: String,
        span: Span,
        inline: bool,
        recover: Option<(SyncTokens, SyncTokens)>,
        scope: bool,
        def: Option<DefAttr>,
        import: bool,
        body: RuleBody,
    },
}

struct DslParser {
    toks: Vec<DslToken>,
    pos: usize,
}

impl DslParser {
    fn new(text: &str, source: SourceId) -> Result<DslParser, Diagnostics> {
        Ok(DslParser {
            toks: lex_dsl(text, source)?,
            pos: 0,
        })
    }

    fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }

    fn span(&self) -> Span {
        self.toks[self.pos].span
    }

    fn bump(&mut self) -> DslToken {
        let t = self.toks[self.pos].clone();
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }

    fn fail<T>(&self, what: &str) -> Result<T, Diagnostics> {
        let found = match self.peek() {
            Tok::Ident(s) => format!("'{s}'"),
            Tok::Str(s) => format!("\"{s}\""),
            Tok::Regex(_) => "a pattern".to_string(),
            Tok::Attr(a) => format!("'@{a}'"),
            Tok::Punct(p) => format!("'{p}'"),
            Tok::Eof => "end of file".to_string(),
        };
        Err(
            Diagnostic::error("grammar.syntax", &format!("expected {what}, found {found}"))
                .with_span(self.span())
                .into(),
        )
    }

    fn punct(&mut self, p: &'static str) -> Result<(), Diagnostics> {
        if *self.peek() == Tok::Punct(p) {
            self.bump();
            Ok(())
        } else {
            self.fail(&format!("'{p}'"))
        }
    }

    fn is_punct(&self, p: &str) -> bool {
        matches!(self.peek(), Tok::Punct(q) if *q == p)
    }

    fn is_word(&self, w: &str) -> bool {
        matches!(self.peek(), Tok::Ident(s) if s == w)
    }

    fn ident(&mut self, what: &str) -> Result<(String, Span), Diagnostics> {
        let span = self.span();
        match self.peek().clone() {
            Tok::Ident(s) => {
                self.bump();
                Ok((s, span))
            }
            _ => self.fail(what),
        }
    }

    fn string(&mut self, what: &str) -> Result<(String, Span), Diagnostics> {
        let span = self.span();
        match self.peek().clone() {
            Tok::Str(s) => {
                self.bump();
                Ok((s, span))
            }
            _ => self.fail(what),
        }
    }

    fn regex(&mut self) -> Result<(String, Span), Diagnostics> {
        let span = self.span();
        match self.peek().clone() {
            Tok::Regex(s) => {
                self.bump();
                Ok((s, span))
            }
            _ => self.fail("a pattern /.../"),
        }
    }

    fn file(&mut self) -> Result<Vec<Item>, Diagnostics> {
        let mut items = Vec::new();
        if self.is_word("grammar") {
            self.bump();
            let (name, _) = self.ident("the grammar name")?;
            self.punct(";")?;
            items.push(Item::Name(name));
        }
        while *self.peek() != Tok::Eof {
            items.push(self.item()?);
        }
        Ok(items)
    }

    fn item(&mut self) -> Result<Item, Diagnostics> {
        if self.is_word("skip") {
            self.bump();
            let (pat, span) = self.regex()?;
            self.punct(";")?;
            return Ok(Item::Skip(pat, span));
        }
        if self.is_word("token") {
            self.bump();
            let (name, span) = self.ident("a token name")?;
            let (pat, _) = self.regex()?;
            self.punct(";")?;
            return Ok(Item::Token(name, pat, span));
        }
        if self.is_word("error") {
            self.bump();
            let (msg, _) = self.string("an error message")?;
            let (pat, span) = self.regex()?;
            self.punct(";")?;
            return Ok(Item::Error(msg, pat, span));
        }
        if self.is_word("unknown") {
            self.bump();
            let (msg, _) = self.string("an error message")?;
            self.punct(";")?;
            return Ok(Item::Unknown(msg));
        }
        let (name, span) = self.ident("a rule or declaration")?;
        let mut inline = false;
        let mut recover = None;
        let mut scope = false;
        let mut def = None;
        let mut import = false;
        while let Tok::Attr(a) = self.peek().clone() {
            match a.as_str() {
                "inline" => {
                    self.bump();
                    inline = true;
                }
                "import" => {
                    self.bump();
                    import = true;
                }
                "scope" => {
                    self.bump();
                    scope = true;
                }
                "def" => {
                    let span = self.span();
                    self.bump();
                    self.punct("(")?;
                    let (kind, _) = self.ident("a symbol kind")?;
                    let field = if self.is_punct(":") {
                        self.bump();
                        Some(self.ident("a field name")?.0)
                    } else {
                        None
                    };
                    self.punct(")")?;
                    def = Some(DefAttr { kind, field, span });
                }
                "recover" => {
                    self.bump();
                    self.punct("(")?;
                    let mut after = Vec::new();
                    let mut before = Vec::new();
                    loop {
                        if self.is_word("after") {
                            self.bump();
                            while let Tok::Str(_) = self.peek() {
                                after.push(self.string("a literal")?);
                            }
                        } else if self.is_word("before") {
                            self.bump();
                            while let Tok::Str(_) = self.peek() {
                                before.push(self.string("a literal")?);
                            }
                        } else {
                            break;
                        }
                    }
                    self.punct(")")?;
                    recover = Some((after, before));
                }
                _ => break,
            }
        }
        self.punct("=")?;
        let body = if matches!(self.peek(), Tok::Attr(a) if a == "prec") {
            self.bump();
            self.prec_body()?
        } else {
            RuleBody::Plain(self.alt()?)
        };
        self.punct(";")?;
        Ok(Item::Rule {
            name,
            span,
            inline,
            recover,
            scope,
            def,
            import,
            body,
        })
    }

    fn prec_body(&mut self) -> Result<RuleBody, Diagnostics> {
        let atom = self.item_expr()?;
        self.punct("{")?;
        let mut lines = Vec::new();
        while !self.is_punct("}") {
            let (kw, _) = self.ident("'left', 'right', 'prefix' or 'postfix'")?;
            let line = match kw.as_str() {
                "left" | "right" | "prefix" => {
                    let mut ops = Vec::new();
                    while let Tok::Str(_) = self.peek() {
                        ops.push(self.string("an operator literal")?);
                    }
                    if ops.is_empty() {
                        return self.fail("an operator literal");
                    }
                    self.punct("=>")?;
                    let (node, _) = self.ident("a node kind name")?;
                    match kw.as_str() {
                        "left" => PrecLine::Infix(Assoc::Left, ops, node),
                        "right" => PrecLine::Infix(Assoc::Right, ops, node),
                        _ => PrecLine::Prefix(ops, node),
                    }
                }
                "postfix" => {
                    let span = self.span();
                    let mut seq = Vec::new();
                    while !self.is_punct("=>") {
                        seq.push(self.item_expr()?);
                    }
                    self.punct("=>")?;
                    let (node, _) = self.ident("a node kind name")?;
                    PrecLine::Postfix(seq, node, span)
                }
                _ => return self.fail("'left', 'right', 'prefix' or 'postfix'"),
            };
            self.punct(";")?;
            lines.push(line);
        }
        self.punct("}")?;
        Ok(RuleBody::Prec { atom, lines })
    }

    fn alt(&mut self) -> Result<Ast, Diagnostics> {
        let mut options = vec![self.seq()?];
        while self.is_punct("|") {
            self.bump();
            options.push(self.seq()?);
        }
        Ok(if options.len() == 1 {
            options.pop().unwrap()
        } else {
            Ast::Alt(options)
        })
    }

    fn seq(&mut self) -> Result<Ast, Diagnostics> {
        let mut items = Vec::new();
        while !(self.is_punct("|")
            || self.is_punct(")")
            || self.is_punct(";")
            || *self.peek() == Tok::Eof)
        {
            items.push(self.item_expr()?);
        }
        Ok(if items.len() == 1 {
            items.pop().unwrap()
        } else {
            Ast::Seq(items)
        })
    }

    /// `[field:] [!|&] primary [*|+|?]... [~ "message"]`
    fn item_expr(&mut self) -> Result<Ast, Diagnostics> {
        let mut field = None;
        if let (Tok::Ident(name), Some(next)) = (self.peek().clone(), self.toks.get(self.pos + 1))
            && next.tok == Tok::Punct(":")
        {
            self.bump();
            self.bump();
            field = Some(name);
        }
        let prefix = if self.is_punct("!") || self.is_punct("&") {
            let p = self.bump();
            Some(p.tok)
        } else {
            None
        };
        let mut e = self.primary()?;
        loop {
            if self.is_punct("*") {
                self.bump();
                e = Ast::Repeat(Box::new(e), 0, false);
            } else if self.is_punct("+") {
                self.bump();
                e = Ast::Repeat(Box::new(e), 1, false);
            } else if self.is_punct("?") {
                self.bump();
                e = Ast::Repeat(Box::new(e), 0, true);
            } else {
                break;
            }
        }
        e = match prefix {
            Some(Tok::Punct("!")) => Ast::Not(Box::new(e)),
            Some(_) => Ast::And(Box::new(e)),
            None => e,
        };
        if self.is_punct("~") {
            self.bump();
            let (msg, _) = self.string("an error message")?;
            e = Ast::Expect(Box::new(e), msg);
        }
        if let Some(f) = field {
            e = Ast::Field(f, Box::new(e));
        }
        Ok(e)
    }

    fn primary(&mut self) -> Result<Ast, Diagnostics> {
        let span = self.span();
        match self.peek().clone() {
            Tok::Str(s) => {
                self.bump();
                if s.is_empty() {
                    return Err(Diagnostic::error("grammar.syntax", "empty literal")
                        .with_span(span)
                        .into());
                }
                Ok(Ast::Lit(s))
            }
            Tok::Ident(name) if name == "EOF" => {
                self.bump();
                Ok(Ast::Eof)
            }
            Tok::Ident(name) => {
                self.bump();
                Ok(Ast::Name(name, span))
            }
            Tok::Punct("(") => {
                self.bump();
                let inner = self.alt()?;
                self.punct(")")?;
                Ok(inner)
            }
            _ => self.fail("a literal, a name or '('"),
        }
    }
}

// ------------------------------------------------------------------------------------------ building

struct Builder {
    source: SourceId,
    diags: Diagnostics,
    kinds: Vec<String>,
    is_token: Vec<bool>,
    literal_text: Vec<Option<String>>,
    kind_map: HashMap<String, KindId>,
    literal_map: HashMap<String, KindId>,
    fields: Vec<String>,
    field_map: HashMap<String, FieldId>,
    exprs: Vec<Expr>,
    rule_map: HashMap<String, RuleId>,
    token_map: HashMap<String, KindId>,
    precs: Vec<Prec>,
}

impl Builder {
    fn new(source: SourceId) -> Builder {
        let mut b = Builder {
            source,
            diags: Diagnostics::new(),
            kinds: Vec::new(),
            is_token: Vec::new(),
            literal_text: Vec::new(),
            kind_map: HashMap::new(),
            literal_map: HashMap::new(),
            fields: Vec::new(),
            field_map: HashMap::new(),
            exprs: Vec::new(),
            rule_map: HashMap::new(),
            token_map: HashMap::new(),
            precs: Vec::new(),
        };
        b.new_kind("EOF", true, None);
        b.new_kind("error", false, None);
        b
    }

    fn error(&mut self, span: Option<Span>, msg: &str) {
        self.diags
            .push(Diagnostic::error("grammar.invalid", msg).with_span_opt(span));
    }

    fn new_kind(&mut self, name: &str, token: bool, literal: Option<&str>) -> KindId {
        let id = KindId(self.kinds.len() as u32);
        self.kinds.push(name.to_string());
        self.is_token.push(token);
        self.literal_text.push(literal.map(str::to_string));
        self.kind_map.insert(name.to_string(), id);
        id
    }

    fn node_kind(&mut self, name: &str) -> KindId {
        match self.kind_map.get(name) {
            Some(&k) => k,
            None => self.new_kind(name, false, None),
        }
    }

    fn literal(&mut self, text: &str) -> KindId {
        if let Some(&k) = self.literal_map.get(text) {
            return k;
        }
        let k = self.new_kind(&format!("\"{text}\""), true, Some(text));
        self.literal_map.insert(text.to_string(), k);
        k
    }

    fn field(&mut self, name: &str) -> FieldId {
        if let Some(&f) = self.field_map.get(name) {
            return f;
        }
        let f = FieldId(self.fields.len() as u32);
        self.fields.push(name.to_string());
        self.field_map.insert(name.to_string(), f);
        f
    }

    fn push(&mut self, e: Expr) -> ExprId {
        self.exprs.push(e);
        self.exprs.len() - 1
    }

    fn build(mut self, items: Vec<Item>) -> Result<Grammar, Diagnostics> {
        let mut name = String::from("grammar");
        let mut unknown_message = "unexpected character".to_string();
        // Lexer patterns that are not literals, in declaration order.
        let mut patterns: Vec<(regex::Node, LexAction)> = Vec::new();

        // First pass: tokens and rule names, so rules may refer to anything in any order.
        let mut rule_items = Vec::new();
        for item in items {
            match item {
                Item::Name(n) => name = n,
                Item::Unknown(m) => unknown_message = m,
                Item::Skip(pat, span) => {
                    if let Some(node) = self.pattern(&pat, span) {
                        patterns.push((node, LexAction::Skip));
                    }
                }
                Item::Error(msg, pat, span) => {
                    if let Some(node) = self.pattern(&pat, span) {
                        patterns.push((node, LexAction::Error(msg)));
                    }
                }
                Item::Token(tname, pat, span) => {
                    if self.kind_map.contains_key(&tname) {
                        self.error(Some(span), &format!("'{tname}' is defined more than once"));
                        continue;
                    }
                    let kind = self.new_kind(&tname, true, None);
                    self.token_map.insert(tname, kind);
                    if let Some(node) = self.pattern(&pat, span) {
                        patterns.push((node, LexAction::Token(kind)));
                    }
                }
                Item::Rule { .. } => rule_items.push(item),
            }
        }
        if rule_items.is_empty() {
            self.error(None, "the grammar has no rules");
            return Err(self.diags);
        }

        let mut rules = Vec::new();
        for item in &rule_items {
            if let Item::Rule {
                name: rname,
                span,
                inline,
                ..
            } = item
            {
                if self.rule_map.contains_key(rname)
                    || self.token_map.contains_key(rname)
                    || rname == "EOF"
                {
                    self.error(Some(*span), &format!("'{rname}' is defined more than once"));
                    continue;
                }
                let kind = if *inline {
                    None
                } else {
                    Some(self.node_kind(rname))
                };
                self.rule_map.insert(rname.clone(), rules.len());
                rules.push(Rule {
                    name: rname.clone(),
                    kind,
                    body: 0,
                    recover: None,
                    span: *span,
                });
            }
        }
        let std_fields = StdFields {
            left: self.field("left"),
            op: self.field("op"),
            right: self.field("right"),
            operand: self.field("operand"),
        };

        // Second pass: rule bodies.
        let mut annotations = Vec::new();
        for item in rule_items {
            let Item::Rule {
                name: rname,
                span,
                recover,
                scope,
                def,
                import,
                body,
                ..
            } = item
            else {
                continue;
            };
            if scope || def.is_some() || import {
                annotations.push((rname.clone(), span, scope, def, import));
            }
            let Some(&rid) = self.rule_map.get(&rname) else {
                continue;
            };
            let body = match body {
                RuleBody::Plain(ast) => self.lower(&ast),
                RuleBody::Prec { atom, lines } => {
                    let p = self.prec(&atom, &lines, &rules);
                    self.push(Expr::Prec(p))
                }
            };
            rules[rid].body = body;
            if let Some((after, before)) = recover {
                let after = after.iter().map(|(s, _)| self.literal(s)).collect();
                let before = before.iter().map(|(s, _)| self.literal(s)).collect();
                rules[rid].recover = Some(Recover { after, before });
            }
        }
        if rules[0].kind.is_none() {
            let span = rules[0].span;
            self.error(
                Some(span),
                "the start rule (the first rule) cannot be @inline",
            );
        }

        self.validate(&rules);
        let (scopes, definitions, imports) = self.annotations(&rules, annotations);
        if self.diags.has_errors() {
            return Err(self.diags);
        }

        // Literals first (they win ties), then the other patterns in declaration order.
        let mut matcher = Matcher::new();
        let mut actions = Vec::new();
        let mut literals: Vec<(String, KindId)> = self
            .literal_map
            .iter()
            .map(|(t, &k)| (t.clone(), k))
            .collect();
        literals.sort_by_key(|(_, k)| *k);
        for (text, kind) in literals {
            matcher.add_literal(&text, actions.len() as u32);
            actions.push(LexAction::Token(kind));
        }
        for (node, action) in patterns {
            matcher.add(&node, actions.len() as u32);
            actions.push(action);
        }

        Ok(Grammar {
            name,
            matcher,
            actions,
            unknown_message,
            names: Arc::new(Names {
                kinds: self.kinds,
                is_token: self.is_token,
                literal_text: self.literal_text,
                fields: self.fields,
            }),
            kind_map: self.kind_map,
            literal_map: self.literal_map,
            field_map: self.field_map,
            exprs: self.exprs,
            rules,
            precs: self.precs,
            start: 0,
            std_fields,
            scopes,
            definitions,
            imports,
        })
    }

    /// Checks `@scope`, `@def` and `@import` and indexes them by node kind.
    #[allow(clippy::type_complexity)]
    fn annotations(
        &mut self,
        rules: &[Rule],
        annotations: Vec<(String, Span, bool, Option<DefAttr>, bool)>,
    ) -> (Vec<bool>, Vec<Option<Definition>>, Vec<bool>) {
        let mut scopes = vec![false; self.kinds.len()];
        let mut definitions = vec![None; self.kinds.len()];
        let mut imports = vec![false; self.kinds.len()];
        for (rname, span, scope, def, import) in annotations {
            let Some(kind) = self.rule_map.get(&rname).and_then(|&r| rules[r].kind) else {
                self.error(
                    Some(span),
                    &format!(
                        "'{rname}' is @inline, so it has no nodes to mark with @scope, @def or \
                         @import"
                    ),
                );
                continue;
            };
            scopes[kind.0 as usize] = scope;
            if import {
                if self.field_map.contains_key("path") {
                    imports[kind.0 as usize] = true;
                } else {
                    self.error(
                        Some(span),
                        "@import reads the module from a field called 'path', and there is none",
                    );
                }
            }
            let Some(def) = def else { continue };
            let Some(symbol) = SymbolKind::from_name(&def.kind) else {
                let names: Vec<&str> = SymbolKind::ALL.iter().map(|k| k.name()).collect();
                self.error(
                    Some(def.span),
                    &format!(
                        "unknown symbol kind '{}' (expected one of: {})",
                        def.kind,
                        names.join(", ")
                    ),
                );
                continue;
            };
            let field_name = def.field.as_deref().unwrap_or("name");
            let Some(&field) = self.field_map.get(field_name) else {
                let what = match def.field {
                    Some(_) => format!("there is no field '{field_name}'"),
                    None => {
                        "@def(kind) reads the name from a field called 'name', and there is none; \
                             write @def(kind: field)"
                            .to_string()
                    }
                };
                self.error(Some(def.span), &what);
                continue;
            };
            definitions[kind.0 as usize] = Some(Definition {
                kind: symbol,
                field,
                each: def.field.is_some(),
            });
        }
        (scopes, definitions, imports)
    }

    fn pattern(&mut self, pat: &str, span: Span) -> Option<regex::Node> {
        match regex::parse(pat) {
            Ok(node) => {
                if regex::matches_empty(&node) {
                    self.error(
                        Some(span),
                        "this pattern can match empty text, which would never advance the lexer",
                    );
                    None
                } else {
                    Some(node)
                }
            }
            Err(e) => {
                self.error(Some(span), &format!("invalid pattern: {e}"));
                None
            }
        }
    }

    fn lower(&mut self, ast: &Ast) -> ExprId {
        let e = match ast {
            Ast::Lit(s) => Expr::Token(self.literal(s)),
            Ast::Eof => Expr::Eof,
            Ast::Name(n, span) => {
                if let Some(&k) = self.token_map.get(n) {
                    Expr::Token(k)
                } else if let Some(&r) = self.rule_map.get(n) {
                    Expr::Rule(r)
                } else {
                    self.error(Some(*span), &format!("'{n}' is not a rule or token"));
                    Expr::Eof
                }
            }
            Ast::Seq(items) => {
                let ids = items.iter().map(|i| self.lower(i)).collect();
                Expr::Seq(ids)
            }
            Ast::Alt(options) => {
                let ids = options.iter().map(|i| self.lower(i)).collect();
                Expr::Alt(ids)
            }
            Ast::Repeat(inner, min, once) => {
                let e = self.lower(inner);
                Expr::Repeat {
                    expr: e,
                    min: *min,
                    at_most_once: *once,
                }
            }
            Ast::Not(inner) => {
                let e = self.lower(inner);
                Expr::Not(e)
            }
            Ast::And(inner) => {
                let e = self.lower(inner);
                Expr::And(e)
            }
            Ast::Field(f, inner) => {
                let fid = self.field(f);
                let e = self.lower(inner);
                Expr::Field(fid, e)
            }
            Ast::Expect(inner, msg) => {
                let e = self.lower(inner);
                Expr::Expect(e, msg.clone())
            }
        };
        self.push(e)
    }

    fn prec(&mut self, atom: &Ast, lines: &[PrecLine], rules: &[Rule]) -> usize {
        let atom_id = self.lower(atom);
        // The operand must produce exactly one node: a token or a non-inline rule.
        let mut core = atom_id;
        while let Expr::Expect(inner, _) | Expr::Field(_, inner) = &self.exprs[core] {
            core = *inner;
        }
        let single = match &self.exprs[core] {
            Expr::Token(_) => true,
            Expr::Rule(r) => rules[*r].kind.is_some(),
            _ => false,
        };
        if !single {
            self.error(
                None,
                "the operand of @prec must be a token or a (non-inline) rule",
            );
        }
        let mut prec = Prec {
            atom: atom_id,
            binary: HashMap::new(),
            prefix: HashMap::new(),
            postfix: HashMap::new(),
        };
        for (i, line) in lines.iter().enumerate() {
            let level = i as u32 + 1;
            match line {
                PrecLine::Infix(assoc, ops, node) => {
                    let kind = self.node_kind(node);
                    for (op, span) in ops {
                        let k = self.literal(op);
                        if prec.binary.insert(k, (level, *assoc, kind)).is_some() {
                            self.error(
                                Some(*span),
                                &format!("binary operator \"{op}\" is listed twice"),
                            );
                        }
                    }
                }
                PrecLine::Prefix(ops, node) => {
                    let kind = self.node_kind(node);
                    for (op, span) in ops {
                        let k = self.literal(op);
                        if prec.prefix.insert(k, (level, kind)).is_some() {
                            self.error(
                                Some(*span),
                                &format!("prefix operator \"{op}\" is listed twice"),
                            );
                        }
                    }
                }
                PrecLine::Postfix(seq, node, span) => {
                    let kind = self.node_kind(node);
                    let Some(Ast::Lit(first)) = seq.first() else {
                        self.error(
                            Some(*span),
                            "a postfix form must start with a literal token",
                        );
                        continue;
                    };
                    let k = self.literal(first);
                    let rest = Ast::Seq(seq[1..].to_vec());
                    let rest_id = self.lower(&rest);
                    if prec.binary.contains_key(&k)
                        || prec.postfix.insert(k, (level, rest_id, kind)).is_some()
                    {
                        self.error(
                            Some(*span),
                            &format!("\"{first}\" already starts another operator"),
                        );
                    }
                }
            }
        }
        self.precs.push(prec);
        self.precs.len() - 1
    }

    fn validate(&mut self, rules: &[Rule]) {
        let n = self.exprs.len();
        // Which expressions can succeed without consuming a token (fixpoint over rules).
        let mut nullable = vec![false; n];
        let mut rule_nullable = vec![false; rules.len()];
        loop {
            let mut changed = false;
            for id in 0..n {
                let v = self.expr_nullable(id, &nullable, &rule_nullable);
                if v != nullable[id] {
                    nullable[id] = v;
                    changed = true;
                }
            }
            for (r, rule) in rules.iter().enumerate() {
                let v = nullable[rule.body];
                if v != rule_nullable[r] {
                    rule_nullable[r] = v;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        for id in 0..n {
            if let Expr::Repeat {
                expr,
                at_most_once: false,
                ..
            } = self.exprs[id]
                && nullable[expr]
            {
                self.error(None, "a repetition (* or +) applies to something that can match nothing; it would loop forever");
            }
        }
        // Left recursion: a rule reachable from itself without consuming input.
        let first_calls: Vec<Vec<RuleId>> = rules
            .iter()
            .map(|r| {
                let mut out = Vec::new();
                self.first_calls(r.body, &nullable, &mut out);
                out
            })
            .collect();
        for start in 0..rules.len() {
            let mut stack = vec![start];
            let mut seen = vec![false; rules.len()];
            while let Some(r) = stack.pop() {
                for &next in &first_calls[r] {
                    if next == start {
                        let msg = format!(
                            "rule '{}' is left-recursive; use @prec for operators or rewrite with repetition",
                            rules[start].name
                        );
                        self.error(Some(rules[start].span), &msg);
                        stack.clear();
                        break;
                    }
                    if !seen[next] {
                        seen[next] = true;
                        stack.push(next);
                    }
                }
            }
        }
        let _ = self.source;
    }

    fn expr_nullable(&self, id: ExprId, nullable: &[bool], rule_nullable: &[bool]) -> bool {
        match &self.exprs[id] {
            Expr::Token(_) => false,
            Expr::Eof | Expr::Not(_) | Expr::And(_) => true,
            Expr::Rule(r) => rule_nullable[*r],
            Expr::Seq(items) => items.iter().all(|&i| nullable[i]),
            Expr::Alt(options) => options.iter().any(|&i| nullable[i]),
            Expr::Repeat { expr, min, .. } => *min == 0 || nullable[*expr],
            Expr::Field(_, e) | Expr::Expect(e, _) => nullable[*e],
            Expr::Prec(p) => nullable[self.precs[*p].atom],
        }
    }

    fn first_calls(&self, id: ExprId, nullable: &[bool], out: &mut Vec<RuleId>) {
        match &self.exprs[id] {
            Expr::Token(_) | Expr::Eof => {}
            Expr::Rule(r) => out.push(*r),
            Expr::Seq(items) => {
                for &i in items {
                    self.first_calls(i, nullable, out);
                    if !nullable[i] {
                        break;
                    }
                }
            }
            Expr::Alt(options) => {
                for &i in options {
                    self.first_calls(i, nullable, out);
                }
            }
            Expr::Repeat { expr, .. }
            | Expr::Not(expr)
            | Expr::And(expr)
            | Expr::Field(_, expr)
            | Expr::Expect(expr, _) => self.first_calls(*expr, nullable, out),
            Expr::Prec(p) => self.first_calls(self.precs[*p].atom, nullable, out),
        }
    }
}
