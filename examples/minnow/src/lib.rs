//! Minnow: a small statically typed language built on A0.
//!
//! ```text
//! struct Point { x: float, y: float }
//! enum Shape { Circle(float), Rect(Point, Point) }
//!
//! fn largest<T>(xs: [T], size: fn(T) -> float) -> T? {
//!   let best: T? = null;
//!   for x in xs {
//!     if best == null || size(x) > size(best!) { best = x; }
//!   }
//!   return best;
//! }
//! ```
//!
//! Types are `int`, `float`, `bool`, `string`, arrays `[T]`, functions `fn(T, ...) -> R`, structs and
//! enums (both possibly generic) and nullable references `T?`. Closures capture variables by
//! reference. The compiler is one walk over the syntax tree: A0's HIR builder resolves names,
//! infers and checks types, monomorphizes generics and reports errors, so Minnow has no checker of
//! its own.

use a0_diag::{Diagnostics, SourceId};
use a0_hir::{BinOp, BodyId, Builtin, ExprId, HirBuilder, HirModule, Item, TypeId, TypeTag, UnOp};
use a0_syntax::{Grammar, NodeId, SyntaxTree};

/// The Minnow grammar.
pub const GRAMMAR: &str = include_str!("../minnow.a0g");

/// Loads the grammar. It is part of this crate, so failing to load it is a bug.
pub fn grammar() -> Grammar {
    match Grammar::from_str(GRAMMAR, SourceId(u32::MAX)) {
        Ok(g) => g,
        Err(d) => panic!("minnow.a0g is invalid:\n{}", d.summary()),
    }
}

/// Compiles a program to HIR. `source` is the id of `text` in the caller's source map.
pub fn compile(text: &str, source: SourceId) -> Result<HirModule, Diagnostics> {
    let (tree, diags) = grammar().parse(text, source);
    if diags.has_errors() {
        return Err(diags);
    }
    let mut c = Compiler {
        tree: &tree,
        b: HirBuilder::new("minnow"),
    };
    c.b.set_type_syntax(TypeTag::Array, "[{}]");
    c.program();
    let m = c.b.finish()?;
    let entry = a0_hir::check_entry(&m);
    if entry.has_errors() {
        Err(entry)
    } else {
        Ok(m)
    }
}

