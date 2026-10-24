//! Parser for the text format (see `docs/ir.md`).
//!
//! Item headers are read in a first pass so that functions and globals can be referenced before
//! they appear; function bodies are parsed in a second pass. Syntax errors are reported as
//! diagnostics with spans; an error inside a function body abandons that body only.

use std::collections::{HashMap, HashSet};

use a0_diag::{Diagnostic, Diagnostics, SourceId, Span};

use crate::module::{GlobalInit, Module};
use crate::types::*;
use crate::verify::verify;

/// Parses a module from text. `source` is the id under which `text` is registered in the caller's
/// `SourceMap`; it is only used for spans. The result is not verified; see [`parse_and_verify`].
pub fn parse(text: &str, source: SourceId) -> Result<Module, Diagnostics> {
    let tokens = lex(text, source)?;
    let mut p = Parser {
        toks: tokens,
        pos: 0,
        diags: Diagnostics::new(),
        module: Module::new(""),
    };
    p.module();
    if p.diags.has_errors() {
        Err(p.diags)
    } else {
        Ok(p.module)
    }
}

/// Parses and then verifies a module.
pub fn parse_and_verify(text: &str, source: SourceId) -> Result<Module, Diagnostics> {
    let m = parse(text, source)?;
    let diags = verify(&m);
    if diags.has_errors() {
        Err(diags)
    } else {
        Ok(m)
    }
}

// ------------------------------------------------------------------------------------------ lexer

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Name {
    Named(String),
    Num(u32),
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Word(String),
    Sym(String),
    Val(Name),
    VarRef(Name),
    Int(i128),
    Float(String),
    Str(Vec<u8>),
    LParen,
    RParen,
    LBrace,
    RBrace,
    Comma,
    Colon,
    Eq,
    Arrow,
    Newline,
    Eof,
}

impl Tok {
    fn describe(&self) -> String {
        match self {
            Tok::Word(w) => format!("'{w}'"),
            Tok::Sym(s) => format!("'@{s}'"),
            Tok::Val(_) => "a value name".to_string(),
            Tok::VarRef(_) => "a variable name".to_string(),
            Tok::Int(_) => "an integer".to_string(),
            Tok::Float(_) => "a float".to_string(),
            Tok::Str(_) => "a string".to_string(),
            Tok::LParen => "'('".to_string(),
            Tok::RParen => "')'".to_string(),
            Tok::LBrace => "'{'".to_string(),
            Tok::RBrace => "'}'".to_string(),
            Tok::Comma => "','".to_string(),
            Tok::Colon => "':'".to_string(),
            Tok::Eq => "'='".to_string(),
            Tok::Arrow => "'->'".to_string(),
            Tok::Newline => "end of line".to_string(),
            Tok::Eof => "end of file".to_string(),
        }
    }
}

struct Token {
    tok: Tok,
    span: Span,
}

