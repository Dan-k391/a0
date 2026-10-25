//! Languages defined by their grammar alone (D55): each rule says what its nodes mean with
//! `@hir(form)`, or is named after a form (`binary`, `call`, ...), and this module builds HIR from
//! the syntax tree by those forms. Names, types, errors, garbage collection, both backends and the
//! language server then come from A0, with no code of the language's own.
//!
//! Forms and the fields they read (`docs/grammar.md`, "Languages without host code"):
//!
//! | Form | Fields |
//! |---|---|
//! | `program` | its children: `function`s, then statements, which are `main` |
//! | `function` | `name`, parameters (`param` fields or `param` nodes with `name` and `type`), result `type`, `body` |
//! | `block` | its children, statements, in a scope of their own |
//! | `let` | `name`, `value`, `type` |
//! | `assign` | `name`, `value` |
//! | `print` | `value`, one or more, printed on a line separated by spaces |
//! | `if`, `while` | `cond`, `then`, `else`; `cond`, `body` |
//! | `repeat` | `count`, `body`: the body `count` times |
//! | `return` | `value` |
//! | `binary`, `unary` | `left`, `op`, `right`; `op`, `operand` |
//! | `call` | `callee` (or `operand`), `arg`s (or an `args` node of `arg`s) |
//! | `name`, `number`, `int`, `float`, `string`, `bool` | the node's text |
//! | `paren` | `inner` |
//!
//! A node without a form that has a single child means that child; tokens named `NUMBER`, `INT`,
//! `FLOAT`, `STRING`, `NAME`, `IDENT` or `IDENTIFIER` and the literals `true` and `false` mean
//! what they say. Numbers are floats. The standard library's mathematics (`sqrt`, `floor`, ...)
//! and the HIR's builtins (`str`, `len`, ...) can be called.

use a0_diag::codes as code;
use a0_diag::{Diagnostics, Span};
use a0_syntax::{Grammar, NodeId, SyntaxTree};

use crate::builder::HirBuilder;
use crate::model::*;

/// The forms rules can give their nodes, whose names also apply to rules without `@hir`.
pub const FORMS: &[&str] = &[
    "program", "function", "param", "block", "let", "assign", "print", "if", "while", "repeat",
    "return", "eval", "binary", "unary", "call", "name", "number", "int", "float", "string",
    "bool", "paren",
];

/// The mathematics of `std.math` a program can call by name.
const MATH: &[&str] = &[
    "sqrt", "cbrt", "exp", "log", "log2", "log10", "pow", "sin", "cos", "tan", "atan2", "floor",
    "ceil", "round", "trunc", "abs", "min", "max", "pi", "e",
];

/// Builds the program in `tree`, parsed with `grammar`, by the grammar's forms.
pub fn compile(grammar: &Grammar, tree: &SyntaxTree, name: &str) -> Result<HirModule, Diagnostics> {
    let unknown = check_forms(grammar);
    if unknown.has_errors() {
        return Err(unknown);
    }
    let mut b = HirBuilder::new(name);
    for f in MATH {
        b.import_item("std.math", f);
    }
    Dsl {
        g: grammar,
        tree,
        b: &mut b,
    }
    .program();
    let m = b.finish()?;
    let entry = crate::check_entry(&m);
    if entry.has_errors() {
        Err(entry)
    } else {
        Ok(m)
    }
}

/// Reports `@hir` forms that are not [`FORMS`] (the grammar layer does not know them).
pub fn check_forms(grammar: &Grammar) -> Diagnostics {
    let mut d = Diagnostics::new();
    for k in 0..grammar.kind_count() {
        let kind = a0_syntax::KindId(k);
        if let Some(form) = grammar.hir_form(kind)
            && !FORMS.contains(&form)
        {
            d.error(
                code::SYNTAX,
                &format!(
                    "the rule `{}` has the @hir form `{form}`, which is not one of {}",
                    grammar.kind_name(kind),
                    FORMS.join(", ")
                ),
                None,
            );
        }
    }
    d
}

struct Dsl<'a> {
    g: &'a Grammar,
    tree: &'a SyntaxTree,
    b: &'a mut HirBuilder,
}

