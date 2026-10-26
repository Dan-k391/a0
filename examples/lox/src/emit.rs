//! HIR generation for Lox.
//!
//! Every Lox value is an `any`. Lox's runtime semantics (values, calls, classes, errors) are written
//! in HIR in `prelude.hir`; this module parses the prelude and adds to it one closure per Lox function
//! and a `main` that runs the script. Captured variables, garbage collection and lowering are the
//! HIR's business.

use std::collections::HashMap;

use a0_diag::SourceId;
use a0_hir::{
    BinOp, BodyId, Builtin, ExprId, GlobalId, HirBuilder, HirModule, Item, LocalId, TypeId, UnOp,
};
use a0_syntax::{NodeId, SyntaxTree};

use crate::Lines;
use crate::analyze::{Analysis, DeclId, Names, Place, is_variable, statements};

/// Lox's runtime, in HIR.
pub const PRELUDE: &str = include_str!("../prelude.hir");
/// Native functions over the standard library, added when a program uses one.
pub const NATIVES: &str = include_str!("../natives.hir");

/// Lox's native functions: the name, the prelude function that makes it, and whether it is one of
/// `NATIVES` (A0's extensions, which also need the standard library's modules).
const NATIVE_FUNCTIONS: &[(&str, &str, bool)] = &[
    ("clock", "native_clock", false),
    ("readLine", "native_read_line", true),
    ("readFile", "native_read_file", true),
    ("writeFile", "native_write_file", true),
    ("env", "native_env", true),
    ("number", "native_number", true),
    ("sqrt", "native_sqrt", true),
    ("floor", "native_floor", true),
];

struct Emit<'a> {
    tree: &'a SyntaxTree,
    n: Names,
    a: &'a Analysis,
    lines: &'a Lines<'a>,
    b: HirBuilder,
    /// `array<any>`, the type of call arguments.
    args_ty: TypeId,
    locals: HashMap<DeclId, LocalId>,
    /// Lox globals: the value, and whether it has been defined yet.
    globals: HashMap<String, (GlobalId, GlobalId)>,
    /// The body being built.
    body: BodyId,
    /// `this`, while building an initializer (which always returns it).
    init_this: Option<LocalId>,
}

/// Generates the HIR module for an analyzed program.
pub fn generate(tree: &SyntaxTree, n: &Names, a: &Analysis, lines: &Lines) -> HirModule {
    let prelude = match a0_hir::parse(PRELUDE, SourceId(u32::MAX)) {
        Ok(m) => m,
        Err(d) => panic!("prelude.hir is invalid:\n{}", d.summary()),
    };
    let mut b = HirBuilder::from_module(prelude);
    let args_ty = b.array_type(TypeId::ANY);
    let main = b.declare_function("main", &[], &[], TypeId::INT);
    let body = b.function_body(main);
    let mut e = Emit {
        tree,
        n: *n,
        a,
        lines,
        b,
        args_ty,
        locals: HashMap::new(),
        globals: HashMap::new(),
        body,
        init_this: None,
    };
    // Natives other than `clock` exist only in programs that name them, so that the others pay
    // for neither the standard library nor the exceptions natives can throw.
    let named = identifiers(tree);
    let used: Vec<(&str, &str, bool)> = NATIVE_FUNCTIONS
        .iter()
        .copied()
        .filter(|&(name, _, extension)| !extension || named.contains(name))
        .collect();
    if used.iter().any(|n| n.2) {
        for module in ["std.io", "std.fs", "std.env", "std.math", "std.str"] {
            e.b.import_module(module, module);
        }
        if let Err(d) = e.b.add_text(NATIVES, SourceId(u32::MAX)) {
            panic!("natives.hir is invalid:\n{}", d.summary());
        }
    }
    let mut items = Vec::new();
    for (name, make, _) in used {
        let (value, defined) = e.global(name);
        let native = e.rt(make, &[]);
        let t = e.b.bool_lit(true);
        items.push(e.b.set_global(value, native));
        items.push(e.b.set_global(defined, t));
    }
    let mut script = Vec::new();
    for s in statements(tree, tree.root()) {
        e.stmt(s, &mut script);
    }
    // An exception nothing catches ends the program like a runtime error.
    let script = e.b.block(&script, None);
    let error = e.b.new_local(body, "error", TypeId::ANY);
    let report = e.b.local(error);
    let report = e.rt("uncaught", &[report]);
    let handler = e.b.block(&[report], None);
    items.push(e.b.try_catch(script, error, handler));
    let zero = e.b.int(0);
    let root = e.b.block(&items, Some(zero));
    e.b.set_root(body, root);
    match e.b.finish() {
        Ok(m) => m,
        Err(d) => panic!("generated HIR is invalid:\n{}", d.summary()),
    }
}