fn lex(text: &str, source: SourceId) -> Result<Vec<Token>, Diagnostic> {
    let bytes = text.as_bytes();
    let mut toks = Vec::new();
    let mut i = 0;
    let span = |a: usize, b: usize| Span::new(source, a as u32, b as u32);
    let err = |a: usize, b: usize, msg: &str| {
        Diagnostic::error("ir.syntax", msg).with_span(Span::new(source, a as u32, b as u32))
    };
    while i < bytes.len() {
        let c = bytes[i];
        let start = i;
        match c {
            b' ' | b'\t' | b'\r' => i += 1,
            b';' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'\n' => {
                i += 1;
                toks.push(Token {
                    tok: Tok::Newline,
                    span: span(start, i),
                });
            }
            b'(' | b')' | b'{' | b'}' | b',' | b':' | b'=' => {
                i += 1;
                let tok = match c {
                    b'(' => Tok::LParen,
                    b')' => Tok::RParen,
                    b'{' => Tok::LBrace,
                    b'}' => Tok::RBrace,
                    b',' => Tok::Comma,
                    b':' => Tok::Colon,
                    _ => Tok::Eq,
                };
                toks.push(Token {
                    tok,
                    span: span(start, i),
                });
            }
            b'-' if bytes.get(i + 1) == Some(&b'>') => {
                i += 2;
                toks.push(Token {
                    tok: Tok::Arrow,
                    span: span(start, i),
                });
            }
            b'"' => {
                let (s, end) = lex_string(bytes, i).map_err(|(a, b, m)| err(a, b, m))?;
                i = end;
                toks.push(Token {
                    tok: Tok::Str(s),
                    span: span(start, i),
                });
            }
            b'@' | b'%' | b'$' => {
                i += 1;
                let name = if bytes.get(i) == Some(&b'"') {
                    let (s, end) = lex_string(bytes, i).map_err(|(a, b, m)| err(a, b, m))?;
                    i = end;
                    match String::from_utf8(s) {
                        Ok(s) => Name::Named(s),
                        Err(_) => return Err(err(start, i, "name is not valid UTF-8")),
                    }
                } else {
                    let s = i;
                    while i < bytes.len() && is_ident_byte(bytes[i]) {
                        i += 1;
                    }
                    let word = &text[s..i];
                    if word.is_empty() {
                        return Err(err(start, i, "expected a name after the sigil"));
                    }
                    if word.bytes().all(|b| b.is_ascii_digit()) {
                        match word.parse::<u32>() {
                            Ok(n) => Name::Num(n),
                            Err(_) => return Err(err(start, i, "number too large")),
                        }
                    } else if crate::print::is_bare_ident(word) {
                        Name::Named(word.to_string())
                    } else {
                        return Err(err(start, i, "invalid name; quote it"));
                    }
                };
                let tok = match c {
                    b'@' => match name {
                        Name::Named(s) => Tok::Sym(s),
                        Name::Num(n) => Tok::Sym(n.to_string()),
                    },
                    b'%' => Tok::Val(name),
                    _ => Tok::VarRef(name),
                };
                toks.push(Token {
                    tok,
                    span: span(start, i),
                });
            }
            b'-' | b'0'..=b'9' => {
                let (tok, end) = lex_number(text, i).map_err(|m| err(start, i + 1, m))?;
                i = end;
                toks.push(Token {
                    tok,
                    span: span(start, i),
                });
            }
            _ if c.is_ascii_alphabetic() || c == b'_' => {
                while i < bytes.len() && is_ident_byte(bytes[i]) {
                    i += 1;
                }
                toks.push(Token {
                    tok: Tok::Word(text[start..i].to_string()),
                    span: span(start, i),
                });
            }
            _ => {
                let len = text[i..].chars().next().map(char::len_utf8).unwrap_or(1);
                return Err(err(i, i + len, "unexpected character"));
            }
        }
    }
    toks.push(Token {
        tok: Tok::Eof,
        span: span(bytes.len(), bytes.len()),
    });
    Ok(toks)
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'.'
}

fn lex_string(
    bytes: &[u8],
    start: usize,
) -> Result<(Vec<u8>, usize), (usize, usize, &'static str)> {
    let mut out = Vec::new();
    let mut i = start + 1;
    loop {
        match bytes.get(i) {
            None | Some(b'\n') => return Err((start, i, "unterminated string")),
            Some(b'"') => return Ok((out, i + 1)),
            Some(b'\\') => {
                let esc = bytes.get(i + 1).copied();
                match esc {
                    Some(b'"') => out.push(b'"'),
                    Some(b'\\') => out.push(b'\\'),
                    Some(b'n') => out.push(b'\n'),
                    Some(b't') => out.push(b'\t'),
                    Some(b'r') => out.push(b'\r'),
                    Some(h) if h.is_ascii_hexdigit() => {
                        let l = bytes.get(i + 2).copied().filter(u8::is_ascii_hexdigit);
                        let Some(l) = l else {
                            return Err((i, i + 2, "expected two hex digits after '\\'"));
                        };
                        let hex = [h, l];
                        let s = std::str::from_utf8(&hex).unwrap();
                        out.push(u8::from_str_radix(s, 16).unwrap());
                        i += 1;
                    }
                    _ => return Err((i, i + 2, "unknown escape")),
                }
                i += 2;
            }
            Some(&b) => {
                out.push(b);
                i += 1;
            }
        }
    }
}

