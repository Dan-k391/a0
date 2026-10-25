//! The compiler of {{Name}}: one walk over the syntax tree that the grammar ({{name}}.a0g) gives,
//! building A0's high-level IR by name. The HIR builder resolves names, infers and checks types and
//! reports errors in {{Name}}'s terms; A0 does the rest (interpreter, native code, collector,
//! language server).

use a0_diag::codes as code;
use a0_diag::{Diagnostics, SourceId};
use a0_hir::{BinOp, Builtin, ExprId, HirBuilder, HirModule, TypeId, UnOp, Warning};
use a0_syntax::{Grammar, NodeId, SyntaxTree};

/// The grammar.
pub const GRAMMAR: &str = include_str!("../{{name}}.a0g");

pub fn grammar() -> Grammar {
    match Grammar::from_str(GRAMMAR, SourceId(u32::MAX)) {
        Ok(g) => g,
        Err(d) => panic!("{{name}}.a0g is invalid:\n{}", d.summary()),
    }
}

/// Compiles a program to HIR; `source` is the id of `text` in the caller's source map, for spans.
pub fn compile(text: &str, source: SourceId) -> Result<HirModule, Diagnostics> {
    let (tree, diags) = grammar().parse(text, source);
    if diags.has_errors() {
        return Err(diags);
    }
    let mut b = HirBuilder::new("{{name}}");
    b.enable_warning(Warning::UnusedVariable);
    Compiler {
        tree: &tree,
        b: &mut b,
    }
    .program();
    let m = b.finish()?;
    let entry = a0_hir::check_entry(&m);
    if entry.has_errors() {
        Err(entry)
    } else {
        Ok(m)
    }
}

struct Compiler<'a> {
    tree: &'a SyntaxTree,
    b: &'a mut HirBuilder,
}

impl<'a> Compiler<'a> {
    fn field(&self, n: NodeId, name: &str) -> NodeId {
        self.tree.field_named(n, name).unwrap()
    }

    fn opt(&self, n: NodeId, name: &str) -> Option<NodeId> {
        self.tree.field_named(n, name)
    }

