//! Kaleidoscope, the language of LLVM's tutorial, built on A0.
//!
//! The pipeline is the A0 one: the grammar (`kaleidoscope.a0g`) parses source into a syntax tree,
//! this module walks the tree (resolving names with `a0-sema` scopes and operator chains with
//! `a0_syntax::resolve_precedence`) and emits A0 IR with the builder, and the IR runs in the
//! interpreter or becomes a native executable.
//!
//! Semantics follow the tutorial through chapter 7: every value is a double; `if` treats nonzero
//! as true; `for` runs its body at least once and evaluates to 0; `var`/`in` and `=` give mutable
//! variables; `def binary| 5 (a b)` and `def unary!(v)` define operators, which may be used only after
//! their definition. Top-level expressions run in order when the program starts.

use std::collections::HashMap;

use a0_diag::{Diagnostic, Diagnostics, SourceId, Span};
use a0_ir::{Builder, FloatCC, FuncId, Module, Opcode, Type, Value, Var};
use a0_sema::{ScopeKind, Scopes};
use a0_syntax::{FieldId, Grammar, KindId, NodeId, Operand, SyntaxTree, resolve_precedence};

/// The Kaleidoscope grammar.
pub const GRAMMAR: &str = include_str!("../kaleidoscope.a0g");

/// Precedence of user-defined binary operators that do not give one.
pub const DEFAULT_PRECEDENCE: i32 = 30;

/// Loads the grammar. It is part of this crate, so failing to load it is a bug.
pub fn grammar() -> Grammar {
    match Grammar::from_str(GRAMMAR, SourceId(u32::MAX)) {
        Ok(g) => g,
        Err(d) => panic!("kaleidoscope.a0g is invalid:\n{}", d.summary()),
    }
}

/// Compiles a Kaleidoscope program to an IR module with a `@main` that runs its top-level
/// expressions. `source` is the id of `text` in the caller's `SourceMap` (used for spans).
pub fn compile(text: &str, source: SourceId) -> Result<Module, Diagnostics> {
    let g = grammar();
    let (tree, diags) = g.parse(text, source);
    if diags.has_errors() {
        return Err(diags);
    }
    let mut lower = Lower::new(&g, &tree);
    lower.program();
    let mut diags = lower.diags;
    if diags.has_errors() {
        return Err(diags);
    }
    match lower.b.finish() {
        Ok(m) => Ok(m),
        Err(ir_diags) => {
            // The compiler produced invalid IR: a bug in this crate, but report it rather than crash.
            diags.push(Diagnostic::error(
                "kaleidoscope.internal",
                "the compiler produced invalid IR",
            ));
            diags.extend(ir_diags);
            Err(diags)
        }
    }
}

/// The Kaleidoscope runtime library (`putchard`, `printd`), for linking native executables.
pub fn runtime_library() -> a0_native::Library {
    a0_native::Library {
        name: "kaleidoscope_rt".to_string(),
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/kaleidoscope_rt.embedded")).to_vec(),
    }
}

unsafe extern "C" {
    static kaleidoscope_symbols: a0_runtime::A0Symbol;
}

/// Host symbols for the interpreter: the A0 runtime, the C library and Kaleidoscope's runtime.
pub fn host_symbols() -> a0_interp::HostSymbols {
    let mut symbols = a0_interp::HostSymbols::new();
    // SAFETY: `kaleidoscope_symbols` is a NULL-terminated table defined in runtime/kaleidoscope.c.
    unsafe { symbols.add_table(&raw const kaleidoscope_symbols) };
    symbols
}

struct FuncInfo {
    id: FuncId,
    arity: usize,
    defined: bool,
}

/// Field and kind ids of the grammar, looked up once.
#[derive(Clone, Copy)]
struct Names {
    proto: FieldId,
    body: FieldId,
    expr: FieldId,
    op: FieldId,
    precedence: FieldId,
    params: FieldId,
    param: FieldId,
    name: FieldId,
    operand: FieldId,
    callee: FieldId,
    args: FieldId,
    arg: FieldId,
    inner: FieldId,
    cond: FieldId,
    then: FieldId,
    else_: FieldId,
    var: FieldId,
    start: FieldId,
    end: FieldId,
    step: FieldId,
    binding: FieldId,
    init: FieldId,
    definition: KindId,
    external: KindId,
    toplevel: KindId,
    expr_kind: KindId,
    unary: KindId,
    number: KindId,
    call: KindId,
    variable: KindId,
    paren: KindId,
    if_expr: KindId,
    for_expr: KindId,
    var_expr: KindId,
    binary_kw: KindId,
    unary_kw: KindId,
}