/// The text of every identifier in the program.
fn identifiers(tree: &SyntaxTree) -> std::collections::HashSet<&str> {
    let mut out = std::collections::HashSet::new();
    let mut stack = vec![tree.root()];
    while let Some(node) = stack.pop() {
        if tree.is_token(node) {
            out.insert(tree.text(node));
        } else {
            stack.extend(tree.children(node));
        }
    }
    out
}

impl Emit<'_> {
    /// Calls a prelude function.
    fn rt(&mut self, name: &str, args: &[ExprId]) -> ExprId {
        match self.b.module().lookup_value(name) {
            Some(Item::Func(f)) => self.b.call(f, args),
            _ => panic!("prelude.hir has no fn {name}"),
        }
    }

    fn any(&mut self, e: ExprId) -> ExprId {
        self.b.cast(e, TypeId::ANY)
    }

    fn nil(&mut self) -> ExprId {
        let u = self.b.unit();
        self.any(u)
    }

    fn line(&mut self, node: NodeId) -> ExprId {
        let line = self.lines.of(self.tree, node);
        self.b.int(line as i64)
    }

    fn text(&mut self, node: NodeId) -> ExprId {
        self.b.string(self.tree.text(node))
    }

    fn truthy(&mut self, node: NodeId) -> ExprId {
        let v = self.expr(node);
        self.rt("truthy", &[v])
    }

    fn new_local(&mut self, name: &str) -> LocalId {
        self.b.new_local(self.body, name, TypeId::ANY)
    }

    fn local(&mut self, d: DeclId) -> LocalId {
        self.locals[&d]
    }

    /// The HIR globals for a Lox global, declared on first use. The prefixes keep them apart from
    /// each other and from the prelude.
    fn global(&mut self, name: &str) -> (GlobalId, GlobalId) {
        if let Some(&g) = self.globals.get(name) {
            return g;
        }
        let value = self.b.declare_global(&format!("g_{name}"), TypeId::ANY);
        let defined = self.b.declare_global(&format!("d_{name}"), TypeId::BOOL);
        self.globals.insert(name.to_string(), (value, defined));
        (value, defined)
    }

    /// `if !d_x { undefined_variable("x", line) }`
    fn check_defined(&mut self, defined: GlobalId, token: NodeId) -> ExprId {
        let d = self.b.global(defined);
        let not = self.b.unary(UnOp::Not, d);
        let name = self.text(token);
        let line = self.line(token);
        let error = self.rt("undefined_variable", &[name, line]);
        let then = self.b.block(&[error], None);
        self.b.if_(not, then, None)
    }

    /// Reads a variable; `token` names it.
    fn read(&mut self, place: Place, token: NodeId) -> ExprId {
        match place {
            Place::Local(d) => {
                let l = self.local(d);
                self.b.local(l)
            }
            Place::Global => {
                let (g, defined) = self.global(self.tree.text(token));
                let check = self.check_defined(defined, token);
                let value = self.b.global(g);
                self.b.block(&[check], Some(value))
            }
        }
    }

    /// Binds a declaration's value: a new local, or a global that is now defined.
    fn bind(&mut self, place: Place, token: NodeId, value: ExprId, out: &mut Vec<ExprId>) {
        match place {
            Place::Local(d) => {
                let l = self.new_local(self.tree.text(token));
                self.locals.insert(d, l);
                out.push(self.b.let_(l, value));
            }
            Place::Global => {
                let (g, defined) = self.global(self.tree.text(token));
                out.push(self.b.set_global(g, value));
                let t = self.b.bool_lit(true);
                out.push(self.b.set_global(defined, t));
            }
        }
    }

    /// A statement as a block of its own.
    fn block(&mut self, node: NodeId) -> ExprId {
        let mut items = Vec::new();
        self.stmt(node, &mut items);
        self.b.block(&items, None)
    }

    /// Appends a statement to `out`. Declarations stay in the enclosing block, so later statements
    /// see their locals.
    fn stmt(&mut self, node: NodeId, out: &mut Vec<ExprId>) {
        let n = self.n;
        let tree = self.tree;
        let kind = tree.kind(node);
        // Where the HIR comes from, for debuggers; Lox's own errors carry jlox's line numbers.
        self.b.set_span(Some(tree.span(node)));
        if kind == n.print_stmt {
            let v = self.expr(tree.field(node, n.value).unwrap());
            out.push(self.rt("print_value", &[v]));
        } else if kind == n.expr_stmt {
            out.push(self.expr(tree.field(node, n.expr).unwrap()));
        } else if kind == n.var_decl {
            let value = match tree.field(node, n.init) {
                Some(e) => self.expr(e),
                None => self.nil(),
            };
            let name = tree.field(node, n.name).unwrap();
            self.bind(self.a.defs[&node], name, value, out);
        } else if kind == n.fun_decl {
            let name = tree.field(node, n.name).unwrap();
            let place = self.a.defs[&node];
            // The variable exists before the closure, so the function can refer to itself.
            let nil = self.nil();
            self.bind(place, name, nil, out);
            let f = self.function(node, false);
            let f = self.any(f);
            out.push(self.assign(place, name, f));
        } else if kind == n.class_decl {
            self.class_decl(node, out);
        } else if kind == n.block {
            let mut items = Vec::new();
            for s in statements(tree, node) {
                self.stmt(s, &mut items);
            }
            out.push(self.b.block(&items, None));
        } else if kind == n.if_stmt {
            let c = self.truthy(tree.field(node, n.cond).unwrap());
            let then = self.block(tree.field(node, n.then).unwrap());
            let els = tree.field(node, n.else_).map(|e| self.block(e));
            out.push(self.b.if_(c, then, els));
        } else if kind == n.while_stmt {
            let c = self.truthy(tree.field(node, n.cond).unwrap());
            let body = self.block(tree.field(node, n.body).unwrap());
            out.push(self.b.while_(c, body));
        } else if kind == n.for_stmt {
            let mut items = Vec::new();
            if let Some(init) = tree.field(node, n.init) {
                self.stmt(init, &mut items);
            }
            let c = match tree.field(node, n.cond) {
                Some(c) => self.truthy(c),
                None => self.b.bool_lit(true),
            };
            let mut body = Vec::new();
            self.stmt(tree.field(node, n.body).unwrap(), &mut body);
            if let Some(incr) = tree.field(node, n.incr) {
                body.push(self.expr(incr));
            }
            let body = self.b.block(&body, None);
            items.push(self.b.while_(c, body));
            out.push(self.b.block(&items, None));
        } else if kind == n.throw_stmt {
            let v = self.expr(tree.field(node, n.value).unwrap());
            let line = self.line(tree.child(node, 0));
            out.push(self.rt("throw_value", &[v, line]));
        } else if kind == n.try_stmt {
            self.try_stmt(node, out);
        } else if kind == n.return_stmt {
            let v = match (self.init_this, tree.field(node, n.value)) {
                (Some(this), _) => self.b.local(this),
                (None, Some(value)) => self.expr(value),
                (None, None) => self.nil(),
            };
            out.push(self.b.return_(Some(v)));
        }
    }

    /// `try { body } catch (name) { handler }`. The call depth is restored for the handler: the
    /// calls the exception left never counted themselves out.
    fn try_stmt(&mut self, node: NodeId, out: &mut Vec<ExprId>) {
        let n = self.n;
        let tree = self.tree;
        let Some(Item::Global(depth)) = self.b.module().lookup_value("depth") else {
            panic!("prelude.hir has no global depth");
        };
        let saved = self.b.new_local(self.body, "depth", TypeId::INT);
        let now = self.b.global(depth);
        let save = self.b.let_(saved, now);
        let body = self.block(tree.field(node, n.body).unwrap());
        let error = self.b.new_local(self.body, "error", TypeId::ANY);
        let then = self.b.local(saved);
        let mut handler = vec![self.b.set_global(depth, then)];
        let name = tree.field(node, n.name).unwrap();
        let e = self.b.local(error);
        let value = self.rt("caught", &[e]);
        self.bind(self.a.defs[&name], name, value, &mut handler);
        for s in statements(tree, tree.field(node, n.handler).unwrap()) {
            self.stmt(s, &mut handler);
        }
        let handler = self.b.block(&handler, None);
        let t = self.b.try_catch(body, error, handler);
        out.push(self.b.block(&[save, t], None));
    }

    /// Assigns a variable and yields the value.
    fn assign(&mut self, place: Place, token: NodeId, value: ExprId) -> ExprId {
        match place {
            Place::Local(d) => {
                let l = self.local(d);
                let set = self.b.set_local(l, value);
                let get = self.b.local(l);
                self.b.block(&[set], Some(get))
            }
            Place::Global => {
                // The value is evaluated before the variable is checked, as in jlox.
                let (g, defined) = self.global(self.tree.text(token));
                let t = self.new_local("value");
                let bind = self.b.let_(t, value);
                let check = self.check_defined(defined, token);
                let v = self.b.local(t);
                let set = self.b.set_global(g, v);
                let v = self.b.local(t);
                self.b.block(&[bind, check, set], Some(v))
            }
        }
    }

    /// A `LoxFunction` for a function or method declaration. Its code takes the receiver (`()` for
    /// plain functions) and the arguments: up to three directly, more in an array. The prelude's
    /// `call` functions have checked their number.
    fn function(&mut self, node: NodeId, initializer: bool) -> ExprId {
        let n = self.n;
        let tree = self.tree;
        let params = match tree.field(node, n.params) {
            Some(p) => tree.fields(p, n.param),
            None => Vec::new(),
        };
        let direct = params.len() <= 3;
        let (names, types) = if direct {
            let names: Vec<&str> = std::iter::once("this")
                .chain(params.iter().map(|&p| tree.text(p)))
                .collect();
            (names, vec![TypeId::ANY; params.len() + 1])
        } else {
            (vec!["this", "args"], vec![TypeId::ANY, self.args_ty])
        };
        let body = self
            .b
            .new_closure_body(self.body, &names, &types, TypeId::ANY);
        if let Some(name) = tree.field(node, n.name) {
            self.b.set_closure_name(body, tree.text(name));
        }
        let outer = (self.body, self.init_this);
        self.body = body;
        let this = self.b.param(body, 0);
        self.init_this = initializer.then_some(this);
        if let Some(&d) = self.a.this_decls.get(&node) {
            self.locals.insert(d, this);
        }
        let mut items = Vec::new();
        for (i, &p) in params.iter().enumerate() {
            if direct {
                let Place::Local(d) = self.a.defs[&p] else {
                    unreachable!("parameters are locals")
                };
                self.locals.insert(d, self.b.param(body, i as u32 + 1));
                continue;
            }
            let args = self.b.param(body, 1);
            let args = self.b.local(args);
            let i = self.b.int(i as i64);
            let value = self.b.index(args, i);
            self.bind(self.a.defs[&p], p, value, &mut items);
        }
        for s in statements(tree, tree.field(node, n.body).unwrap()) {
            self.stmt(s, &mut items);
        }
        let result = if initializer {
            self.b.local(this)
        } else {
            self.nil()
        };
        let root = self.b.block(&items, Some(result));
        self.b.set_root(body, root);
        (self.body, self.init_this) = outer;

        let Some(Item::Struct(s)) = self.b.module().lookup_type("LoxFunction") else {
            panic!("prelude.hir has no struct LoxFunction");
        };
        let name = self.text(tree.field(node, n.name).unwrap());
        let arity = self.b.int(params.len() as i64);
        let code = self.b.lambda(body);
        // Fields `code`, `code0` .. `code3`: this function's code in the one for its arity.
        let slot = if direct { params.len() + 1 } else { 0 };
        let mut fields = vec![name, arity];
        for k in 0..5 {
            if k == slot {
                fields.push(code);
                continue;
            }
            let t = match k {
                0 => self.b.fn_type(&[TypeId::ANY, self.args_ty], TypeId::ANY),
                k => self.b.fn_type(&vec![TypeId::ANY; k], TypeId::ANY),
            };
            fields.push(self.b.null(t));
        }
        self.b.new_struct(s, &fields)
    }

    fn class_decl(&mut self, node: NodeId, out: &mut Vec<ExprId>) {
        let n = self.n;
        let tree = self.tree;
        let name = tree.field(node, n.name).unwrap();
        let place = self.a.defs[&node];
        if let Place::Local(_) = place {
            let nil = self.nil();
            self.bind(place, name, nil, out);
        }
        let superclass = match (
            tree.field(node, n.superclass),
            self.a.super_decls.get(&node),
        ) {
            (Some(sup), Some(&d)) => {
                let value = self.read(self.a.uses[&sup], sup);
                let l = self.new_local("super");
                self.locals.insert(d, l);
                out.push(self.b.let_(l, value));
                let v = self.b.local(l);
                let line = self.line(sup);
                out.push(self.rt("check_superclass", &[v, line]));
                self.b.local(l)
            }
            _ => self.nil(),
        };
        if place == Place::Global {
            let nil = self.nil();
            self.bind(place, name, nil, out);
        }
        let class_name = self.text(name);
        let class = self.rt("new_class", &[class_name, superclass]);
        let k = self.b.new_local(self.body, "class", self.b.type_of(class));
        out.push(self.b.let_(k, class));
        for m in tree.fields(node, n.methods) {
            let method_name = tree.field(m, n.name).unwrap();
            let f = self.function(m, tree.text(method_name) == "init");
            let k_ref = self.b.local(k);
            let methods = self.b.field_named(k_ref, "methods");
            let key = self.text(method_name);
            out.push(self.b.builtin(Builtin::SetField, &[methods, key, f]));
        }
        let k_ref = self.b.local(k);
        let class = self.any(k_ref);
        out.push(self.assign(place, name, class));
    }

    fn expr(&mut self, node: NodeId) -> ExprId {
        let n = self.n;
        let tree = self.tree;
        let kind = tree.kind(node);
        self.b.set_span(Some(tree.span(node)));
        if kind == n.primary {
            self.primary(node)
        } else if kind == n.assign {
            let left = tree.field(node, n.left).unwrap();
            let right = tree.field(node, n.right).unwrap();
            if is_variable(tree, &n, left) {
                let value = self.expr(right);
                self.assign(self.a.uses[&node], tree.child(left, 0), value)
            } else {
                // The object is checked before the value is evaluated, as in jlox.
                let object = self.expr(tree.field(left, n.operand).unwrap());
                let name = tree.field(left, n.name).unwrap();
                let o = self.new_local("object");
                let bind = self.b.let_(o, object);
                let o_ref = self.b.local(o);
                let line = self.line(name);
                let check = self.rt("check_fields", &[o_ref, line]);
                let o_ref = self.b.local(o);
                let key = self.text(name);
                let value = self.expr(right);
                let set = self.rt("set_property", &[o_ref, key, value]);
                self.b.block(&[bind, check], Some(set))
            }
        } else if kind == n.logical {
            // `a and b` is `a` if `a` is falsey, else `b`; `a or b` the other way around.
            let left = self.expr(tree.field(node, n.left).unwrap());
            let t = self.new_local("left");
            let bind = self.b.let_(t, left);
            let t_ref = self.b.local(t);
            let c = self.rt("truthy", &[t_ref]);
            let right = self.expr(tree.field(node, n.right).unwrap());
            let t_ref = self.b.local(t);
            let is_and = tree.kind(tree.field(node, n.op).unwrap()) == n.kw_and;
            let (then, els) = if is_and {
                (right, t_ref)
            } else {
                (t_ref, right)
            };
            let choice = self.b.if_(c, then, Some(els));
            self.b.block(&[bind], Some(choice))
        } else if kind == n.binary {
            let op = tree.field(node, n.op).unwrap();
            let a = self.expr(tree.field(node, n.left).unwrap());
            let b = self.expr(tree.field(node, n.right).unwrap());
            let f = match tree.text(op) {
                "==" | "!=" => {
                    let op = BinOp::from_symbol(tree.text(op)).unwrap();
                    let eq = self.b.binary(op, a, b);
                    return self.any(eq);
                }
                "+" => "add",
                "-" => "subtract",
                "*" => "multiply",
                "/" => "divide",
                "<" => "less",
                "<=" => "less_equal",
                ">" => "greater",
                _ => "greater_equal",
            };
            let line = self.line(op);
            self.rt(f, &[a, b, line])
        } else if kind == n.unary {
            let op = tree.field(node, n.op).unwrap();
            let operand = tree.field(node, n.operand).unwrap();
            if tree.text(op) == "-" {
                let v = self.expr(operand);
                let line = self.line(op);
                self.rt("negate", &[v, line])
            } else {
                let c = self.truthy(operand);
                let not = self.b.unary(UnOp::Not, c);
                self.any(not)
            }
        } else if kind == n.call {
            let callee_node = tree.field(node, n.operand).unwrap();
            // `o.m(...)` and `super.m(...)` find what they call, then call it on the object
            // without binding it to the object first (`find_method` in the prelude).
            let method = self.method_callee(callee_node);
            let (callee, receiver) = match method {
                Some((_, callee, receiver)) => (callee, Some(receiver)),
                None => (self.expr(callee_node), None),
            };
            let args = match tree.field(node, n.args) {
                Some(a) => tree.fields(a, n.arg),
                None => Vec::new(),
            };
            let args: Vec<ExprId> = args.into_iter().map(|a| self.expr(a)).collect();
            let paren = tree.child(node, tree.child_count(node) - 1);
            let line = self.line(paren);
            let call = match receiver {
                // `call0` .. `call3` take the arguments directly.
                None if args.len() <= 3 => {
                    let mut all = vec![callee];
                    all.extend(args);
                    all.push(line);
                    self.rt(&format!("call{}", all.len() - 2), &all)
                }
                None => {
                    let args = self.b.array(&args);
                    self.rt("call", &[callee, args, line])
                }
                Some(receiver) if args.len() <= 3 => {
                    let mut all = vec![callee, receiver];
                    all.extend(args);
                    all.push(line);
                    self.rt(&format!("call_with{}", all.len() - 3), &all)
                }
                Some(receiver) => {
                    let args = self.b.array(&args);
                    self.rt("call_with", &[callee, receiver, args, line])
                }
            };
            match method {
                Some((bind, _, _)) => self.b.block(&[bind], Some(call)),
                None => call,
            }
        } else {
            // `get`
            let object = self.expr(tree.field(node, n.operand).unwrap());
            let name = tree.field(node, n.name).unwrap();
            let key = self.text(name);
            let line = self.line(name);
            self.rt("get_property", &[object, key, line])
        }
    }

    /// For a callee `o.m` or `super.m`: a `let` of the object, what to call (found as the call's
    /// first operand, so before the arguments, as in jlox) and the object to call it on.
    fn method_callee(&mut self, node: NodeId) -> Option<(ExprId, ExprId, ExprId)> {
        let n = self.n;
        let tree = self.tree;
        let (object, name, find) = if tree.kind(node) == n.get {
            let object = self.expr(tree.field(node, n.operand).unwrap());
            (object, tree.field(node, n.name).unwrap(), "find_method")
        } else if tree.kind(node) == n.primary && tree.kind(tree.child(node, 0)) == n.kw_super {
            let this = self.local(self.a.super_this[&node]);
            let this = self.b.local(this);
            (this, tree.field(node, n.method).unwrap(), "find_super")
        } else {
            return None;
        };
        self.b.set_span(Some(tree.span(node)));
        let receiver = self.b.new_local(self.body, "receiver", TypeId::ANY);
        let bind = self.b.let_(receiver, object);
        let key = self.text(name);
        let line = self.line(name);
        let found = if find == "find_method" {
            let r = self.b.local(receiver);
            self.rt(find, &[r, key, line])
        } else {
            let sup = self.read(self.a.uses[&node], tree.child(node, 0));
            self.rt(find, &[sup, key, line])
        };
        let r = self.b.local(receiver);
        Some((bind, found, r))
    }

    fn primary(&mut self, node: NodeId) -> ExprId {
        let n = self.n;
        let tree = self.tree;
        let first = tree.child(node, 0);
        let fk = tree.kind(first);
        let value = if fk == n.number {
            let value: f64 = tree
                .text(first)
                .parse()
                .expect("the grammar only accepts valid numbers");
            self.b.float(value)
        } else if fk == n.string {
            let text = tree.text(first);
            self.b.string(&text[1..text.len() - 1])
        } else if fk == n.kw_true || fk == n.kw_false {
            self.b.bool_lit(fk == n.kw_true)
        } else if fk == n.kw_nil {
            return self.nil();
        } else if fk == n.identifier || fk == n.kw_this {
            return self.read(self.a.uses[&node], first);
        } else if fk == n.kw_super {
            let method = tree.field(node, n.method).unwrap();
            let sup = self.read(self.a.uses[&node], first);
            let this = self.local(self.a.super_this[&node]);
            let this = self.b.local(this);
            let name = self.text(method);
            let line = self.line(method);
            return self.rt("get_super", &[sup, this, name, line]);
        } else {
            return self.expr(tree.field(node, n.inner).unwrap());
        };
        self.any(value)
    }
}