impl<'a> Dsl<'a> {
    fn text(&self, n: NodeId) -> &'a str {
        self.tree.text(n)
    }

    fn field(&self, n: NodeId, name: &str) -> Option<NodeId> {
        self.tree.field_named(n, name)
    }

    fn at(&mut self, n: NodeId) {
        self.b.set_span(Some(self.tree.span(n)));
    }

    /// What `n` means: its rule's `@hir` form, its rule's name if that is a form, or what its
    /// token kind says.
    fn form(&self, n: NodeId) -> Option<&str> {
        let kind = self.tree.kind(n);
        if self.tree.is_token(n) {
            let name = self.tree.kind_name(n);
            return match name.to_ascii_lowercase().as_str() {
                "number" => Some("number"),
                "int" | "integer" => Some("int"),
                "float" => Some("float"),
                "string" => Some("string"),
                "name" | "ident" | "identifier" => Some("name"),
                _ if matches!(self.text(n), "true" | "false") => Some("bool"),
                _ => None,
            };
        }
        if let Some(form) = self.g.hir_form(kind) {
            return Some(form);
        }
        let name = self.tree.kind_name(n);
        FORMS.contains(&name).then_some(name).map(|f| {
            // The static string, not the tree's.
            FORMS.iter().find(|&&x| x == f).copied().unwrap_or(f)
        })
    }

    /// `n` when it has a form, or the one child that means it.
    fn meaning(&self, mut n: NodeId) -> (NodeId, Option<&'static str>) {
        loop {
            if let Some(form) = self.form(n) {
                let form = FORMS.iter().find(|&&f| f == form).copied();
                return (n, form);
            }
            let children = if self.tree.is_token(n) {
                Vec::new()
            } else {
                self.tree.nodes(n)
            };
            match children.as_slice() {
                [only] => n = *only,
                [] if !self.tree.is_token(n) => {
                    // A node of one token: the token says.
                    let tokens: Vec<NodeId> = self.tree.children(n);
                    match tokens.as_slice() {
                        [only] => n = *only,
                        _ => return (n, None),
                    }
                }
                _ => return (n, None),
            }
        }
    }

    /// Reports that the grammar gives `n` no meaning.
    fn meaningless(&mut self, n: NodeId) -> ExprId {
        self.at(n);
        let what = if self.tree.is_token(n) {
            format!("the token `{}`", self.text(n))
        } else {
            format!("a `{}`", self.tree.kind_name(n))
        };
        self.b.report_code(
            code::SYNTAX,
            &format!(
                "the grammar does not say what {what} means here (give its rule an @hir form)"
            ),
        );
        self.b.report_primary_label("no meaning");
        self.b.error_expr()
    }

    fn missing(&mut self, n: NodeId, form: &str, field: &str) -> ExprId {
        self.at(n);
        self.b.report_code(
            code::SYNTAX,
            &format!("the @hir form `{form}` needs a field `{field}`, and this node has none"),
        );
        self.b.error_expr()
    }

    // ---------------------------------------------------------------- program and functions

    fn program(&mut self) {
        let root = self.tree.root();
        let items = self.tree.nodes(root);
        let mut functions = Vec::new();
        let mut statements = Vec::new();
        for item in items {
            match self.meaning(item) {
                (n, Some("function")) => {
                    if let Some(f) = self.declare_function(n) {
                        functions.push((f, n));
                    }
                }
                _ => statements.push(item),
            }
        }
        let main = self.b.declare_function("main", &[], &[], TypeId::UNIT);
        let body = self.b.function_body(main);
        self.b.begin_body(body);
        let items: Vec<ExprId> = statements.into_iter().map(|s| self.stmt(s)).collect();
        let root = self.b.block(&items, None);
        self.b.end_body();
        self.b.set_root(body, root);
        for (f, n) in functions {
            let body = self.b.function_body(f);
            self.b.begin_body(body);
            let root = match self.field(n, "body") {
                Some(block) => self.stmt(block),
                None => self.missing(n, "function", "body"),
            };
            self.b.end_body();
            self.b.set_root(body, root);
        }
    }

    fn declare_function(&mut self, n: NodeId) -> Option<FuncId> {
        let Some(name) = self.field(n, "name") else {
            self.missing(n, "function", "name");
            return None;
        };
        let mut params: Vec<NodeId> = self.tree.fields_named(n, "param");
        let nodes: Vec<NodeId> = self
            .tree
            .nodes(n)
            .into_iter()
            .filter(|&c| self.meaning(c).1 == Some("param") && !params.contains(&c))
            .collect();
        params.extend(nodes);
        let mut names = Vec::new();
        let mut types = Vec::new();
        for &p in &params {
            let (name, ty) = match self.field(p, "name") {
                Some(name) if !self.tree.is_token(p) => (name, self.field(p, "type")),
                _ => (p, None),
            };
            names.push(self.text(name).to_string());
            types.push(match ty {
                Some(t) => self.ty(t),
                None => self.b.fresh_type(),
            });
        }
        let ret = match self.field(n, "type") {
            Some(t) => self.ty(t),
            None if self.returns_value(n) => self.b.fresh_type(),
            None => TypeId::UNIT,
        };
        self.at(name);
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let f = self.b.declare_function(self.text(name), &refs, &types, ret);
        let body = self.b.function_body(f);
        for (i, &p) in params.iter().enumerate() {
            let at = self
                .field(p, "name")
                .filter(|_| !self.tree.is_token(p))
                .unwrap_or(p);
            let span: Span = self.tree.span(at);
            self.b.set_param_span(body, i as u32, span);
        }
        Some(f)
    }

    /// Whether a function's body returns a value somewhere.
    fn returns_value(&self, n: NodeId) -> bool {
        let mut stack = vec![n];
        while let Some(x) = stack.pop() {
            if self.tree.is_token(x) {
                continue;
            }
            if self.form(x) == Some("return") && self.field(x, "value").is_some() {
                return true;
            }
            stack.extend(self.tree.nodes(x));
        }
        false
    }

    fn ty(&mut self, t: NodeId) -> TypeId {
        let name = match self.field(t, "name") {
            Some(n) if !self.tree.is_token(t) => self.text(n).to_string(),
            _ => self.text(t).trim().to_string(),
        };
        self.at(t);
        self.b.named_type(&name, &[])
    }

    // ---------------------------------------------------------------- statements

    fn stmt(&mut self, n: NodeId) -> ExprId {
        let (n, form) = self.meaning(n);
        let value = |d: &mut Self, field: &str, form: &str| match d.field(n, field) {
            Some(v) => d.expr(v),
            None => d.missing(n, form, field),
        };
        match form {
            Some("block") => {
                self.b.push_scope();
                let items: Vec<ExprId> = self
                    .tree
                    .nodes(n)
                    .into_iter()
                    .map(|s| self.stmt(s))
                    .collect();
                self.b.pop_scope();
                self.at(n);
                self.b.block(&items, None)
            }
            Some("let") => {
                let v = value(self, "value", "let");
                let ty = match self.field(n, "type") {
                    Some(t) => self.ty(t),
                    None => self.b.fresh_type(),
                };
                let Some(name) = self.field(n, "name") else {
                    return self.missing(n, "let", "name");
                };
                self.at(name);
                self.b.let_named(self.text(name), ty, v)
            }
            Some("assign") => {
                let v = value(self, "value", "assign");
                let Some(name) = self.field(n, "name").or_else(|| self.field(n, "target")) else {
                    return self.missing(n, "assign", "name");
                };
                self.at(name);
                let name = self.meaning(name).0;
                self.b.assign_named(self.text(name), v)
            }
            Some("print") => {
                let values = self.tree.fields_named(n, "value");
                let mut items = Vec::new();
                for (i, v) in values.into_iter().enumerate() {
                    if i > 0 {
                        let space = self.b.string(" ");
                        items.push(self.b.builtin(Builtin::Print, &[space]));
                    }
                    let e = self.expr(v);
                    self.at(v);
                    items.push(self.b.builtin(Builtin::Print, &[e]));
                }
                let newline = self.b.string("\n");
                items.push(self.b.builtin(Builtin::Print, &[newline]));
                self.at(n);
                self.b.block(&items, None)
            }
            Some("if") => {
                let c = value(self, "cond", "if");
                let then = match self.field(n, "then") {
                    Some(t) => self.stmt(t),
                    None => self.missing(n, "if", "then"),
                };
                let els = self.field(n, "else").map(|e| self.stmt(e));
                self.at(n);
                self.b.if_(c, then, els)
            }
            Some("while") => {
                let c = value(self, "cond", "while");
                let body = match self.field(n, "body") {
                    Some(b) => self.stmt(b),
                    None => self.missing(n, "while", "body"),
                };
                self.at(n);
                self.b.while_(c, body)
            }
            Some("repeat") => self.repeat(n),
            Some("return") => {
                let v = self.field(n, "value").map(|v| self.expr(v));
                self.at(n);
                self.b.return_(v)
            }
            Some("eval") => {
                let inner = self.tree.nodes(n);
                match inner.first() {
                    Some(&e) => self.expr(e),
                    None => self.meaningless(n),
                }
            }
            _ => self.expr(n),
        }
    }

    /// `repeat count { body }`: `{ let n = count as int; let i = 0; while i < n { i = i + 1;
    /// body } }`, with names a program cannot write.
    fn repeat(&mut self, n: NodeId) -> ExprId {
        let count = match self.field(n, "count") {
            Some(c) => self.expr(c),
            None => return self.missing(n, "repeat", "count"),
        };
        self.at(n);
        self.b.push_scope();
        let count = self.b.cast(count, TypeId::INT);
        let int = TypeId::INT;
        let limit = self.b.let_named(" count", int, count);
        let zero = self.b.int(0);
        let index = self.b.let_named(" index", int, zero);
        let i = self.b.name(" index");
        let limit_value = self.b.name(" count");
        let cond = self.b.binary(BinOp::Lt, i, limit_value);
        let i = self.b.name(" index");
        let one = self.b.int(1);
        let next = self.b.binary(BinOp::Add, i, one);
        let step = self.b.assign_named(" index", next);
        let body = match self.field(n, "body") {
            Some(b) => self.stmt(b),
            None => self.missing(n, "repeat", "body"),
        };
        self.at(n);
        let body = self.b.block(&[step, body], None);
        let lp = self.b.while_(cond, body);
        self.b.pop_scope();
        self.b.block(&[limit, index, lp], None)
    }

    // ---------------------------------------------------------------- expressions

    fn expr(&mut self, n: NodeId) -> ExprId {
        let (n, form) = self.meaning(n);
        let sub = |d: &mut Self, field: &str, form: &str| match d.field(n, field) {
            Some(v) => d.expr(v),
            None => d.missing(n, form, field),
        };
        match form {
            Some("binary") => {
                let (l, r) = (sub(self, "left", "binary"), sub(self, "right", "binary"));
                let op = self
                    .field(n, "op")
                    .map(|o| self.text(o).to_string())
                    .unwrap_or_default();
                self.at(n);
                match op.as_str() {
                    "&&" | "and" => self.b.and(l, r),
                    "||" | "or" => self.b.or(l, r),
                    _ => match BinOp::from_symbol(&op) {
                        Some(op) => self.b.binary(op, l, r),
                        None => {
                            self.b.report_code(
                                code::BAD_OPERAND,
                                &format!("`{op}` is not an operator the HIR knows"),
                            );
                            self.b.error_expr()
                        }
                    },
                }
            }
            Some("unary") => {
                let a = sub(self, "operand", "unary");
                let op = self
                    .field(n, "op")
                    .map(|o| self.text(o).to_string())
                    .unwrap_or_default();
                self.at(n);
                match op.as_str() {
                    "-" => self.b.unary(UnOp::Neg, a),
                    _ => self.b.unary(UnOp::Not, a),
                }
            }
            Some("call") => {
                let Some(callee) = self.field(n, "callee").or_else(|| self.field(n, "operand"))
                else {
                    return self.missing(n, "call", "callee");
                };
                let mut args = self.tree.fields_named(n, "arg");
                if let Some(list) = self.field(n, "args") {
                    args.extend(self.tree.fields_named(list, "arg"));
                }
                let args: Vec<ExprId> = args.into_iter().map(|a| self.expr(a)).collect();
                let callee = self.meaning(callee).0;
                self.at(n);
                self.b.set_name_span(Some(self.tree.span(callee)));
                self.b.call_named(self.text(callee), &args)
            }
            Some("paren") => match self.field(n, "inner") {
                Some(i) => self.expr(i),
                None => self.missing(n, "paren", "inner"),
            },
            Some("name") => {
                let at = self
                    .field(n, "name")
                    .filter(|_| !self.tree.is_token(n))
                    .unwrap_or(n);
                self.at(at);
                self.b.name(self.text(at).trim())
            }
            Some("number" | "float") => {
                self.at(n);
                match self.text(n).trim().parse::<f64>() {
                    Ok(v) => self.b.float(v),
                    Err(_) => self.invalid(n, "not a number"),
                }
            }
            Some("int") => {
                self.at(n);
                match self.text(n).trim().parse::<i64>() {
                    Ok(v) => self.b.int(v),
                    Err(_) => self.invalid(n, "not an integer that fits in 64 bits"),
                }
            }
            Some("string") => {
                self.at(n);
                match a0_syntax::unquote(self.text(n).trim()) {
                    Ok(s) => self.b.string(&s),
                    Err(message) => self.invalid(n, &message),
                }
            }
            Some("bool") => {
                self.at(n);
                self.b.bool_lit(self.text(n).trim() == "true")
            }
            Some(other) => {
                self.at(n);
                self.b.report_code(
                    code::SYNTAX,
                    &format!("the @hir form `{other}` is a statement, not a value"),
                );
                self.b.error_expr()
            }
            None => self.meaningless(n),
        }
    }

    fn invalid(&mut self, n: NodeId, message: &str) -> ExprId {
        self.at(n);
        self.b.report_code(code::INVALID_TOKEN, message);
        self.b.error_expr()
    }
}