fn lex_number(text: &str, start: usize) -> Result<(Tok, usize), &'static str> {
    let bytes = text.as_bytes();
    let mut i = start;
    if bytes[i] == b'-' {
        i += 1;
        if text[i..].starts_with("inf") && !bytes.get(i + 3).is_some_and(|&b| is_ident_byte(b)) {
            return Ok((Tok::Float("-inf".to_string()), i + 3));
        }
        if !bytes.get(i).is_some_and(u8::is_ascii_digit) {
            return Err("expected a number");
        }
    }
    if text[i..].starts_with("0x") {
        let s = i + 2;
        let mut j = s;
        while j < bytes.len() && bytes[j].is_ascii_hexdigit() {
            j += 1;
        }
        if j == s {
            return Err("expected hex digits");
        }
        let v = u64::from_str_radix(&text[s..j], 16).map_err(|_| "hex number too large")?;
        let v = if bytes[start] == b'-' {
            -(v as i128)
        } else {
            v as i128
        };
        return Ok((Tok::Int(v), j));
    }
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    let mut is_float = false;
    if bytes.get(i) == Some(&b'.') && bytes.get(i + 1).is_some_and(u8::is_ascii_digit) {
        is_float = true;
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        if matches!(bytes.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        if bytes.get(j).is_some_and(u8::is_ascii_digit) {
            is_float = true;
            i = j;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
        }
    }
    let s = &text[start..i];
    if is_float {
        Ok((Tok::Float(s.to_string()), i))
    } else {
        let v = s.parse::<i128>().map_err(|_| "integer too large")?;
        Ok((Tok::Int(v), i))
    }
}

// ------------------------------------------------------------------------------------------ parser

struct Abort;

type PResult<T> = Result<T, Abort>;

struct Parser {
    toks: Vec<Token>,
    pos: usize,
    diags: Diagnostics,
    module: Module,
}

/// Per-function name bindings while parsing a body.
struct Scope {
    values: HashMap<Name, Value>,
    /// Names defined in each open region, popped when the region closes.
    frames: Vec<Vec<Name>>,
    all_values: HashSet<Name>,
    vars: HashMap<Name, Var>,
    loop_depth: u32,
}

