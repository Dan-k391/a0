//! B0, A0's flagship language (`README.md`).
//!
//! ```text
//! inline struct Vec2 { x: f32, y: f32 }
//! enum Shape { Circle(float), Rect(float, float) }
//!
//! fn total<T: Area>(xs: [T]) -> float {
//!   return fold(xs, 0.0, fn(sum: float, x: T) -> float { return sum + x.area(); });
//! }
//! ```
//!
//! The compiler is one walk over the syntax tree that builds HIR by name. A0's HIR builder resolves
//! names, infers and checks types, monomorphizes generics and reports errors, so B0 has no checker
//! of its own; what B0 adds itself is syntax (maps, ranges, compound assignment), the prelude
//! (`src/prelude.b0`, functions written in B0 that every program sees) and one rule A0 leaves to
//! languages: raw pointers only inside `unsafe`. It uses only A0's public API
//! (`docs/compatibility.md`): `a0-diag`, `a0-syntax`, `a0-hir` and `a0-driver`.

use a0_diag::codes as code;
use a0_diag::{Diagnostics, SourceId};
use std::sync::OnceLock;

use a0_hir::{
    BinOp, BodyId, Builtin, ExprId, FuncId, HirBuilder, HirModule, InterfaceId, Item, LocalId,
    Memory, TypeId, TypeTag, UnOp, Warning,
};
use a0_syntax::{Grammar, NodeId, SyntaxTree};

/// The B0 grammar.
pub const GRAMMAR: &str = include_str!("../b0.a0g");

/// The prelude: functions written in B0 that every program can call.
pub const PRELUDE: &str = include_str!("prelude.b0");

/// The prelude's built-in source number (`a0_diag::register_builtin_source`); the standard
/// library's are below it.
const PRELUDE_SOURCE: u32 = 100;

/// The grammar, loaded once. It is part of this crate, so failing to load it is a bug.
pub fn grammar() -> &'static Grammar {
    static GRAMMAR_ONCE: OnceLock<Grammar> = OnceLock::new();
    GRAMMAR_ONCE.get_or_init(|| match Grammar::from_str(GRAMMAR, SourceId(u32::MAX)) {
        Ok(g) => g,
        Err(d) => panic!("b0.a0g is invalid:\n{}", d.summary()),
    })
}

/// Compiles a one-file program to HIR. `source` is the id of `text` in the caller's source map.
/// Programs of several files go through the driver, which calls [`start`] and [`compile_file`].
pub fn compile(text: &str, source: SourceId) -> Result<HirModule, Diagnostics> {
    let (tree, diags) = grammar().parse(text, source);
    if diags.has_errors() {
        return Err(diags);
    }
    let mut b = HirBuilder::new("b0");
    start(&mut b);
    // The program is a module of its own, as the driver makes it, so it may reuse the prelude's names.
    b.begin_module("main", true);
    compile_file(&mut b, &tree);
    let m = b.finish()?;
    let entry = a0_hir::check_entry(&m);
    if entry.has_errors() {
        Err(entry)
    } else {
        Ok(m)
    }
}

/// The module the prelude is compiled into. Every file imports its functions by name, unless it
/// declares one of the same name itself, which then shadows it.
const PRELUDE_MODULE: &str = "b0.prelude";

/// The prelude's syntax tree, parsed once.
fn prelude() -> &'static SyntaxTree {
    static PRELUDE_TREE: OnceLock<SyntaxTree> = OnceLock::new();
    PRELUDE_TREE.get_or_init(|| {
        let source = a0_diag::register_builtin_source(PRELUDE_SOURCE, "<b0>/prelude.b0", PRELUDE);
        let (tree, diags) = grammar().parse(PRELUDE, source);
        assert!(
            !diags.has_errors(),
            "the prelude is invalid:\n{}",
            diags.summary()
        );
        tree
    })
}

/// The names of the functions a file declares.
fn declared_functions(tree: &SyntaxTree) -> Vec<&str> {
    tree.nodes(tree.root())
        .into_iter()
        .filter(|&i| matches!(tree.kind_name(i), "fn_decl" | "extern_decl"))
        .filter_map(|i| tree.field_named(i, "name").map(|n| tree.text(n)))
        .collect()
}

