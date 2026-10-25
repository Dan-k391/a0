//! The canonical text form of a HIR module (see `docs/hir.md`).
//!
//! Operator precedence matches the `@prec` table in `hir.a0g`, so printed text parses back to the
//! same text: `print(parse(print(m))) == print(m)`.

use std::fmt::Write as _;

use crate::builder::ident;
use crate::model::*;

pub fn print_module(m: &HirModule) -> String {
    let mut p = Printer {
        m,
        out: String::new(),
    };
    let _ = writeln!(p.out, "module {};", quote(&m.name));
    // Types first, as the text format declares them before anything that uses them; otherwise
    // in declaration order, so printing what was parsed gives the same text.
    let is_type = |i: &&Item| matches!(i, Item::Struct(_) | Item::Enum(_));
    let types = m.items().iter().filter(is_type);
    let rest = m.items().iter().filter(|i| !is_type(i));
    // Modules may declare the same C function; the text declares it once.
    let mut externs = std::collections::HashSet::new();
    for item in types.chain(rest) {
        if let Item::Extern(x) = item
            && !externs.insert(m.extern_name(*x))
        {
            continue;
        }
        p.out.push('\n');
        p.item(*item);
    }
    p.out
}

impl std::fmt::Display for HirModule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&print_module(self))
    }
}

impl std::fmt::Debug for HirModule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&print_module(self))
    }
}

pub(crate) fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\0' => out.push_str("\\0"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                let _ = write!(out, "\\u{{{:x}}}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

// Precedence levels, from the `@prec` table in hir.a0g (higher binds tighter).
const ASSIGN: u8 = 1;
const OR: u8 = 2;
const AND: u8 = 3;
const EQUALITY: u8 = 4;
const COMPARISON: u8 = 5;
const ADDITIVE: u8 = 6;
const MULTIPLICATIVE: u8 = 7;
const AS: u8 = 8;
const IS: u8 = 9;
const PREFIX: u8 = 10;
const POSTFIX: u8 = 11;
const ATOM: u8 = 14;

fn binary_level(op: BinOp) -> u8 {
    match op {
        BinOp::Eq | BinOp::Ne => EQUALITY,
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => COMPARISON,
        BinOp::Add | BinOp::Sub => ADDITIVE,
        BinOp::Mul | BinOp::Div | BinOp::Rem => MULTIPLICATIVE,
    }
}

struct Printer<'a> {
    m: &'a HirModule,
    out: String,
}