impl Names {
    fn new(g: &Grammar) -> Names {
        let f = |n: &str| {
            g.field(n)
                .unwrap_or_else(|| panic!("grammar has no field {n}"))
        };
        let k = |n: &str| {
            g.kind(n)
                .unwrap_or_else(|| panic!("grammar has no kind {n}"))
        };
        Names {
            proto: f("proto"),
            body: f("body"),
            expr: f("expr"),
            op: f("op"),
            precedence: f("precedence"),
            params: f("params"),
            param: f("param"),
            name: f("name"),
            operand: f("operand"),
            callee: f("callee"),
            args: f("args"),
            arg: f("arg"),
            inner: f("inner"),
            cond: f("cond"),
            then: f("then"),
            else_: f("else"),
            var: f("var"),
            start: f("start"),
            end: f("end"),
            step: f("step"),
            binding: f("binding"),
            init: f("init"),
            definition: k("definition"),
            external: k("external"),
            toplevel: k("toplevel"),
            expr_kind: k("expr"),
            unary: k("unary"),
            number: k("number"),
            call: k("call"),
            variable: k("variable"),
            paren: k("paren"),
            if_expr: k("if_expr"),
            for_expr: k("for_expr"),
            var_expr: k("var_expr"),
            binary_kw: g.literal("binary").unwrap(),
            unary_kw: g.literal("unary").unwrap(),
        }
    }
}

/// Marker for "an error was reported; stop lowering this function".
struct Reported;

type R<T> = Result<T, Reported>;

struct Lower<'a> {
    tree: &'a SyntaxTree,
    n: Names,
    b: Builder,
    funcs: HashMap<String, FuncInfo>,
    binops: HashMap<String, i32>,
    scopes: Scopes,
    diags: Diagnostics,
    toplevel: Vec<FuncId>,
}

impl<'a> Lower<'a> {
    fn new(g: &Grammar, tree: &'a SyntaxTree) -> Lower<'a> {
        let binops = [("=", 2), ("<", 10), ("+", 20), ("-", 20), ("*", 40)]
            .into_iter()
            .map(|(op, p)| (op.to_string(), p))
            .collect();
        Lower {
            tree,
            n: Names::new(g),
            b: Builder::new("kaleidoscope"),
            funcs: HashMap::new(),
            binops,
            scopes: Scopes::new(),
            diags: Diagnostics::new(),
            toplevel: Vec::new(),
        }
    }

    fn error<T>(&mut self, node: NodeId, message: &str) -> R<T> {
        self.diags
            .push(Diagnostic::error("kaleidoscope", message).with_span(self.tree.span(node)));
        Err(Reported)
    }

    fn span(&self, node: NodeId) -> Span {
        self.tree.span(node)
    }

    fn text(&self, node: NodeId) -> &'a str {
        self.tree.text(node)
    }

    fn field(&self, node: NodeId, f: FieldId) -> Option<NodeId> {
        self.tree.field(node, f)
    }

    fn program(&mut self) {
        let root = self.tree.root();
        for item in self.tree.children(root) {
            let kind = self.tree.kind(item);
            if kind == self.n.definition {
                self.definition(item);
            } else if kind == self.n.external {
                let proto = self.field(item, self.n.proto).unwrap();
                let _ = self.prototype(proto);
            } else if kind == self.n.toplevel {
                self.toplevel(item);
            }
            // `;` separators produce bare tokens; nothing to do.
        }
        let main = self.b.declare_function("main", &[], Some(Type::I32));
        self.b.begin_function(main);
        for f in std::mem::take(&mut self.toplevel) {
            self.b.call(f, &[]);
        }
        let zero = self.b.iconst(Type::I32, 0);
        self.b.ret(Some(zero));
        self.b.end_function();
    }