struct Compiler<'a> {
    tree: &'a SyntaxTree,
    b: HirBuilder,
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

    /// Attributes what is built next to `n`.
    fn at(&mut self, n: NodeId) {
        self.b.set_span(Some(self.tree.span(n)));
    }

    // ---------------------------------------------------------------- items

    fn program(&mut self) {
        let items = self.tree.nodes(self.tree.root());
        // Types first, so that declarations can use them in any order.
        for &item in &items {
            let name = self.text(self.field(item, "name"));
            let params = self.type_params(item);
            self.at(item);
            match self.kind(item) {
                "struct_decl" => drop(self.b.declare_generic_struct(name, &params)),
                "enum_decl" => drop(self.b.declare_generic_enum(name, &params)),
                _ => {}
            }
        }
        let mut funcs = Vec::new();
        for &item in &items {
            let name = self.text(self.field(item, "name"));
            match (self.kind(item), self.b.module().lookup_type(name)) {
                ("struct_decl", Some(Item::Struct(s))) => {
                    self.b.bind_type_params(Item::Struct(s));
                    for f in self.tree.nodes_of_kind(item, "field") {
                        let t = self.ty(self.field(f, "type"));
                        self.at(f);
                        self.b.add_field(s, self.text(self.field(f, "name")), t);
                    }
                }
                ("enum_decl", Some(Item::Enum(e))) => {
                    self.b.bind_type_params(Item::Enum(e));
                    for v in self.tree.nodes_of_kind(item, "variant") {
                        let types: Vec<TypeId> = self
                            .tree
                            .fields_named(v, "type")
                            .into_iter()
                            .map(|t| self.ty(t))
                            .collect();
                        self.at(v);
                        self.b
                            .add_variant(e, self.text(self.field(v, "name")), &types);
                    }
                }
                ("fn_decl", _) => {
                    self.at(item);
                    let f = self
                        .b
                        .declare_generic_function(name, &self.type_params(item));
                    self.b.bind_type_params(Item::Func(f));
                    let (names, types, ret) = self.signature(item);
                    self.at(item);
                    self.b.set_signature(f, &names, &types, ret);
                    funcs.push((f, item));
                }
                _ => {}
            }
        }
        for (f, item) in funcs {
            let body = self.b.function_body(f);
            let root = self.body(body, item);
            self.b.set_root(body, root);
        }
    }

    fn type_params(&self, item: NodeId) -> Vec<&'a str> {
        let tp = self.tree.nodes_of_kind(item, "type_params");
        let names = tp.first().map(|&t| self.tree.fields_named(t, "tparam"));
        names
            .unwrap_or_default()
            .into_iter()
            .map(|n| self.text(n))
            .collect()
    }

    /// The parameter names and types and the result type of a function or closure.
    fn signature(&mut self, node: NodeId) -> (Vec<&'a str>, Vec<TypeId>, TypeId) {
        let mut names = Vec::new();
        let mut types = Vec::new();
        for p in self.tree.nodes_of_kind(node, "param") {
            names.push(self.text(self.field(p, "name")));
            types.push(self.ty(self.field(p, "type")));
        }
        let ret = match self.opt(node, "ret") {
            Some(r) => self.ty(r),
            None => TypeId::UNIT,
        };
        (names, types, ret)
    }

    /// The statements of a function or closure, as the root of `body`.
    fn body(&mut self, body: BodyId, decl: NodeId) -> ExprId {
        self.b.begin_body(body);
        let root = self.block(self.field(decl, "body"));
        self.b.end_body();
        root
    }

    fn ty(&mut self, node: NodeId) -> TypeId {
        let base = self.field(node, "base");
        let t = match self.kind(base) {
            "array_type" => {
                let e = self.ty(self.field(base, "elem"));
                self.b.array_type(e)
            }
            "fn_type" => {
                let params: Vec<TypeId> = self
                    .tree
                    .fields_named(base, "param_type")
                    .into_iter()
                    .map(|p| self.ty(p))
                    .collect();
                let ret = match self.opt(base, "ret") {
                    Some(r) => self.ty(r),
                    None => TypeId::UNIT,
                };
                self.b.fn_type(&params, ret)
            }
            _ => {
                let args: Vec<TypeId> = self
                    .tree
                    .fields_named(base, "targ")
                    .into_iter()
                    .map(|a| self.ty(a))
                    .collect();
                self.at(base);
                self.b
                    .named_type(self.text(self.field(base, "name")), &args)
            }
        };
        if self.opt(node, "nullable").is_none() {
            return t;
        }
        self.at(node);
        self.b.nullable_type(t)
    }

    // ---------------------------------------------------------------- statements

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
        let opt_expr = |c: &mut Self, name: &str| c.opt(node, name).map(|e| c.expr(e));
        match self.kind(node) {
            "let_stmt" => {
                let value = self.expr(self.field(node, "value"));
                let ty = match self.opt(node, "type") {
                    Some(t) => self.ty(t),
                    None => self.b.fresh_type(),
                };
                let name = self.field(node, "name");
                self.at(name);
                if self.b.is_declared_in_scope(self.text(name)) {
                    let msg = format!("`{}` is already defined in this scope", self.text(name));
                    self.b.report(&msg);
                }
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
            "for_stmt" => self.for_stmt(node),
            "return_stmt" => {
                let value = opt_expr(self, "value");
                self.at(node);
                self.b.return_(value)
            }
            "break_stmt" => {
                self.at(node);
                self.b.break_()
            }
            "continue_stmt" => {
                self.at(node);
                self.b.continue_()
            }
            "block" => self.block(node),
            "match_stmt" => self.expr(self.field(node, "expr")),
            _ => {
                let target = self.field(node, "expr");
                match self.opt(node, "value") {
                    Some(value) => self.assign(target, value),
                    None => self.expr(target),
                }
            }
        }
    }

    /// `for x in items { body }` is
    /// `{ let a = items; let i = 0; while i < len(a) { let x = a[i]; i = i + 1; body } }`, where `a`
    /// and `i` have no names in scope.
    fn for_stmt(&mut self, node: NodeId) -> ExprId {
        let items = self.expr(self.field(node, "iterable"));
        self.at(node);
        let body = self.b.current_body();
        let elem = self.b.fresh_type();
        let array = self.b.array_type(elem);
        let a = self.b.new_local(body, "items", array);
        let i = self.b.new_local(body, "i", TypeId::INT);
        let bind_a = self.b.let_(a, items);
        let zero = self.b.int(0);
        let bind_i = self.b.let_(i, zero);
        let (a_ref, i_ref) = (self.b.local(a), self.b.local(i));
        let len = self.b.builtin(Builtin::Len, &[a_ref]);
        let cond = self.b.binary(BinOp::Lt, i_ref, len);
        self.b.push_scope();
        let (a_ref, i_ref) = (self.b.local(a), self.b.local(i));
        let x = self.b.index(a_ref, i_ref);
        let bind_x = self
            .b
            .let_named(self.text(self.field(node, "name")), elem, x);
        let (i_ref, one) = (self.b.local(i), self.b.int(1));
        let next = self.b.binary(BinOp::Add, i_ref, one);
        let step = self.b.set_local(i, next);
        let body = self.block(self.field(node, "body"));
        self.b.pop_scope();
        self.at(node);
        let body = self.b.block(&[bind_x, step, body], None);
        let lp = self.b.while_(cond, body);
        self.b.block(&[bind_a, bind_i, lp], None)
    }

    fn assign(&mut self, target: NodeId, value: NodeId) -> ExprId {
        let target = self.unwrap_primary(target);
        let operand = |c: &mut Self| c.expr(c.field(target, "operand"));
        match self.kind(target) {
            "name_expr" => {
                let v = self.expr(value);
                self.at(target);
                self.b.assign_named(self.text(target), v)
            }
            "get" => {
                let o = operand(self);
                let v = self.expr(value);
                let name = self.field(target, "name");
                self.at(name);
                self.b.set_field_named(o, self.text(name), v)
            }
            "index" => {
                let o = operand(self);
                let i = self.expr(self.field(target, "index"));
                let v = self.expr(value);
                self.at(target);
                self.b.set_index(o, i, v)
            }
            _ => {
                self.at(target);
                self.b.report("cannot assign to this expression");
                self.b.error_expr()
            }
        }
    }

    // ---------------------------------------------------------------- expressions

    fn unwrap_primary(&self, n: NodeId) -> NodeId {
        if self.kind(n) == "primary" {
            self.tree.child(n, 0)
        } else {
            n
        }
    }

    fn args(&mut self, node: NodeId) -> Vec<ExprId> {
        match self.opt(node, "args") {
            Some(a) => self
                .tree
                .fields_named(a, "arg")
                .into_iter()
                .map(|a| self.expr(a))
                .collect(),
            None => Vec::new(),
        }
    }

    fn expr(&mut self, node: NodeId) -> ExprId {
        let node = self.unwrap_primary(node);
        let sub = |c: &mut Self, name: &str| c.expr(c.field(node, name));
        match self.kind(node) {
            "literal" => self.literal(node),
            "paren" => sub(self, "inner"),
            "match_expr" => self.match_expr(node),
            "array_lit" => {
                let elems = self.args(node);
                self.at(node);
                self.b.array(&elems)
            }
            "new_expr" => self.new_expr(node),
            "closure" => {
                let (names, types, ret) = self.signature(node);
                self.at(node);
                let body = self
                    .b
                    .new_closure_body(self.b.current_body(), &names, &types, ret);
                let root = self.body(body, node);
                self.b.set_root(body, root);
                self.at(node);
                self.b.lambda(body)
            }
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
            "cast" => {
                let a = sub(self, "operand");
                let t = self.ty(self.field(node, "type"));
                self.at(node);
                self.b.cast(a, t)
            }
            "unwrap" => {
                let a = sub(self, "operand");
                self.at(node);
                self.b.unwrap(a)
            }
            "call" => self.call(node),
            "get" => {
                let o = sub(self, "operand");
                let name = self.field(node, "name");
                self.at(name);
                self.b.field_named(o, self.text(name))
            }
            "index" => {
                let (a, i) = (sub(self, "operand"), sub(self, "index"));
                self.at(node);
                self.b.index(a, i)
            }
            "variant_expr" => {
                let args = self.args(node);
                self.at(node);
                let ename = self.text(self.field(node, "enum"));
                match self.b.module().lookup_type(ename) {
                    Some(Item::Enum(e)) => {
                        self.b
                            .variant_named(e, self.text(self.field(node, "name")), &args)
                    }
                    _ => {
                        self.b.report(&format!("unknown enum `{ename}`"));
                        self.b.error_expr()
                    }
                }
            }
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
                    self.b.report("this integer does not fit in 64 bits");
                    self.b.error_expr()
                }
            },
            "FLOAT" => self.b.float(text.parse().unwrap()),
            "STRING" => match a0_syntax::unquote(text) {
                Ok(s) => self.b.string(&s),
                Err(msg) => {
                    self.b.report(&msg);
                    self.b.error_expr()
                }
            },
            _ => match text {
                "null" => {
                    let t = self.b.fresh_type();
                    self.b.null(t)
                }
                _ => self.b.bool_lit(text == "true"),
            },
        }
    }

    /// `new S { f: v, ... }`, with values evaluated in the order written.
    fn new_expr(&mut self, node: NodeId) -> ExprId {
        let inits = self.tree.nodes_of_kind(node, "field_init");
        let names: Vec<&str> = inits
            .iter()
            .map(|&f| self.text(self.field(f, "name")))
            .collect();
        let values: Vec<ExprId> = inits
            .iter()
            .map(|&f| self.expr(self.field(f, "value")))
            .collect();
        self.at(node);
        let sname = self.text(self.field(node, "name"));
        match self.b.module().lookup_type(sname) {
            Some(Item::Struct(s)) => self.b.new_struct_named(s, &names, &values),
            _ => {
                self.b.report(&format!("unknown struct `{sname}`"));
                self.b.error_expr()
            }
        }
    }

    fn call(&mut self, node: NodeId) -> ExprId {
        let callee = self.unwrap_primary(self.field(node, "operand"));
        let args = self.args(node);
        if self.kind(callee) != "name_expr" {
            let c = self.expr(callee);
            self.at(node);
            return self.b.call_value(c, &args);
        }
        let name = self.text(callee);
        self.at(node);
        // `print(x)` prints any printable value on a line of its own.
        if name == "print" && self.b.lookup_local(name).is_none() {
            let value = self.b.builtin(Builtin::Print, &args);
            let newline = self.b.string("\n");
            let newline = self.b.builtin(Builtin::Print, &[newline]);
            return self.b.block(&[value, newline], None);
        }
        self.b.call_named(name, &args)
    }

    fn match_expr(&mut self, node: NodeId) -> ExprId {
        let scrutinee = self.expr(self.field(node, "scrutinee"));
        let t = self.b.type_of(scrutinee);
        let t = self.b.resolved_type(t);
        let Some(e) = self.b.module().enum_of(t) else {
            self.at(node);
            let msg = format!("cannot match on `{}`", self.b.module().display_type(t));
            self.b.report(&msg);
            return self.b.error_expr();
        };
        let mut arms = Vec::new();
        for arm in self.tree.nodes_of_kind(node, "arm") {
            let pattern = self.field(arm, "pattern");
            self.b.push_scope();
            let variant = self.opt(pattern, "variant").and_then(|v| {
                let index = self.b.module().variant_index(e, self.text(v));
                if index.is_none() {
                    self.at(v);
                    let enum_name = self.b.module().enum_name(e).to_string();
                    self.b
                        .report(&format!("`{enum_name}` has no variant `{}`", self.text(v)));
                }
                index
            });
            let binds: Vec<_> = self
                .tree
                .fields_named(pattern, "bind")
                .into_iter()
                .map(|n| {
                    self.at(n);
                    let t = self.b.fresh_type();
                    self.b.declare_local(self.text(n), t)
                })
                .collect();
            let body = self.field(arm, "body");
            let body = match self.kind(body) {
                "block" => self.block(body),
                _ => self.expr(body),
            };
            self.b.pop_scope();
            arms.push(self.b.arm(variant, &binds, body));
        }
        self.at(node);
        self.b.match_(scrutinee, &arms)
    }
}