/// Sets up a builder for a B0 program: how types are written, the warnings, and the prelude.
pub fn start(b: &mut HirBuilder) {
    b.set_type_syntax(TypeTag::Array, "[{}]");
    b.set_type_syntax(TypeTag::Map, "{{}: {}}");
    b.begin_module(PRELUDE_MODULE, false);
    Compiler {
        tree: prelude(),
        b,
        unsafe_depth: 0,
    }
    .program();
    b.enable_warning(Warning::UnusedVariable);
    b.enable_warning(Warning::UnreachableCode);
}

/// Compiles one file of a program into the builder's current module.
pub fn compile_file(b: &mut HirBuilder, tree: &SyntaxTree) {
    let own = declared_functions(tree);
    for name in declared_functions(prelude()) {
        if !own.contains(&name) {
            b.set_span(None);
            b.import_item(PRELUDE_MODULE, name);
        }
    }
    Compiler {
        tree,
        b,
        unsafe_depth: 0,
    }
    .program();
}

struct Compiler<'a> {
    tree: &'a SyntaxTree,
    b: &'a mut HirBuilder,
    /// How many `unsafe` blocks enclose the code being compiled.
    unsafe_depth: u32,
}

impl<'a> Compiler<'a> {
    fn field(&self, n: NodeId, name: &str) -> NodeId {
        self.tree.field_named(n, name).unwrap()
    }

    fn opt(&self, n: NodeId, name: &str) -> Option<NodeId> {
        self.tree.field_named(n, name)
    }