    /// Declares the function a prototype describes (or finds its earlier declaration) and returns
    /// it with its parameter names.
    fn prototype(&mut self, proto: NodeId) -> R<(FuncId, Vec<NodeId>)> {
        let n = self.n;
        let params = match self.field(proto, n.params) {
            Some(p) => self.tree.fields(p, n.param),
            None => Vec::new(),
        };
        let first = self.tree.child(proto, 0);
        let first_kind = self.tree.kind(first);
        let name = if first_kind == n.binary_kw || first_kind == n.unary_kw {
            let op = self.field(proto, n.op).unwrap();
            let binary = first_kind == n.binary_kw;
            let want = if binary { 2 } else { 1 };
            if params.len() != want {
                return self.error(
                    proto,
                    &format!(
                        "a {} operator takes {want} operand{}, not {}",
                        if binary { "binary" } else { "unary" },
                        if want == 1 { "" } else { "s" },
                        params.len()
                    ),
                );
            }
            if binary && let Some(p) = self.field(proto, n.precedence) {
                let value: f64 = self.text(p).parse().unwrap_or(-1.0);
                if !(1.0..=100.0).contains(&value) || value.fract() != 0.0 {
                    return self.error(
                        p,
                        "invalid precedence: must be a whole number from 1 to 100",
                    );
                }
            }
            format!(
                "{}{}",
                if binary { "binary" } else { "unary" },
                self.text(op)
            )
        } else {
            let name_node = self.field(proto, n.name).unwrap();
            let name = self.text(name_node).to_string();
            if name == "main" {
                return self.error(name_node, "'main' is reserved for the program entry point");
            }
            name
        };
        if let Some(info) = self.funcs.get(&name) {
            if info.arity != params.len() {
                let msg = format!(
                    "function '{name}' was declared with {} parameter(s), not {}",
                    info.arity,
                    params.len()
                );
                return self.error(proto, &msg);
            }
            return Ok((info.id, params));
        }
        let types = vec![Type::F64; params.len()];
        let id = self.b.declare_function(&name, &types, Some(Type::F64));
        self.funcs.insert(
            name,
            FuncInfo {
                id,
                arity: params.len(),
                defined: false,
            },
        );
        Ok((id, params))
    }

    fn definition(&mut self, def: NodeId) {
        let proto = self.field(def, self.n.proto).unwrap();
        let Ok((f, params)) = self.prototype(proto) else {
            return;
        };
        let name = self.b.module().func_name(f).to_string();
        if self.funcs[&name].defined {
            let _: R<()> = self.error(proto, &format!("function '{name}' cannot be redefined"));
            return;
        }
        self.funcs.get_mut(&name).unwrap().defined = true;
        // A binary operator's precedence is installed before its body, so the body may use it.
        let first = self.tree.child(proto, 0);
        if self.tree.kind(first) == self.n.binary_kw {
            let op = self.text(self.field(proto, self.n.op).unwrap()).to_string();
            if op == "=" || op == "<" || op == "+" || op == "-" || op == "*" {
                let _: R<()> = self.error(
                    proto,
                    &format!("cannot redefine the built-in operator '{op}'"),
                );
                return;
            }
            let prec = match self.field(proto, self.n.precedence) {
                Some(p) => self.text(p).parse::<f64>().unwrap_or(0.0) as i32,
                None => DEFAULT_PRECEDENCE,
            };
            self.binops.insert(op, prec);
        }
        let body = self.field(def, self.n.body).unwrap();
        self.b.set_span(Some(self.span(def)));
        self.b.begin_function(f);
        self.scopes.push(ScopeKind::Function);
        let result = self.function_body(&params, body);
        self.scopes.pop();
        if let Ok(v) = result {
            self.b.ret(Some(v));
        }
        self.b.end_function();
    }

    fn function_body(&mut self, params: &[NodeId], body: NodeId) -> R<Value> {
        // Parameters are mutable (chapter 7), so each lives in a variable.
        for (i, &p) in params.iter().enumerate() {
            let name = self.text(p);
            let var = self.b.declare_var(name, Type::F64);
            let value = self.b.param(i as u32);
            self.b.set(var, value);
            if self.scopes.declare_defined(name, var.0 as u64).is_err() {
                return self.error(p, &format!("duplicate parameter '{name}'"));
            }
        }
        self.expr(body)
    }

    fn toplevel(&mut self, item: NodeId) {
        let name = format!("toplevel.{}", self.toplevel.len());
        let f = self.b.declare_function(&name, &[], Some(Type::F64));
        let expr = self.field(item, self.n.expr).unwrap();
        self.b.set_span(Some(self.span(item)));
        self.b.begin_function(f);
        self.scopes.push(ScopeKind::Function);
        let result = self.expr(expr);
        self.scopes.pop();
        if let Ok(v) = result {
            self.b.ret(Some(v));
        }
        self.b.end_function();
        self.toplevel.push(f);
    }

    // ---------------------------------------------------------------- expressions