    fn text(&self, n: NodeId) -> &'a str {
        self.tree.text(n)
    }

    fn kind(&self, n: NodeId) -> &'a str {
        self.tree.kind_name(n)
    }

    /// Attributes what is built next (and its errors) to `n`.
    fn at(&mut self, n: NodeId) {
        self.b.set_span(Some(self.tree.span(n)));
    }

    /// Declares every function first, so that they can call each other in any order, then
    /// compiles their bodies.
    fn program(&mut self) {
        let mut funcs = Vec::new();
        for item in self.tree.nodes(self.tree.root()) {
            let name = self.field(item, "name");
            let mut names = Vec::new();
            let mut types = Vec::new();
            for p in self.tree.nodes_of_kind(item, "param") {
                names.push(self.text(self.field(p, "name")));
                types.push(self.ty(self.field(p, "type")));
            }
            let ret = match self.opt(item, "ret") {
                Some(r) => self.ty(r),
                None => TypeId::UNIT,
            };
            self.at(name);
            let f = self
                .b
                .declare_function(self.text(name), &names, &types, ret);
            let body = self.b.function_body(f);
            for (i, p) in self
                .tree
                .nodes_of_kind(item, "param")
                .into_iter()
                .enumerate()
            {
                let span = self.tree.span(self.field(p, "name"));
                self.b.set_param_span(body, i as u32, span);
            }
            funcs.push((f, item));
        }
        for (f, item) in funcs {
            let body = self.b.function_body(f);
            self.b.begin_body(body);
            let root = self.block(self.field(item, "body"));
            self.b.end_body();
            self.b.set_root(body, root);
        }
    }

    /// `int`, `float`, `bool` or `string`.
    fn ty(&mut self, node: NodeId) -> TypeId {
        self.at(node);
        self.b.named_type(self.text(self.field(node, "name")), &[])
    }

    fn block(&mut self, node: NodeId) -> ExprId {
        self.b.push_scope();
        let items: Vec<ExprId> = self
            .tree
            .nodes(node)
            .into_iter()
            .map(|s| self.stmt(s))
            .collect();
        self.b.pop_scope();
        self.at(node);
        self.b.block(&items, None)
    }

    fn stmt(&mut self, node: NodeId) -> ExprId {
        match self.kind(node) {
            "let_stmt" => {
                let value = self.expr(self.field(node, "value"));
                let name = self.field(node, "name");
                self.at(name);
                let ty = self.b.fresh_type();
                self.b.let_named(self.text(name), ty, value)
            }
            "if_stmt" => {
                let cond = self.expr(self.field(node, "cond"));
                let then = self.block(self.field(node, "then"));
                let els = self.opt(node, "else").map(|e| match self.kind(e) {
                    "if_stmt" => self.stmt(e),
                    _ => self.block(e),
                });
                self.at(node);
                self.b.if_(cond, then, els)
            }
            "while_stmt" => {
                let cond = self.expr(self.field(node, "cond"));
                let body = self.block(self.field(node, "body"));
                self.at(node);
                self.b.while_(cond, body)
            }
            "return_stmt" => {
                let value = self.opt(node, "value").map(|v| self.expr(v));
                self.at(node);
                self.b.return_(value)
            }
            "block" => self.block(node),
            _ => {
                let target = self.field(node, "expr");
                match self.opt(node, "value") {
                    Some(value) => self.assign(target, value),
                    None => self.expr(target),
                }
            }
        }
    }

    fn assign(&mut self, target: NodeId, value: NodeId) -> ExprId {
        let target = self.unwrap_primary(target);
        if self.kind(target) != "name_expr" {
            self.at(target);
            self.b
                .report_code(code::NOT_ASSIGNABLE, "only variables can be assigned");
            self.b.report_primary_label("not a variable");
            return self.b.error_expr();
        }
        let v = self.expr(value);
        self.at(target);
        self.b.assign_named(self.text(target), v)
    }

    fn unwrap_primary(&self, n: NodeId) -> NodeId {
        if self.kind(n) == "primary" {
            self.tree.child(n, 0)
        } else {
            n
        }
    }

    fn expr(&mut self, node: NodeId) -> ExprId {
        let node = self.unwrap_primary(node);
        let sub = |c: &mut Self, name: &str| c.expr(c.field(node, name));
        match self.kind(node) {
            "literal" => self.literal(node),
            "paren" => sub(self, "inner"),
            "binary" | "logical" => {
                let (l, r) = (sub(self, "left"), sub(self, "right"));
                self.at(node);
                match self.text(self.field(node, "op")) {
                    "&&" => self.b.and(l, r),
                    "||" => self.b.or(l, r),
                    op => self.b.binary(BinOp::from_symbol(op).unwrap(), l, r),
                }
            }
            "unary" => {
                let a = sub(self, "operand");
                self.at(node);
                match self.text(self.field(node, "op")) {
                    "-" => self.b.unary(UnOp::Neg, a),
                    _ => self.b.unary(UnOp::Not, a),
                }
            }
            "call" => self.call(node),
            _ => {
                self.at(node);
                self.b.name(self.text(node))
            }
        }
    }

    fn literal(&mut self, node: NodeId) -> ExprId {
        let tok = self.tree.child(node, 0);
        let text = self.text(tok);
        self.at(node);
        match self.kind(tok) {
            "INT" => match text.parse() {
                Ok(v) => self.b.int(v),
                Err(_) => {
                    self.b
                        .report_code(code::INVALID_TOKEN, "this integer does not fit in 64 bits");
                    self.b.error_expr()
                }
            },
            "FLOAT" => self.b.float(text.parse().unwrap()),
            "STRING" => match a0_syntax::unquote(text) {
                Ok(s) => self.b.string(&s),
                Err(message) => {
                    self.b.report_code(code::INVALID_TOKEN, &message);
                    self.b.error_expr()
                }
            },
            _ => self.b.bool_lit(text == "true"),
        }
    }

    /// `f(args)`: a function or a builtin (`str`, `len`, ...); `print(x)` prints `x` on a line.
    fn call(&mut self, node: NodeId) -> ExprId {
        let callee = self.unwrap_primary(self.field(node, "operand"));
        let args: Vec<ExprId> = match self.opt(node, "args") {
            Some(a) => self
                .tree
                .fields_named(a, "arg")
                .into_iter()
                .map(|a| self.expr(a))
                .collect(),
            None => Vec::new(),
        };
        self.at(node);
        if self.kind(callee) != "name_expr" {
            self.b
                .report_code(code::NOT_CALLABLE, "only functions can be called");
            return self.b.error_expr();
        }
        let name = self.text(callee);
        self.b.set_name_span(Some(self.tree.span(callee)));
        if name == "print" {
            let value = self.b.builtin(Builtin::Print, &args);
            let newline = self.b.string("\n");
            let newline = self.b.builtin(Builtin::Print, &[newline]);
            return self.b.block(&[value, newline], None);
        }
        self.b.call_named(name, &args)
    }
}