struct PendingBody {
    func: FuncId,
    params: Vec<(Name, Span)>,
    /// Token index just after the opening `{`.
    start: usize,
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }

    fn span(&self) -> Span {
        self.toks[self.pos].span
    }

    fn bump(&mut self) -> Tok {
        let t = self.toks[self.pos].tok.clone();
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }

    fn error_at<T>(&mut self, span: Span, msg: &str) -> PResult<T> {
        self.diags.error("ir.syntax", msg, Some(span));
        Err(Abort)
    }

    fn unexpected<T>(&mut self, what: &str) -> PResult<T> {
        let found = self.peek().describe();
        let span = self.span();
        self.error_at(span, &format!("expected {what}, found {found}"))
    }

    fn expect(&mut self, tok: Tok, what: &str) -> PResult<()> {
        if *self.peek() == tok {
            self.bump();
            Ok(())
        } else {
            self.unexpected(what)
        }
    }

    fn expect_word(&mut self, word: &str) -> PResult<()> {
        if matches!(self.peek(), Tok::Word(w) if w == word) {
            self.bump();
            Ok(())
        } else {
            self.unexpected(&format!("'{word}'"))
        }
    }

    fn at_word(&self, word: &str) -> bool {
        matches!(self.peek(), Tok::Word(w) if w == word)
    }

    fn skip_newlines(&mut self) {
        while *self.peek() == Tok::Newline {
            self.bump();
        }
    }

    fn end_of_line(&mut self) -> PResult<()> {
        match self.peek() {
            Tok::Newline => {
                self.skip_newlines();
                Ok(())
            }
            Tok::Eof => Ok(()),
            _ => self.unexpected("end of line"),
        }
    }

    fn ty(&mut self) -> PResult<Type> {
        if let Tok::Word(w) = self.peek()
            && let Some(t) = Type::from_name(w)
        {
            self.bump();
            return Ok(t);
        }
        self.unexpected("a type")
    }

    fn sym(&mut self) -> PResult<(String, Span)> {
        let span = self.span();
        match self.peek().clone() {
            Tok::Sym(s) => {
                self.bump();
                Ok((s, span))
            }
            _ => self.unexpected("a symbol name (@name)"),
        }
    }

    fn ret_type(&mut self) -> PResult<Option<Type>> {
        if *self.peek() == Tok::Arrow {
            self.bump();
            Ok(Some(self.ty()?))
        } else {
            Ok(None)
        }
    }

    // ---------------------------------------------------------------- items

    fn module(&mut self) {
        let mut pending = Vec::new();
        if self.header(&mut pending).is_err() {
            return;
        }
        for body in pending {
            self.pos = body.start;
            let _ = self.body(&body);
        }
    }

    fn header(&mut self, pending: &mut Vec<PendingBody>) -> PResult<()> {
        self.skip_newlines();
        self.expect_word("module")?;
        match self.bump() {
            Tok::Str(s) => match String::from_utf8(s) {
                Ok(name) => self.module.name = name,
                Err(_) => {
                    let span = self.toks[self.pos - 1].span;
                    return self.error_at(span, "module name is not valid UTF-8");
                }
            },
            _ => {
                self.pos -= 1;
                return self.unexpected("the module name as a string");
            }
        }
        self.end_of_line()?;
        loop {
            self.skip_newlines();
            let start = self.span();
            match self.peek().clone() {
                Tok::Eof => return Ok(()),
                Tok::Word(w) if w == "data" => {
                    self.bump();
                    let (name, _) = self.sym()?;
                    self.expect(Tok::Eq, "'='")?;
                    let bytes = match self.bump() {
                        Tok::Str(s) => s,
                        _ => {
                            self.pos -= 1;
                            return self.unexpected("a string");
                        }
                    };
                    let span = start.to(self.toks[self.pos - 1].span);
                    if self
                        .module
                        .add_global(&name, GlobalInit::Data(bytes), Some(span))
                        .is_none()
                    {
                        return self.error_at(span, &format!("symbol @{name} is already defined"));
                    }
                    self.end_of_line()?;
                }
                Tok::Word(w) if w == "global" => {
                    self.bump();
                    let (name, _) = self.sym()?;
                    self.expect(Tok::Colon, "':'")?;
                    let ty = self.ty()?;
                    self.expect(Tok::Eq, "'='")?;
                    let bits = self.scalar(ty)?;
                    let span = start.to(self.toks[self.pos - 1].span);
                    if self
                        .module
                        .add_global(&name, GlobalInit::Scalar(ty, bits), Some(span))
                        .is_none()
                    {
                        return self.error_at(span, &format!("symbol @{name} is already defined"));
                    }
                    self.end_of_line()?;
                }
                Tok::Word(w) if w == "extern" => {
                    self.bump();
                    self.expect_word("func")?;
                    let (name, _) = self.sym()?;
                    self.expect(Tok::LParen, "'('")?;
                    let mut params = Vec::new();
                    if *self.peek() != Tok::RParen {
                        loop {
                            params.push(self.ty()?);
                            if *self.peek() == Tok::Comma {
                                self.bump();
                            } else {
                                break;
                            }
                        }
                    }
                    self.expect(Tok::RParen, "')'")?;
                    let ret = self.ret_type()?;
                    let span = start.to(self.toks[self.pos - 1].span);
                    if self
                        .module
                        .add_func(&name, &params, ret, Some(span))
                        .is_none()
                    {
                        return self.error_at(span, &format!("symbol @{name} is already defined"));
                    }
                    self.end_of_line()?;
                }
                Tok::Word(w) if w == "func" => {
                    self.bump();
                    let (name, _) = self.sym()?;
                    self.expect(Tok::LParen, "'('")?;
                    let mut params = Vec::new();
                    let mut names = Vec::new();
                    if *self.peek() != Tok::RParen {
                        loop {
                            let span = self.span();
                            let n = match self.bump() {
                                Tok::Val(n) => n,
                                _ => {
                                    self.pos -= 1;
                                    return self.unexpected("a parameter (%name: type)");
                                }
                            };
                            self.expect(Tok::Colon, "':'")?;
                            params.push(self.ty()?);
                            names.push((n, span));
                            if *self.peek() == Tok::Comma {
                                self.bump();
                            } else {
                                break;
                            }
                        }
                    }
                    self.expect(Tok::RParen, "')'")?;
                    let ret = self.ret_type()?;
                    let span = start.to(self.toks[self.pos - 1].span);
                    let Some(f) = self.module.add_func(&name, &params, ret, Some(span)) else {
                        return self.error_at(span, &format!("symbol @{name} is already defined"));
                    };
                    self.expect(Tok::LBrace, "'{'")?;
                    pending.push(PendingBody {
                        func: f,
                        params: names,
                        start: self.pos,
                    });
                    self.skip_body()?;
                    self.end_of_line()?;
                }
                _ => return self.unexpected("'data', 'global', 'extern func' or 'func'"),
            }
        }
    }

    /// Skips to just after the `}` matching an already consumed `{`.
    fn skip_body(&mut self) -> PResult<()> {
        let mut depth = 1;
        loop {
            match self.bump() {
                Tok::LBrace => depth += 1,
                Tok::RBrace => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(());
                    }
                }
                Tok::Eof => return self.unexpected("'}'"),
                _ => {}
            }
        }
    }

    /// Parses a scalar initializer or constant of type `ty` and returns its stored bits.
    fn scalar(&mut self, ty: Type) -> PResult<u64> {
        let span = self.span();
        match (ty, self.peek().clone()) {
            (Type::Ptr, Tok::Word(w)) if w == "null" => {
                self.bump();
                Ok(0)
            }
            (Type::F32 | Type::F64, _) => self.float(ty),
            (_, Tok::Int(v)) if ty.is_int() || ty == Type::Ptr => {
                self.bump();
                if ty == Type::Ptr {
                    return Ok(v as u64);
                }
                let bits = ty.bits().unwrap();
                let fits = if bits == 1 {
                    (-1..=1).contains(&v)
                } else {
                    let min = -(1i128 << (bits - 1));
                    let max = (1i128 << bits) - 1;
                    (min..=max).contains(&v)
                };
                if !fits {
                    return self.error_at(span, &format!("{v} does not fit in {ty}"));
                }
                Ok(canonical_int(ty, v as i64) as u64)
            }
            _ => self.unexpected(&format!("a {ty} constant")),
        }
    }

    fn float(&mut self, ty: Type) -> PResult<u64> {
        let span = self.span();
        let tok = self.bump();
        let text = match &tok {
            Tok::Float(s) => s.clone(),
            Tok::Int(v) => v.to_string(),
            Tok::Word(w) if w == "inf" => "inf".to_string(),
            Tok::Word(w) if w == "nan" => {
                if *self.peek() == Tok::LParen {
                    self.bump();
                    let bits = match self.bump() {
                        Tok::Int(v) if v >= 0 => v as u64,
                        _ => {
                            return self
                                .error_at(span, "expected NaN bits like nan(0x7ff8000000000001)");
                        }
                    };
                    self.expect(Tok::RParen, "')'")?;
                    let is_nan = match ty {
                        Type::F32 => {
                            bits <= u32::MAX as u64 && f32::from_bits(bits as u32).is_nan()
                        }
                        _ => f64::from_bits(bits).is_nan(),
                    };
                    if !is_nan {
                        return self.error_at(span, "bits in nan(...) are not a NaN of this type");
                    }
                    return Ok(bits);
                }
                return Ok(match ty {
                    Type::F32 => F32_CANONICAL_NAN as u64,
                    _ => F64_CANONICAL_NAN,
                });
            }
            _ => {
                self.pos -= 1;
                return self.unexpected(&format!("a {ty} constant"));
            }
        };
        match ty {
            Type::F32 => match text.parse::<f32>() {
                Ok(f) => Ok(f.to_bits() as u64),
                Err(_) => self.error_at(span, "invalid float"),
            },
            _ => match text.parse::<f64>() {
                Ok(f) => Ok(f.to_bits()),
                Err(_) => self.error_at(span, "invalid float"),
            },
        }
    }

    // ---------------------------------------------------------------- bodies

    fn body(&mut self, pending: &PendingBody) -> PResult<()> {
        let f = pending.func;
        let entry = self.module.create_body(f);
        let mut scope = Scope {
            values: HashMap::new(),
            frames: vec![Vec::new()],
            all_values: HashSet::new(),
            vars: HashMap::new(),
            loop_depth: 0,
        };
        let params = self.module.func_param_values(f).to_vec();
        for ((name, span), v) in pending.params.iter().zip(params) {
            self.define(&mut scope, name.clone(), v, *span)?;
        }
        self.skip_newlines();
        while self.at_word("var") {
            self.bump();
            let span = self.span();
            let name = match self.bump() {
                Tok::VarRef(n) => n,
                _ => {
                    self.pos -= 1;
                    return self.unexpected("a variable name ($name)");
                }
            };
            self.expect(Tok::Colon, "':'")?;
            let ty = self.ty()?;
            if scope.vars.contains_key(&name) {
                return self.error_at(span, "variable is already declared");
            }
            let text_name = match &name {
                Name::Named(s) => Some(s.clone()),
                Name::Num(_) => None,
            };
            let var = self.module.new_var(f, ty, text_name);
            scope.vars.insert(name, var);
            self.end_of_line()?;
        }
        self.region(entry, &mut scope)?;
        self.expect(Tok::RBrace, "'}'")
    }

    fn define(&mut self, scope: &mut Scope, name: Name, v: Value, span: Span) -> PResult<()> {
        if !scope.all_values.insert(name.clone()) {
            return self.error_at(span, "value name is already used in this function");
        }
        if let Name::Named(s) = &name {
            self.module.values[v.index()].name = Some(s.clone());
        }
        scope.values.insert(name.clone(), v);
        scope.frames.last_mut().unwrap().push(name);
        Ok(())
    }

    /// Parses instructions into `region` until the closing `}` (not consumed).
    fn region(&mut self, region: Region, scope: &mut Scope) -> PResult<()> {
        loop {
            self.skip_newlines();
            if matches!(self.peek(), Tok::RBrace | Tok::Eof) {
                return Ok(());
            }
            self.inst(region, scope)?;
        }
    }

    fn nested(&mut self, region: Region, scope: &mut Scope) -> PResult<()> {
        self.expect(Tok::LBrace, "'{'")?;
        self.end_of_line()?;
        scope.frames.push(Vec::new());
        let r = self.region(region, scope);
        for name in scope.frames.pop().unwrap() {
            scope.values.remove(&name);
        }
        r?;
        self.expect(Tok::RBrace, "'}'")
    }

    fn value(&mut self, scope: &Scope) -> PResult<Value> {
        let span = self.span();
        match self.bump() {
            Tok::Val(n) => match scope.values.get(&n) {
                Some(&v) => Ok(v),
                None => {
                    let shown = match &n {
                        Name::Named(s) => s.clone(),
                        Name::Num(x) => x.to_string(),
                    };
                    self.error_at(
                        span,
                        &format!("unknown value %{shown} (not defined here or not in scope)"),
                    )
                }
            },
            _ => {
                self.pos -= 1;
                self.unexpected("a value (%name)")
            }
        }
    }

    fn var(&mut self, scope: &Scope) -> PResult<Var> {
        let span = self.span();
        match self.bump() {
            Tok::VarRef(n) => match scope.vars.get(&n) {
                Some(&v) => Ok(v),
                None => self.error_at(span, "unknown variable"),
            },
            _ => {
                self.pos -= 1;
                self.unexpected("a variable ($name)")
            }
        }
    }

    fn comma(&mut self) -> PResult<()> {
        self.expect(Tok::Comma, "','")
    }

    fn value_list(&mut self, scope: &Scope) -> PResult<Vec<Value>> {
        self.expect(Tok::LParen, "'('")?;
        let mut vals = Vec::new();
        if *self.peek() != Tok::RParen {
            loop {
                vals.push(self.value(scope)?);
                if *self.peek() == Tok::Comma {
                    self.bump();
                } else {
                    break;
                }
            }
        }
        self.expect(Tok::RParen, "')'")?;
        Ok(vals)
    }

    fn inst(&mut self, region: Region, scope: &mut Scope) -> PResult<()> {
        let start = self.span();
        // Optional result: `%name: type =`.
        let mut result = None;
        if let Tok::Val(name) = self.peek().clone() {
            self.bump();
            self.expect(Tok::Colon, "':'")?;
            let ty = self.ty()?;
            self.expect(Tok::Eq, "'='")?;
            result = Some((name, ty));
        }
        let op_span = self.span();
        let op = match self.peek() {
            Tok::Word(w) => match Opcode::from_name(w) {
                Some(op) => op,
                None => return self.unexpected("an opcode"),
            },
            _ => return self.unexpected("an instruction"),
        };
        self.bump();
        let result_ty = result.as_ref().map(|(_, t)| *t);
        let needs_result = !matches!(
            op,
            Opcode::Store
                | Opcode::Set
                | Opcode::If
                | Opcode::Loop
                | Opcode::Break
                | Opcode::Continue
                | Opcode::Return
                | Opcode::Unreachable
                | Opcode::Call
                | Opcode::CallIndirect
        );
        if needs_result && result.is_none() {
            return self.error_at(
                op_span,
                &format!("'{op}' produces a value; write '%name: type = {op} ...'"),
            );
        }
        if !needs_result && result.is_some() && !matches!(op, Opcode::Call | Opcode::CallIndirect) {
            return self.error_at(op_span, &format!("'{op}' does not produce a value"));
        }

        let mut args: Vec<Value> = Vec::new();
        let mut imm = 0u64;
        match op {
            Opcode::Iconst => {
                let ty = result_ty.unwrap();
                if !ty.is_int() {
                    return self.error_at(op_span, "iconst needs an integer result type");
                }
                imm = self.scalar(ty)?;
            }
            Opcode::Fconst => {
                let ty = result_ty.unwrap();
                if !ty.is_float() {
                    return self.error_at(op_span, "fconst needs a float result type");
                }
                imm = self.float(ty)?;
            }
            Opcode::Null | Opcode::Break | Opcode::Continue | Opcode::Unreachable => {}
            Opcode::Addr => {
                let (name, span) = self.sym()?;
                match self.module.lookup(&name) {
                    Some(sym) => imm = sym.to_imm(),
                    None => return self.error_at(span, &format!("unknown symbol @{name}")),
                }
            }
            Opcode::Icmp | Opcode::Fcmp => {
                let span = self.span();
                let cc = match self.bump() {
                    Tok::Word(w) => w,
                    _ => String::new(),
                };
                imm = if op == Opcode::Icmp {
                    match IntCC::from_name(&cc) {
                        Some(c) => c as u64,
                        None => {
                            return self
                                .error_at(span, "expected an integer condition (eq, ne, slt, ...)");
                        }
                    }
                } else {
                    match FloatCC::from_name(&cc) {
                        Some(c) => c as u64,
                        None => {
                            return self
                                .error_at(span, "expected a float condition (eq, ne, lt, ...)");
                        }
                    }
                };
                args.push(self.value(scope)?);
                self.comma()?;
                args.push(self.value(scope)?);
            }
            Opcode::Select => {
                args.push(self.value(scope)?);
                self.comma()?;
                args.push(self.value(scope)?);
                self.comma()?;
                args.push(self.value(scope)?);
            }
            Opcode::Store | Opcode::Ptradd => {
                args.push(self.value(scope)?);
                self.comma()?;
                args.push(self.value(scope)?);
            }
            op if op.is_int_binary() || op.is_float_binary() => {
                args.push(self.value(scope)?);
                self.comma()?;
                args.push(self.value(scope)?);
            }
            op if op.is_unary() || op.is_cast() || op == Opcode::Load => {
                args.push(self.value(scope)?);
            }
            Opcode::Call => {
                let (name, span) = self.sym()?;
                let Some(f) = self.module.lookup_func(&name) else {
                    return self.error_at(span, &format!("unknown function @{name}"));
                };
                imm = f.0 as u64;
                args = self.value_list(scope)?;
            }
            Opcode::CallIndirect => {
                args.push(self.value(scope)?);
                args.extend(self.value_list(scope)?);
                self.expect(Tok::Colon, "':'")?;
                self.expect(Tok::LParen, "'('")?;
                let mut params = Vec::new();
                if *self.peek() != Tok::RParen {
                    loop {
                        params.push(self.ty()?);
                        if *self.peek() == Tok::Comma {
                            self.bump();
                        } else {
                            break;
                        }
                    }
                }
                self.expect(Tok::RParen, "')'")?;
                let ret = self.ret_type()?;
                imm = self.module.intern_sig(&params, ret).0 as u64;
            }
            Opcode::Get => {
                imm = self.var(scope)?.0 as u64;
            }
            Opcode::Set => {
                imm = self.var(scope)?.0 as u64;
                self.comma()?;
                args.push(self.value(scope)?);
            }
            Opcode::Return => {
                if matches!(self.peek(), Tok::Val(_)) {
                    args.push(self.value(scope)?);
                }
            }
            Opcode::If => {
                args.push(self.value(scope)?);
            }
            Opcode::Loop => {}
            _ => unreachable!("every opcode is handled above"),
        }

        let span = start.to(self.toks[self.pos - 1].span);
        let inst = self.module.push_inst(region, op, &args, imm, result_ty);
        self.module.set_inst_span(inst, Some(span));

        match op {
            Opcode::If => {
                let then = self.module.new_region(self.module.inst_func(inst), inst);
                self.module.set_inst_region(inst, 0, then);
                self.nested(then, scope)?;
                if self.at_word("else") {
                    self.bump();
                    let els = self.module.new_region(self.module.inst_func(inst), inst);
                    self.module.set_inst_region(inst, 1, els);
                    self.nested(els, scope)?;
                }
            }
            Opcode::Loop => {
                let body = self.module.new_region(self.module.inst_func(inst), inst);
                self.module.set_inst_region(inst, 0, body);
                scope.loop_depth += 1;
                let r = self.nested(body, scope);
                scope.loop_depth -= 1;
                r?;
            }
            _ => {}
        }
        if let (Some((name, _)), Some(v)) = (result, self.module.inst_result(inst)) {
            self.define(scope, name, v, start)?;
        }
        self.end_of_line()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexes_numbers() {
        let t = lex("1 -2 3.5 -0.0 1e300 0xff -inf", SourceId(0)).unwrap();
        let toks: Vec<Tok> = t.into_iter().map(|t| t.tok).collect();
        assert_eq!(
            toks,
            vec![
                Tok::Int(1),
                Tok::Int(-2),
                Tok::Float("3.5".into()),
                Tok::Float("-0.0".into()),
                Tok::Float("1e300".into()),
                Tok::Int(255),
                Tok::Float("-inf".into()),
                Tok::Eof
            ]
        );
    }
}