    fn expr(&mut self, node: NodeId) -> R<Value> {
        let n = self.n;
        let kind = self.tree.kind(node);
        if kind == n.expr_kind {
            self.chain(node)
        } else if kind == n.number {
            let text = self.text(node);
            let value = match text.parse::<f64>() {
                Ok(v) => v,
                Err(_) => return self.error(node, &format!("invalid number '{text}'")),
            };
            Ok(self.b.fconst(Type::F64, value))
        } else if kind == n.variable {
            let var = self.lookup(node)?;
            Ok(self.b.get(var))
        } else if kind == n.paren {
            let inner = self.field(node, n.inner).unwrap();
            self.expr(inner)
        } else if kind == n.call {
            self.call(node)
        } else if kind == n.unary {
            self.unary(node)
        } else if kind == n.if_expr {
            self.if_expr(node)
        } else if kind == n.for_expr {
            self.for_expr(node)
        } else if kind == n.var_expr {
            self.var_expr(node)
        } else {
            self.error(node, "unsupported expression")
        }
    }

    fn lookup(&mut self, name_node: NodeId) -> R<Var> {
        let name = self.text(name_node);
        match self.scopes.lookup(name) {
            Some(r) => Ok(Var(self.scopes.data(r.symbol) as u32)),
            None => self.error(name_node, &format!("unknown variable '{name}'")),
        }
    }

    fn call(&mut self, node: NodeId) -> R<Value> {
        let callee = self.field(node, self.n.callee).unwrap();
        let name = self.text(callee);
        let args = match self.field(node, self.n.args) {
            Some(a) => self.tree.fields(a, self.n.arg),
            None => Vec::new(),
        };
        let Some(info) = self.funcs.get(name) else {
            return self.error(callee, &format!("unknown function '{name}'"));
        };
        let (id, arity) = (info.id, info.arity);
        if arity != args.len() {
            return self.error(
                node,
                &format!(
                    "'{name}' takes {arity} argument(s) but {} were given",
                    args.len()
                ),
            );
        }
        let mut values = Vec::with_capacity(args.len());
        for a in args {
            values.push(self.expr(a)?);
        }
        Ok(self.b.call(id, &values).unwrap())
    }

    fn unary(&mut self, node: NodeId) -> R<Value> {
        let op = self.field(node, self.n.op).unwrap();
        let operand = self.field(node, self.n.operand).unwrap();
        let name = format!("unary{}", self.text(op));
        let Some(info) = self.funcs.get(&name) else {
            let text = self.text(op);
            return self.error(op, &format!("unknown unary operator '{text}'"));
        };
        let id = info.id;
        let v = self.expr(operand)?;
        Ok(self.b.call(id, &[v]).unwrap())
    }

    /// `operand (op operand)*`, resolved with the operator table as it stands at this point.
    fn chain(&mut self, node: NodeId) -> R<Value> {
        let operands = self.tree.fields(node, self.n.operand);
        let ops = self.tree.fields(node, self.n.op);
        if ops.is_empty() {
            return self.expr(operands[0]);
        }
        let mut precs = Vec::with_capacity(ops.len());
        for &op in &ops {
            match self.binops.get(self.text(op)) {
                Some(&p) => precs.push(p),
                None => {
                    let text = self.text(op);
                    return self.error(op, &format!("unknown binary operator '{text}'"));
                }
            }
        }
        // The tutorial's parser groups equal precedence to the left, '=' included.
        let steps = resolve_precedence(&precs, &vec![false; ops.len()]);
        let root = Operand::step(steps.len() as u32 - 1);
        self.emit_operand(&steps, &operands, &ops, root)
    }

    fn emit_operand(
        &mut self,
        steps: &[a0_syntax::Combine],
        operands: &[NodeId],
        ops: &[NodeId],
        o: Operand,
    ) -> R<Value> {
        if !o.is_step {
            return self.expr(operands[o.index as usize]);
        }
        let step = steps[o.index as usize];
        let op_node = ops[step.op as usize];
        let op = self.text(op_node);
        if op == "=" {
            // The destination must be a variable (possibly parenthesized).
            let target = if step.left.is_step {
                None
            } else {
                Some(self.unparen(operands[step.left.index as usize]))
            };
            let Some(target) = target.filter(|&t| self.tree.kind(t) == self.n.variable) else {
                return self.error(op_node, "destination of '=' must be a variable");
            };
            let value = self.emit_operand(steps, operands, ops, step.right)?;
            let var = self.lookup(target)?;
            self.b.set(var, value);
            return Ok(value);
        }
        let l = self.emit_operand(steps, operands, ops, step.left)?;
        let r = self.emit_operand(steps, operands, ops, step.right)?;
        Ok(match op {
            "+" => self.b.fadd(l, r),
            "-" => self.b.fsub(l, r),
            "*" => self.b.fmul(l, r),
            "<" => {
                // Unordered-or-less-than, like the tutorial's `fcmp ult`, as 0.0 or 1.0.
                let ge = self.b.fcmp(FloatCC::Ge, l, r);
                let t = self.b.iconst(Type::I1, 1);
                let ult = self.b.xor(ge, t);
                self.b.cast(Opcode::Uitofp, ult, Type::F64)
            }
            _ => {
                let id = self.funcs[&format!("binary{op}")].id;
                self.b.call(id, &[l, r]).unwrap()
            }
        })
    }