    /// For `if let x = e` and `while let x = e`: the binding (in scope only in the block in field
    /// `block`), the value and that block.
    fn narrow(&mut self, node: NodeId, block: &str) -> Option<(LocalId, ExprId, ExprId)> {
        let name = self.opt(node, "name")?;
        let value = self.expr(self.field(node, "value"));
        self.b.push_scope();
        self.at(name);
        let l = self.b.declare_if_let(self.text(name), value);
        let body = self.block(self.field(node, block));
        self.b.pop_scope();
        Some((l, value, body))
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

    /// Attributes a declaration to its name, which is what messages point at.
    fn at_name(&mut self, n: NodeId) {
        let name = self.opt(n, "name").unwrap_or(n);
        self.at(name);
    }

    /// Where the parameters of `body`, declared by `decl`, are, for messages about arguments.
    fn param_spans(&mut self, body: BodyId, decl: NodeId) {
        for (i, p) in self
            .tree
            .nodes_of_kind(decl, "param")
            .into_iter()
            .enumerate()
        {
            let span = self.tree.span(self.field(p, "name"));
            self.b.set_param_span(body, i as u32, span);
        }
    }

    // ---------------------------------------------------------------- items

    fn program(&mut self) {
        if let Some(decl) = self.opt(self.tree.root(), "memory") {
            let mode = self.field(decl, "mode");
            self.at(mode);
            match self.text(mode) {
                "gc" => self.b.set_memory(Memory::Gc),
                "manual" => self.b.set_memory(Memory::Manual),
                other => {
                    let msg = format!("unknown memory mode `{other}`");
                    self.b.report_code(code::SYNTAX, &msg);
                    self.b.report_primary_label("expected `gc` or `manual`");
                }
            }
        }
        // Imports are the driver's business: it puts the modules in scope.
        let items: Vec<NodeId> = self
            .tree
            .nodes(self.tree.root())
            .into_iter()
            .filter(|&i| !matches!(self.kind(i), "import_decl" | "memory_decl"))
            .collect();
        // Types first, so that declarations can use them in any order.
        for &item in &items {
            let Some(name) = self.opt(item, "name").map(|n| self.text(n)) else {
                continue;
            };
            let params = self.type_params(item);
            self.at_name(item);
            self.b.set_public(self.opt(item, "pub").is_some());
            match self.kind(item) {
                "struct_decl" => {
                    let s = self.b.declare_generic_struct(name, &params);
                    if self.opt(item, "inline").is_some() {
                        self.b.set_struct_inline(s);
                    }
                }
                "enum_decl" => drop(self.b.declare_generic_enum(name, &params)),
                "interface_decl" => drop(self.b.declare_interface(name)),
                _ => {}
            }
        }
        let mut funcs = Vec::new();
        for &item in &items {
            if self.kind(item) == "impl_decl" {
                self.impl_decl(item, &mut funcs);
                continue;
            }
            let name = self.text(self.field(item, "name"));
            self.b.set_public(self.opt(item, "pub").is_some());
            match (self.kind(item), self.b.lookup_type_name(name)) {
                ("struct_decl", Some(Item::Struct(s))) => {
                    self.b.bind_type_params(Item::Struct(s));
                    for f in self.tree.nodes_of_kind(item, "field") {
                        let t = self.ty(self.field(f, "type"));
                        self.at_name(f);
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
                        self.at_name(v);
                        self.b
                            .add_variant(e, self.text(self.field(v, "name")), &types);
                    }
                }
                ("interface_decl", Some(Item::Interface(i))) => {
                    self.b.bind_type_params(Item::Interface(i));
                    for sig in self.tree.fields_named(item, "method") {
                        let (_, types, ret) = self.signature(sig);
                        self.at_name(sig);
                        self.b.add_interface_method(
                            i,
                            self.text(self.field(sig, "name")),
                            &types,
                            ret,
                        );
                    }
                }
                ("extern_decl", _) => {
                    let (_, types, ret) = self.signature(item);
                    self.at_name(item);
                    self.b.declare_extern(name, &types, ret);
                }
                ("fn_decl", _) => {
                    self.at_name(item);
                    let f = self
                        .b
                        .declare_generic_function(name, &self.type_params(item));
                    self.bounds(item, Item::Func(f), 0);
                    self.b.bind_type_params(Item::Func(f));
                    let (names, types, ret) = self.signature(item);
                    self.at_name(item);
                    self.b.set_signature(f, &names, &types, ret);
                    let body = self.b.function_body(f);
                    self.param_spans(body, item);
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
        self.type_param_nodes(item)
            .into_iter()
            .map(|p| self.text(self.field(p, "name")))
            .collect()
    }

    fn type_param_nodes(&self, item: NodeId) -> Vec<NodeId> {
        let tp = self.tree.nodes_of_kind(item, "type_params");
        tp.first()
            .map(|&t| self.tree.nodes_of_kind(t, "type_param"))
            .unwrap_or_default()
    }

    /// The interfaces written on `item`'s type parameters (`T: Shape + Named`), which are the
    /// parameters of `owner` from `skip` on.
    fn bounds(&mut self, item: NodeId, owner: Item, skip: u32) {
        for (k, p) in self.type_param_nodes(item).into_iter().enumerate() {
            for bound in self.tree.fields_named(p, "bound") {
                if let Some(i) = self.interface(bound) {
                    self.b.add_bound(owner, skip + k as u32, i);
                }
            }
        }
    }

    /// The interface a `named_type` names, or an error.
    fn interface(&mut self, node: NodeId) -> Option<InterfaceId> {
        let name = self.text(self.field(node, "name"));
        self.at(node);
        let item = match self.opt(node, "module") {
            Some(m) => self.b.member_item(self.text(m), name, true),
            None => self.b.lookup_type_name(name),
        };
        match item {
            Some(Item::Interface(i)) => Some(i),
            _ => {
                let msg = format!("`{name}` is not an interface");
                self.b.report_code(code::UNKNOWN_TYPE, &msg);
                self.b.report_primary_label("expected an interface");
                None
            }
        }
    }

    /// `impl Type { methods }` and `impl Interface for Type { methods }`: declares the methods
    /// (their bodies are compiled with the functions').
    fn impl_decl(&mut self, item: NodeId, funcs: &mut Vec<(FuncId, NodeId)>) {
        let owner_node = self.field(item, "owner");
        let name = self.text(self.field(owner_node, "name"));
        self.at(owner_node);
        let owner = match self.opt(owner_node, "module") {
            Some(m) => self.b.member_type(self.text(m), name, &[]),
            None => self.b.named_type(name, &[]),
        };
        self.b.set_public(true);
        if let Some(iface) = self.opt(item, "iface")
            && let Some(i) = self.interface(iface)
        {
            self.at(owner_node);
            self.b.implement(owner, i);
        }
        // A method's own type parameters come after its owner's.
        let skip = self.b.module().type_args(owner).len() as u32;
        for method in self.tree.fields_named(item, "method") {
            self.at_name(method);
            let f = self.b.declare_method(
                owner,
                self.text(self.field(method, "name")),
                &self.type_params(method),
            );
            self.bounds(method, Item::Func(f), skip);
            self.b.bind_type_params(Item::Func(f));
            let receiver = self.field(method, "receiver");
            let (mut names, mut types, ret) = self.signature(method);
            names.insert(0, self.text(receiver));
            types.insert(0, self.b.method_self_type(f));
            self.at_name(method);
            self.b.set_signature(f, &names, &types, ret);
            let body = self.b.function_body(f);
            self.b.set_param_span(body, 0, self.tree.span(receiver));
            for (i, p) in self
                .tree
                .nodes_of_kind(method, "param")
                .into_iter()
                .enumerate()
            {
                let span = self.tree.span(self.field(p, "name"));
                self.b.set_param_span(body, i as u32 + 1, span);
            }
            funcs.push((f, method));
        }
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
            "map_type" => {
                let k = self.ty(self.field(base, "key"));
                let v = self.ty(self.field(base, "value"));
                self.at(base);
                self.b.map_type(k, v)
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
                let name = self.text(self.field(base, "name"));
                match self.opt(base, "module") {
                    Some(m) => self.b.member_type(self.text(m), name, &args),
                    None => self.b.named_type(name, &args),
                }
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
                    let msg = format!("`{}` is already declared in this scope", self.text(name));
                    self.b.report_code(code::DUPLICATE, &msg);
                    self.b.report_primary_label("declared again here");
                    let first = self.b.lookup_local(self.text(name));
                    if let Some(span) = first.and_then(|l| self.b.module().local_span(l)) {
                        self.b.report_label(span, "first declared here");
                    }
                }
                self.b.let_named(self.text(name), ty, value)
            }
            "if_stmt" => {
                let narrow = self.narrow(node, "then");
                let cond = narrow
                    .is_none()
                    .then(|| self.expr(self.field(node, "cond")));
                let then = match narrow {
                    Some((_, _, then)) => then,
                    None => self.block(self.field(node, "then")),
                };
                let els = self.opt(node, "else").map(|e| match self.kind(e) {
                    "if_stmt" => self.stmt(e),
                    _ => self.block(e),
                });
                self.at(node);
                match (narrow, cond) {
                    (Some((l, value, _)), _) => self.b.if_let(l, value, then, els),
                    (None, Some(cond)) => self.b.if_(cond, then, els),
                    (None, None) => unreachable!(),
                }
            }
            "while_stmt" => {
                if let Some((l, value, body)) = self.narrow(node, "body") {
                    // `while let x = e { body }` is `loop { if let x = e { body } else { break } }`.
                    let stop = self.b.break_();
                    self.at(node);
                    let step = self.b.if_let(l, value, body, Some(stop));
                    return self.b.loop_(step);
                }
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
            "throw_stmt" => {
                let value = self.expr(self.field(node, "value"));
                self.at(node);
                let value = self.b.cast(value, TypeId::ANY);
                self.b.throw(value)
            }
            "try_stmt" => {
                let body = self.block(self.field(node, "body"));
                let clause = self.field(node, "handler");
                self.b.push_scope();
                let name = self.field(clause, "name");
                self.at(name);
                let local = self.b.declare_catch(self.text(name));
                let handler = self.block(self.field(clause, "body"));
                self.b.pop_scope();
                self.at(node);
                self.b.try_catch(body, local, handler)
            }
            "unsafe_stmt" => {
                self.unsafe_depth += 1;
                let body = self.block(self.field(node, "body"));
                self.unsafe_depth -= 1;
                body
            }
            _ => {
                let target = self.field(node, "expr");
                match self.opt(node, "value") {
                    Some(value) => {
                        let op = self.text(self.field(node, "op"));
                        match op.strip_suffix('=').filter(|o| !o.is_empty()) {
                            Some(op) => self.compound_assign(target, op, value),
                            None => self.assign(target, value),
                        }
                    }
                    None => self.expr(target),
                }
            }
        }
    }

    /// `x op= v` is `x = x op v`. The target is evaluated twice (`a[f()] += 1` calls `f` twice),
    /// as its parts are names, fields and indexes.
    fn compound_assign(&mut self, target: NodeId, op: &str, value: NodeId) -> ExprId {
        let current = self.expr(target);
        let v = self.expr(value);
        self.at(target);
        let op = BinOp::from_symbol(op).expect("the grammar's operators are A0's");
        let combined = self.b.binary(op, current, v);
        self.assign_value(target, combined)
    }

    /// `for x in items { body }` is
    /// `{ let a = items; let i = 0; while i < len(a) { let x = a[i]; i = i + 1; body } }`, where `a`
    /// and `i` have no names in scope.
    fn for_stmt(&mut self, node: NodeId) -> ExprId {
        if self.opt(node, "end").is_some() {
            return self.for_range(node);
        }
        let mut items = self.expr(self.field(node, "iterable"));
        self.at(node);
        // `for k in m` over a map goes through its keys.
        let t = self.b.type_of(items);
        let t = self.b.resolved_type(t);
        if self.b.module().type_tag(t) == TypeTag::Map {
            items = self.b.builtin(Builtin::Keys, &[items]);
        }
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
        let name = self.field(node, "name");
        self.at(name);
        let bind_x = self.b.let_named(self.text(name), elem, x);
        self.at(node);
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

    /// `for i in a..b { body }` is
    /// `{ let next = a; let end = b; while next < end { let i = next; next = next + 1; body } }`,
    /// where `next` and `end` have no names in scope.
    fn for_range(&mut self, node: NodeId) -> ExprId {
        let start = self.expr(self.field(node, "iterable"));
        let end = self.expr(self.field(node, "end"));
        self.at(node);
        let body = self.b.current_body();
        let ty = self.b.fresh_type();
        let next = self.b.new_local(body, "next", ty);
        let last = self.b.new_local(body, "end", ty);
        let bind_next = self.b.let_(next, start);
        let bind_end = self.b.let_(last, end);
        let (n, e) = (self.b.local(next), self.b.local(last));
        let cond = self.b.binary(BinOp::Lt, n, e);
        self.b.push_scope();
        let n = self.b.local(next);
        let name = self.field(node, "name");
        self.at(name);
        let bind_i = self.b.let_named(self.text(name), ty, n);
        self.at(node);
        let (n, one) = (self.b.local(next), self.b.int(1));
        let step = self.b.binary(BinOp::Add, n, one);
        let step = self.b.set_local(next, step);
        let body = self.block(self.field(node, "body"));
        self.b.pop_scope();
        self.at(node);
        let body = self.b.block(&[bind_i, step, body], None);
        let lp = self.b.while_(cond, body);
        self.b.block(&[bind_next, bind_end, lp], None)
    }

    /// Reports `p[i]` on a raw pointer outside `unsafe`. The pointer's type is known here when it
    /// was declared or inferred before; otherwise A0 types it later, unchecked by this rule.
    fn check_pointer_index(&mut self, operand: ExprId, node: NodeId) {
        let t = self.b.type_of(operand);
        let t = self.b.resolved_type(t);
        if self.unsafe_depth == 0 && self.b.module().type_tag(t) == TypeTag::Ptr {
            self.needs_unsafe(node, "indexing a raw pointer");
        }
    }

    /// Whether `e` is a dynamic object, as far as its type is known yet.
    fn is_object(&mut self, e: ExprId) -> bool {
        let t = self.b.type_of(e);
        let t = self.b.resolved_type(t);
        self.b.module().type_tag(t) == TypeTag::Object
    }

    fn needs_unsafe(&mut self, node: NodeId, what: &str) {
        self.at(node);
        self.b
            .report_code(code::NEEDS_UNSAFE, &format!("{what} needs `unsafe`"));
        self.b.report_primary_label("unchecked");
        self.b
            .report_help("raw pointers are unchecked: put this in an `unsafe { ... }` block");
    }

    fn assign(&mut self, target: NodeId, value: NodeId) -> ExprId {
        self.assign_with(target, &mut |c| c.expr(value))
    }

    fn assign_value(&mut self, target: NodeId, value: ExprId) -> ExprId {
        self.assign_with(target, &mut |_| value)
    }

    /// Assigns to `target` the value `value` builds, which comes after the target's parts, so that
    /// they are evaluated in the order written.
    fn assign_with(
        &mut self,
        target: NodeId,
        value: &mut dyn FnMut(&mut Self) -> ExprId,
    ) -> ExprId {
        let target = self.unwrap_primary(target);
        let operand = |c: &mut Self| c.expr(c.field(target, "operand"));
        match self.kind(target) {
            "name_expr" => {
                let v = value(self);
                self.at(target);
                self.b.assign_named(self.text(target), v)
            }
            "get" => {
                let o = operand(self);
                let v = value(self);
                let name = self.field(target, "name");
                self.at(name);
                if self.is_object(o) {
                    let key = self.b.string(self.text(name));
                    let v = self.b.cast(v, TypeId::ANY);
                    return self.b.builtin(Builtin::SetField, &[o, key, v]);
                }
                self.b.set_field_named(o, self.text(name), v)
            }
            "index" => {
                let o = operand(self);
                self.check_pointer_index(o, target);
                let i = self.expr(self.field(target, "index"));
                let v = value(self);
                self.at(target);
                self.b.set_index(o, i, v)
            }
            _ => {
                self.at(target);
                self.b
                    .report_code(code::NOT_ASSIGNABLE, "cannot assign to this expression");
                self.b.report_primary_label("cannot be assigned to");
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
            "map_lit" => self.map_lit(node),
            "sizeof" => {
                let t = self.ty(self.field(node, "type"));
                self.at(node);
                self.b.size_of(t)
            }
            "is" => {
                let a = sub(self, "operand");
                let t = self.ty(self.field(node, "type"));
                self.at(node);
                self.b.is(a, t)
            }
            "new_expr" => self.new_expr(node),
            "closure" => {
                let (names, types, ret) = self.signature(node);
                self.at(node);
                let body = self
                    .b
                    .new_closure_body(self.b.current_body(), &names, &types, ret);
                self.param_spans(body, node);
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
                    "~" => self.b.unary(UnOp::BitNot, a),
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
            "propagate" => {
                let a = sub(self, "operand");
                self.at(node);
                self.b.propagate(a)
            }
            "call" => self.call(node),
            "get" => {
                let name = self.field(node, "name");
                if let Some(alias) = self.module_alias(self.field(node, "operand")) {
                    self.at(name);
                    return self.b.member(alias, self.text(name));
                }
                let o = sub(self, "operand");
                self.at(name);
                if self.is_object(o) {
                    // A dynamic object's field, found by name at run time: an `any`.
                    let key = self.b.string(self.text(name));
                    return self.b.builtin(Builtin::Field, &[o, key]);
                }
                self.b.field_named(o, self.text(name))
            }
            "index" => {
                let a = sub(self, "operand");
                self.check_pointer_index(a, node);
                let i = sub(self, "index");
                self.at(node);
                self.b.index(a, i)
            }
            "variant_expr" => {
                let args = self.args(node);
                self.at(node);
                let enum_node = self.field(node, "enum");
                let ename = self.text(enum_node);
                let found = match self.opt(node, "module") {
                    Some(m) => {
                        self.at(enum_node);
                        let item = self.b.member_item(self.text(m), ename, true);
                        if item.is_none() {
                            return self.b.error_expr();
                        }
                        item
                    }
                    None => self.b.lookup_type_name(ename),
                };
                match found {
                    Some(Item::Enum(e)) => {
                        let name = self.field(node, "name");
                        self.b.set_name_span(Some(self.tree.span(name)));
                        self.b.variant_named(e, self.text(name), &args)
                    }
                    Some(_) => {
                        self.at(enum_node);
                        self.b
                            .report_code(code::UNKNOWN_TYPE, &format!("`{ename}` is not an enum"));
                        self.b.error_expr()
                    }
                    None => {
                        // Reports the unknown name, with a close one if there is.
                        self.at(enum_node);
                        self.b.named_type(ename, &[]);
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
        let digits = text.replace('_', "");
        match self.kind(tok) {
            "HEX" => match u64::from_str_radix(&digits[2..], 16) {
                // As C does, a hexadecimal literal is its bits: 0xFFFF_FFFF_FFFF_FFFF is -1.
                Ok(v) => self.b.int(v as i64),
                Err(_) => {
                    self.b
                        .report_code(code::INVALID_TOKEN, "this integer does not fit in 64 bits");
                    self.b.error_expr()
                }
            },
            "INT" => match digits.parse() {
                Ok(v) => self.b.int(v),
                Err(_) => {
                    self.b
                        .report_code(code::INVALID_TOKEN, "this integer does not fit in 64 bits");
                    self.b.error_expr()
                }
            },
            "FLOAT" => self.b.float(digits.parse().unwrap()),
            "STRING" => match a0_syntax::unquote(text) {
                Ok(s) => self.b.string(&s),
                Err(msg) => {
                    self.b.report_code(code::INVALID_TOKEN, &msg);
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
        let name = self.field(node, "name");
        let sname = self.text(name);
        let braces = self
            .tree
            .children(node)
            .iter()
            .any(|&c| self.text(c) == "{");
        // `new object`: a dynamic object, whose fields are named at run time.
        if sname == "object" && !braces && self.opt(node, "module").is_none() {
            return self.b.new_object();
        }
        if !braces {
            self.at(name);
            self.b
                .report_code(code::SYNTAX, &format!("expected `{{` after `new {sname}`"));
            self.b
                .report_primary_label("only `new object` has no fields between braces");
            return self.b.error_expr();
        }
        // `new map {}`: an empty map, its key and value types inferred from its use.
        if sname == "map"
            && inits.is_empty()
            && self.opt(node, "module").is_none()
            && self.b.lookup_type_name("map").is_none()
        {
            let t = self.b.named_type("map", &[]);
            return self.b.new_map(t);
        }
        let found = match self.opt(node, "module") {
            Some(m) => {
                self.at(name);
                let item = self.b.member_item(self.text(m), sname, true);
                if item.is_none() {
                    return self.b.error_expr();
                }
                item
            }
            None => self.b.lookup_type_name(sname),
        };
        match found {
            Some(Item::Struct(s)) => self.b.new_struct_named(s, &names, &values),
            Some(_) => {
                self.at(name);
                self.b
                    .report_code(code::UNKNOWN_TYPE, &format!("`{sname}` is not a struct"));
                self.b.error_expr()
            }
            None => {
                self.at(name);
                self.b.named_type(sname, &[]);
                self.b.error_expr()
            }
        }
    }

    /// `{k: v, ...}` is `{ let m = new map; m[k] = v; ...; m }`, entries in the order written.
    fn map_lit(&mut self, node: NodeId) -> ExprId {
        let entries: Vec<(ExprId, ExprId)> = self
            .tree
            .nodes_of_kind(node, "entry")
            .into_iter()
            .map(|e| {
                let k = self.expr(self.field(e, "key"));
                let v = self.expr(self.field(e, "value"));
                (k, v)
            })
            .collect();
        self.at(node);
        let (k, v) = (self.b.fresh_type(), self.b.fresh_type());
        let t = self.b.map_type(k, v);
        let made = self.b.new_map(t);
        if entries.is_empty() {
            return made;
        }
        let body = self.b.current_body();
        let m = self.b.new_local(body, "map", t);
        let mut items = vec![self.b.let_(m, made)];
        for (k, v) in entries {
            let target = self.b.local(m);
            items.push(self.b.set_index(target, k, v));
        }
        let result = self.b.local(m);
        self.b.block(&items, Some(result))
    }

    /// The module alias `n` names, if it is a name that is an imported module (and not a local).
    fn module_alias(&self, n: NodeId) -> Option<&'a str> {
        let n = self.unwrap_primary(n);
        let text = self.text(n);
        (self.kind(n) == "name_expr"
            && self.b.lookup_local(text).is_none()
            && self.b.is_module(text))
        .then_some(text)
    }

    fn call(&mut self, node: NodeId) -> ExprId {
        let callee = self.unwrap_primary(self.field(node, "operand"));
        let args = self.args(node);
        // `json.parse(text)`: a function of an imported module.
        if self.kind(callee) == "get"
            && let Some(alias) = self.module_alias(self.field(callee, "operand"))
        {
            let name = self.field(callee, "name");
            self.at(node);
            self.b.set_name_span(Some(self.tree.span(name)));
            return self.b.call_member(alias, self.text(name), &args);
        }
        // `shape.area()`: a method, or a closure in a field.
        if self.kind(callee) == "get" {
            let receiver = self.expr(self.field(callee, "operand"));
            let name = self.field(callee, "name");
            self.at(node);
            self.b.set_name_span(Some(self.tree.span(name)));
            return self.b.method_call(receiver, self.text(name), &args);
        }
        if self.kind(callee) != "name_expr" {
            let c = self.expr(callee);
            self.at(node);
            return self.b.call_value(c, &args);
        }
        let name = self.text(callee);
        if matches!(name, "addr" | "offset" | "data")
            && self.unsafe_depth == 0
            && self.b.lookup_local(name).is_none()
            && self.b.lookup_value_name(name).is_none()
        {
            self.needs_unsafe(node, &format!("`{name}`"));
        }
        self.at(node);
        self.b.set_name_span(Some(self.tree.span(callee)));
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
            self.at(self.field(node, "scrutinee"));
            let msg = format!("cannot match on `{}`", self.b.module().display_type(t));
            self.b.report_code(code::NOT_AN_ENUM, &msg);
            self.b.report_primary_label("not an enum");
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
                    self.b.unknown_variant(e, self.text(v));
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