impl Printer<'_> {
    fn ty(&self, t: TypeId) -> String {
        self.m.type_name(t)
    }

    fn item(&mut self, item: Item) {
        let m = self.m;
        match item {
            Item::Struct(s) => {
                let def = &m.structs[s.index()];
                let fields: Vec<String> = def
                    .fields
                    .iter()
                    .map(|(n, t)| format!("{}: {}", ident(n), self.ty(*t)))
                    .collect();
                let name = format!("{}{}", ident(&def.name), params(&def.type_params));
                if fields.is_empty() {
                    let _ = writeln!(self.out, "struct {name} {{}}");
                } else {
                    let _ = writeln!(self.out, "struct {name} {{ {} }}", fields.join(", "));
                }
            }
            Item::Enum(e) => {
                let def = &m.enums[e.index()];
                let variants: Vec<String> = def
                    .variants
                    .iter()
                    .map(|v| {
                        if v.fields.is_empty() {
                            ident(&v.name).into_owned()
                        } else {
                            let fs: Vec<String> = v.fields.iter().map(|&t| self.ty(t)).collect();
                            format!("{}({})", ident(&v.name), fs.join(", "))
                        }
                    })
                    .collect();
                let _ = writeln!(
                    self.out,
                    "enum {}{} {{ {} }}",
                    ident(&def.name),
                    params(&def.type_params),
                    variants.join(", ")
                );
            }
            Item::Global(g) => {
                let _ = writeln!(
                    self.out,
                    "global {}: {};",
                    ident(m.global_name(g)),
                    self.ty(m.global_type(g))
                );
            }
            Item::Extern(x) => {
                let def = &m.externs[x.index()];
                let ps: Vec<String> = def.params.iter().map(|&t| self.ty(t)).collect();
                let abi = match def.abi {
                    Abi::C => String::new(),
                    abi => format!("\"{}\" ", abi.name()),
                };
                let _ = writeln!(
                    self.out,
                    "extern {abi}fn {}({}){};",
                    ident(&def.name),
                    ps.join(", "),
                    self.ret(def.ret)
                );
            }
            Item::Func(f) => {
                let body = m.func_body(f);
                let _ = write!(
                    self.out,
                    "fn {}{}",
                    ident(m.func_name(f)),
                    params(&m.funcs[f.index()].type_params)
                );
                self.signature(body);
                self.out.push(' ');
                self.body_root(body, 0);
                self.out.push('\n');
            }
        }
    }

    fn ret(&self, t: TypeId) -> String {
        if t == TypeId::UNIT {
            String::new()
        } else {
            format!(" -> {}", self.ty(t))
        }
    }

    fn signature(&mut self, body: BodyId) {
        let m = self.m;
        let ps: Vec<String> = m
            .body_params(body)
            .iter()
            .map(|&l| format!("{}: {}", m.local_name(l), self.ty(m.local_type(l))))
            .collect();
        let _ = write!(
            self.out,
            "({}){}",
            ps.join(", "),
            self.ret(m.body_ret(body))
        );
    }

    fn body_root(&mut self, body: BodyId, indent: usize) {
        match self.m.body_root(body) {
            Some(root) => self.as_block(root, indent),
            None => self.out.push_str("{}"),
        }
    }

    fn newline(&mut self, indent: usize) {
        self.out.push('\n');
        for _ in 0..indent {
            self.out.push_str("  ");
        }
    }

    /// Prints `e` as a `{ ... }` block (wrapping it if it is not one).
    fn as_block(&mut self, e: ExprId, indent: usize) {
        match &self.m.expr(e).kind {
            ExprKind::Block(items, tail) => {
                let (items, tail) = (items.clone(), *tail);
                self.block(&items, tail, indent);
            }
            _ => self.block(&[], Some(e), indent),
        }
    }

    fn block(&mut self, items: &[ExprId], tail: Option<ExprId>, indent: usize) {
        if items.is_empty() && tail.is_none() {
            self.out.push_str("{}");
            return;
        }
        self.out.push('{');
        for (i, &item) in items.iter().enumerate() {
            self.newline(indent + 1);
            self.statement(item, indent + 1);
            // A block-like statement needs no `;` unless the next thing could be read as continuing it.
            let next = items.get(i + 1).copied().or(tail);
            let needs_semicolon = !is_block_like(self.m, item)
                || match next {
                    None => true,
                    Some(n) => self.starts_ambiguously(n),
                };
            if needs_semicolon {
                self.out.push(';');
            }
        }
        if let Some(t) = tail {
            self.newline(indent + 1);
            self.expr(t, 0, indent + 1);
        }
        self.newline(indent);
        self.out.push('}');
    }

    /// Whether `e`'s text begins with a token that could continue a preceding expression (`-`, `(`,
    /// `[`), in which case the statement before it needs a `;`.
    fn starts_ambiguously(&self, e: ExprId) -> bool {
        let m = self.m;
        // The leftmost operand is parenthesized when it binds less tightly than its parent needs.
        let left = |a: ExprId, min: u8| self.level(a) < min || self.starts_ambiguously(a);
        match &m.expr(e).kind {
            ExprKind::Unary(UnOp::Neg, _) | ExprKind::Array(_) | ExprKind::Unit => true,
            ExprKind::Int(v) => *v < 0,
            ExprKind::Float(bits) => {
                let f = f64::from_bits(*bits);
                f.is_sign_negative() && f.is_finite()
            }
            ExprKind::Binary(op, a, _) => left(*a, binary_level(*op)),
            ExprKind::And(a, _) => left(*a, AND),
            ExprKind::Or(a, _) => left(*a, OR),
            ExprKind::Cast(a) => left(*a, AS),
            ExprKind::Is(a, _) => left(*a, IS),
            ExprKind::Field(a, _)
            | ExprKind::FieldName(a, _)
            | ExprKind::Unwrap(a)
            | ExprKind::Index(a, _)
            | ExprKind::CallValue(a, _)
            | ExprKind::SetField(a, _, _)
            | ExprKind::SetFieldName(a, _, _)
            | ExprKind::SetIndex(a, _, _) => left(*a, POSTFIX),
            _ => false,
        }
    }

    fn statement(&mut self, e: ExprId, indent: usize) {
        let m = self.m;
        if let ExprKind::Let(l, init) = &m.expr(e).kind {
            let _ = write!(
                self.out,
                "let {}: {} = ",
                m.local_name(*l),
                self.ty(m.local_type(*l))
            );
            self.expr(*init, ASSIGN, indent);
            return;
        }
        self.expr(e, 0, indent);
    }

    fn level(&self, e: ExprId) -> u8 {
        match &self.m.expr(e).kind {
            ExprKind::SetLocal(..)
            | ExprKind::SetGlobal(..)
            | ExprKind::SetField(..)
            | ExprKind::SetFieldName(..)
            | ExprKind::SetIndex(..) => ASSIGN,
            ExprKind::Or(..) => OR,
            ExprKind::And(..) => AND,
            ExprKind::Binary(op, ..) => binary_level(*op),
            ExprKind::Cast(_) => AS,
            ExprKind::Is(..) => IS,
            ExprKind::Unary(..) => PREFIX,
            ExprKind::Int(v) if *v < 0 => PREFIX,
            ExprKind::Float(bits)
                if f64::from_bits(*bits).is_sign_negative()
                    && f64::from_bits(*bits).is_finite() =>
            {
                PREFIX
            }
            ExprKind::CallValue(..)
            | ExprKind::Field(..)
            | ExprKind::FieldName(..)
            | ExprKind::Unwrap(_)
            | ExprKind::Index(..) => POSTFIX,
            // `return x` swallows everything after it, so it is parenthesized inside operators.
            ExprKind::Return(Some(_)) => ASSIGN,
            _ => ATOM,
        }
    }

    /// Prints `e`, parenthesized if it binds less tightly than `min`.
    fn expr(&mut self, e: ExprId, min: u8, indent: usize) {
        if self.level(e) < min {
            self.out.push('(');
            self.expr(e, 0, indent);
            self.out.push(')');
            return;
        }
        let m = self.m;
        let kind = m.expr(e).kind.clone();
        match kind {
            ExprKind::Unit => self.out.push_str("()"),
            ExprKind::Bool(b) => self.out.push_str(if b { "true" } else { "false" }),
            ExprKind::Int(v) => {
                let _ = write!(self.out, "{v}");
            }
            ExprKind::Float(bits) => {
                let f = f64::from_bits(bits);
                if f.is_finite() {
                    let _ = write!(self.out, "{f:?}");
                } else {
                    let _ = write!(self.out, "float_bits(0x{bits:016x})");
                }
            }
            ExprKind::Str(s) => self.out.push_str(&quote(&s)),
            ExprKind::Null => {
                let _ = write!(self.out, "null<{}>", self.ty(m.non_null(m.expr_type(e))));
            }
            ExprKind::Local(l) => self.out.push_str(m.local_name(l)),
            ExprKind::Global(g) => self.out.push_str(&ident(m.global_name(g))),
            ExprKind::Func(f, targs) => {
                self.out.push_str(&ident(m.func_name(f)));
                self.type_args(&targs);
            }
            ExprKind::Let(..) => self.statement(e, indent),
            ExprKind::SetLocal(l, v) => {
                let _ = write!(self.out, "{} = ", m.local_name(l));
                self.expr(v, ASSIGN, indent);
            }
            ExprKind::SetGlobal(g, v) => {
                let _ = write!(self.out, "{} = ", ident(m.global_name(g)));
                self.expr(v, ASSIGN, indent);
            }
            ExprKind::SetField(o, f, v) => {
                self.expr(o, POSTFIX, indent);
                let _ = write!(self.out, ".{} = ", self.field_name(o, f));
                self.expr(v, ASSIGN, indent);
            }
            ExprKind::SetFieldName(o, name, v) => {
                self.expr(o, POSTFIX, indent);
                let _ = write!(self.out, ".{} = ", ident(&name));
                self.expr(v, ASSIGN, indent);
            }
            ExprKind::SetIndex(t, i, v) => {
                self.expr(t, POSTFIX, indent);
                self.out.push('[');
                self.expr(i, 0, indent);
                self.out.push_str("] = ");
                self.expr(v, ASSIGN, indent);
            }
            ExprKind::Unary(op, a) => {
                self.out.push(if op == UnOp::Neg { '-' } else { '!' });
                // `- -x`, not `--x`, for readability.
                if op == UnOp::Neg && self.starts_with_minus(a) {
                    self.out.push(' ');
                }
                self.expr(a, PREFIX, indent);
            }
            ExprKind::Binary(op, a, b) => {
                let l = binary_level(op);
                self.expr(a, l, indent);
                let _ = write!(self.out, " {} ", op.symbol());
                self.expr(b, l + 1, indent);
            }
            ExprKind::And(a, b) => {
                self.expr(a, AND, indent);
                self.out.push_str(" && ");
                self.expr(b, AND + 1, indent);
            }
            ExprKind::Or(a, b) => {
                self.expr(a, OR, indent);
                self.out.push_str(" || ");
                self.expr(b, OR + 1, indent);
            }
            ExprKind::Call(f, targs, args) => {
                self.out.push_str(&ident(m.func_name(f)));
                self.type_args(&targs);
                self.args(&args, indent);
            }
            ExprKind::CallExtern(x, args) => {
                self.out.push_str(&ident(m.extern_name(x)));
                self.args(&args, indent);
            }
            ExprKind::CallValue(c, args) => {
                self.expr(c, POSTFIX, indent);
                self.args(&args, indent);
            }
            ExprKind::Builtin(op, args) => {
                self.out.push_str(op.name());
                self.args(&args, indent);
            }
            ExprKind::Lambda(body) => {
                self.out.push_str("fn");
                self.signature(body);
                self.out.push(' ');
                self.body_root(body, indent);
            }
            ExprKind::New(s, fields) => {
                let def = &m.structs[s.index()];
                let name = self.ty(m.expr_type(e));
                if fields.is_empty() {
                    let _ = write!(self.out, "new {name} {{}}");
                    return;
                }
                let _ = write!(self.out, "new {name} {{ ");
                for (i, (&v, (name, _))) in fields.iter().zip(&def.fields).enumerate() {
                    if i > 0 {
                        self.out.push_str(", ");
                    }
                    let _ = write!(self.out, "{name}: ");
                    self.expr(v, 0, indent);
                }
                self.out.push_str(" }");
            }
            ExprKind::Field(o, f) => {
                self.expr(o, POSTFIX, indent);
                let _ = write!(self.out, ".{}", self.field_name(o, f));
            }
            ExprKind::FieldName(o, name) => {
                self.expr(o, POSTFIX, indent);
                let _ = write!(self.out, ".{}", ident(&name));
            }
            ExprKind::Unwrap(o) => {
                self.expr(o, POSTFIX, indent);
                self.out.push('!');
            }
            ExprKind::Variant(en, v, args) => {
                let _ = write!(
                    self.out,
                    "{}::{}",
                    self.ty(m.expr_type(e)),
                    ident(m.variant_name(en, v))
                );
                if !args.is_empty() {
                    self.args(&args, indent);
                }
            }
            ExprKind::Match(s, arms) => {
                self.out.push_str("match ");
                self.expr(s, 0, indent);
                self.out.push_str(" {");
                let en = match m.kind(m.expr_type(s)) {
                    TypeKind::Enum(en, _) => Some(*en),
                    _ => None,
                };
                for a in arms {
                    let arm = &m.arms[a.index()];
                    self.newline(indent + 1);
                    match (arm.variant, en) {
                        (Some(v), Some(en)) => {
                            self.out.push_str(&ident(m.variant_name(en, v)));
                            if !arm.binds.is_empty() {
                                let names: Vec<&str> =
                                    arm.binds.iter().map(|&l| m.local_name(l)).collect();
                                let _ = write!(self.out, "({})", names.join(", "));
                            }
                        }
                        _ => self.out.push('_'),
                    }
                    self.out.push_str(" => ");
                    self.expr(arm.body, 0, indent + 1);
                    self.out.push(',');
                }
                self.newline(indent);
                self.out.push('}');
            }
            // An empty literal's type came from inference; print it explicitly.
            ExprKind::Array(elems) if elems.is_empty() => {
                let _ = write!(self.out, "new {}", self.ty(m.expr_type(e)));
            }
            ExprKind::Array(elems) => {
                self.out.push('[');
                for (i, &el) in elems.iter().enumerate() {
                    if i > 0 {
                        self.out.push_str(", ");
                    }
                    self.expr(el, 0, indent);
                }
                self.out.push(']');
            }
            ExprKind::NewArray | ExprKind::NewMap => {
                let _ = write!(self.out, "new {}", self.ty(m.expr_type(e)));
            }
            ExprKind::Index(t, i) => {
                self.expr(t, POSTFIX, indent);
                self.out.push('[');
                self.expr(i, 0, indent);
                self.out.push(']');
            }
            ExprKind::Cast(a) => {
                self.expr(a, AS, indent);
                let _ = write!(self.out, " as {}", self.ty(m.expr_type(e)));
            }
            ExprKind::Is(a, t) => {
                self.expr(a, IS, indent);
                let _ = write!(self.out, " is {}", self.ty(t));
            }
            ExprKind::If(..) | ExprKind::IfLet(..) => {
                let (t, els) = match m.expr(e).kind {
                    ExprKind::If(c, t, els) => {
                        self.out.push_str("if ");
                        self.expr(c, 0, indent);
                        (t, els)
                    }
                    ExprKind::IfLet(l, v, t, els) => {
                        let _ = write!(self.out, "if let {} = ", ident(m.local_name(l)));
                        self.expr(v, 0, indent);
                        (t, els)
                    }
                    _ => unreachable!(),
                };
                self.out.push(' ');
                self.as_block(t, indent);
                if let Some(els) = els {
                    self.out.push_str(" else ");
                    if matches!(m.expr(els).kind, ExprKind::If(..) | ExprKind::IfLet(..)) {
                        self.expr(els, 0, indent);
                    } else {
                        self.as_block(els, indent);
                    }
                }
            }
            ExprKind::While(c, body) => {
                self.out.push_str("while ");
                self.expr(c, 0, indent);
                self.out.push(' ');
                self.as_block(body, indent);
            }
            ExprKind::Loop(body) => {
                self.out.push_str("loop ");
                self.as_block(body, indent);
            }
            ExprKind::Break => self.out.push_str("break"),
            ExprKind::Continue => self.out.push_str("continue"),
            ExprKind::Return(v) => match v {
                Some(v) => {
                    self.out.push_str("return ");
                    self.expr(v, 0, indent);
                }
                None => self.out.push_str("return"),
            },
            ExprKind::Block(items, tail) => self.block(&items, tail, indent),
            ExprKind::Throw(v) => {
                self.out.push_str("throw ");
                self.expr(v, 0, indent);
            }
            ExprKind::Try(body, l, handler) => {
                self.out.push_str("try ");
                self.as_block(body, indent);
                let _ = write!(self.out, " catch {} ", ident(m.local_name(l)));
                self.as_block(handler, indent);
            }
        }
    }

    fn starts_with_minus(&self, e: ExprId) -> bool {
        match &self.m.expr(e).kind {
            ExprKind::Unary(UnOp::Neg, _) => true,
            ExprKind::Int(v) => *v < 0,
            ExprKind::Float(bits) => {
                f64::from_bits(*bits).is_sign_negative() && f64::from_bits(*bits).is_finite()
            }
            _ => false,
        }
    }

    fn field_name(&self, object: ExprId, field: u32) -> String {
        match self.m.kind(self.m.expr_type(object)) {
            TypeKind::Struct(s, _) => self.m.structs[s.index()]
                .fields
                .get(field as usize)
                .map(|(n, _)| ident(n).into_owned())
                .unwrap_or_else(|| format!("field{field}")),
            _ => format!("field{field}"),
        }
    }

    /// `::<A, B>` after a generic function's name.
    fn type_args(&mut self, targs: &[TypeId]) {
        if !targs.is_empty() {
            let names: Vec<String> = targs.iter().map(|&t| self.ty(t)).collect();
            let _ = write!(self.out, "::<{}>", names.join(", "));
        }
    }

    fn args(&mut self, args: &[ExprId], indent: usize) {
        self.out.push('(');
        for (i, &a) in args.iter().enumerate() {
            if i > 0 {
                self.out.push_str(", ");
            }
            self.expr(a, 0, indent);
        }
        self.out.push(')');
    }
}

/// `<A, B>` after a generic item's name.
fn params(names: &[String]) -> String {
    if names.is_empty() {
        String::new()
    } else {
        let names: Vec<_> = names.iter().map(|n| ident(n)).collect();
        format!("<{}>", names.join(", "))
    }
}

/// Expressions that end in a `}` and can stand as statements without a `;`.
fn is_block_like(m: &HirModule, e: ExprId) -> bool {
    matches!(
        m.expr(e).kind,
        ExprKind::If(..)
            | ExprKind::IfLet(..)
            | ExprKind::While(..)
            | ExprKind::Loop(_)
            | ExprKind::Match(..)
            | ExprKind::Block(..)
            | ExprKind::Try(..)
    )
}