    fn unparen(&self, mut node: NodeId) -> NodeId {
        while self.tree.kind(node) == self.n.paren {
            node = self.field(node, self.n.inner).unwrap();
        }
        // A one-operand chain is just its operand.
        if self.tree.kind(node) == self.n.expr_kind && self.tree.fields(node, self.n.op).is_empty()
        {
            let inner = self.tree.fields(node, self.n.operand)[0];
            return self.unparen(inner);
        }
        node
    }

    /// `x != 0.0`, ordered (NaN counts as false), as the tutorial's `fcmp one`.
    fn truthy(&mut self, v: Value) -> Value {
        let zero = self.b.fconst(Type::F64, 0.0);
        let lt = self.b.fcmp(FloatCC::Lt, v, zero);
        let gt = self.b.fcmp(FloatCC::Gt, v, zero);
        self.b.or(lt, gt)
    }

    fn if_expr(&mut self, node: NodeId) -> R<Value> {
        let n = self.n;
        let (cond, then, els) = (
            self.field(node, n.cond).unwrap(),
            self.field(node, n.then).unwrap(),
            self.field(node, n.else_).unwrap(),
        );
        let c = self.expr(cond)?;
        let c = self.truthy(c);
        let result = self.b.declare_var("iftmp", Type::F64);
        self.b.if_(c);
        let v = self.expr(then)?;
        self.b.set(result, v);
        self.b.else_();
        let v = self.expr(els)?;
        self.b.set(result, v);
        self.b.end_if();
        Ok(self.b.get(result))
    }

    fn for_expr(&mut self, node: NodeId) -> R<Value> {
        let n = self.n;
        let var_node = self.field(node, n.var).unwrap();
        let start = self.field(node, n.start).unwrap();
        let end = self.field(node, n.end).unwrap();
        let step = self.field(node, n.step);
        let body = self.field(node, n.body).unwrap();
        // The start value is computed before the loop variable is in scope.
        let start_value = self.expr(start)?;
        let name = self.text(var_node);
        let var = self.b.declare_var(name, Type::F64);
        self.b.set(var, start_value);
        self.scopes.push(ScopeKind::Block);
        let _ = self.scopes.declare_defined(name, var.0 as u64);
        let result = self.for_loop(var, body, step, end);
        self.scopes.pop();
        result?;
        Ok(self.b.fconst(Type::F64, 0.0))
    }

    fn for_loop(&mut self, var: Var, body: NodeId, step: Option<NodeId>, end: NodeId) -> R<()> {
        // As in the tutorial: body, then step, then the end condition (seeing the old value), then
        // the increment; the body always runs at least once.
        self.b.loop_();
        self.expr(body)?;
        let step_value = match step {
            Some(s) => self.expr(s)?,
            None => self.b.fconst(Type::F64, 1.0),
        };
        let end_value = self.expr(end)?;
        let current = self.b.get(var);
        let next = self.b.fadd(current, step_value);
        self.b.set(var, next);
        let keep_going = self.truthy(end_value);
        let t = self.b.iconst(Type::I1, 1);
        let stop = self.b.xor(keep_going, t);
        self.b.if_(stop);
        self.b.break_();
        self.b.end_if();
        self.b.end_loop();
        Ok(())
    }

    fn var_expr(&mut self, node: NodeId) -> R<Value> {
        let bindings = self.tree.fields(node, self.n.binding);
        let body = self.field(node, self.n.body).unwrap();
        let mut opened = 0;
        let result = (|| -> R<Value> {
            for b in bindings {
                let name_node = self.field(b, self.n.name).unwrap();
                // The initializer is evaluated before its own variable is in scope.
                let init = match self.field(b, self.n.init) {
                    Some(i) => self.expr(i)?,
                    None => self.b.fconst(Type::F64, 0.0),
                };
                let name = self.text(name_node);
                let var = self.b.declare_var(name, Type::F64);
                self.b.set(var, init);
                // Each binding gets its own scope so a later binding may shadow an earlier one.
                self.scopes.push(ScopeKind::Block);
                opened += 1;
                let _ = self.scopes.declare_defined(name, var.0 as u64);
            }
            self.expr(body)
        })();
        for _ in 0..opened {
            self.scopes.pop();
        }
        result
    }
}
