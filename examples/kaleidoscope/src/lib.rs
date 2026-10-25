//! Kaleidoscope, the language of LLVM's tutorial, built on A0.
//!
//! The grammar (`kaleidoscope.a0g`) parses a program into a syntax tree. This module walks the tree
//! once and builds A0's high-level IR by name: the HIR builder resolves names, checks types and
//! reports errors, and A0 optimizes the result and runs it in the interpreter or compiles it to a
//! native executable. `docs/tutorial.md` builds this file step by step.
//!
//! Semantics follow the tutorial through chapter 7: every value is a double; `if` treats anything
//! but 0.0 and NaN as true; `for` runs its body at least once and evaluates to 0; `var`/`in` and
//! `=` give mutable variables; `def binary| 5 (a b)` and `def unary!(v)` define operators, which may
//! be used only after their definition. Top-level expressions run in order when the program starts.

use std::collections::{HashMap, HashSet};

use a0_diag::{Diagnostics, SourceId};
use a0_hir::{BinOp, BodyId, ExprId, FuncId, HirBuilder, HirModule, TypeId, UnOp};
use a0_syntax::{Grammar, NodeId, Operand, SyntaxTree, resolve_precedence};

/// The precedence of a binary operator that does not give one.
pub const DEFAULT_PRECEDENCE: i32 = 30;

// ANCHOR: grammar
/// The Kaleidoscope grammar.
pub const GRAMMAR: &str = include_str!("../kaleidoscope.a0g");

/// Loads the grammar. It is part of this crate, so failing to load it is a bug.
pub fn grammar() -> Grammar {
    match Grammar::from_str(GRAMMAR, SourceId(u32::MAX)) {
        Ok(g) => g,
        Err(d) => panic!("kaleidoscope.a0g is invalid:\n{}", d.summary()),
    }
}
// ANCHOR_END: grammar

// ANCHOR: compile
/// Compiles a Kaleidoscope program to a HIR module whose `main` runs its top-level expressions.
/// `source` is the id of `text` in the caller's `SourceMap`, for the spans of errors.
pub fn compile(text: &str, source: SourceId) -> Result<HirModule, Diagnostics> {
    let (tree, diags) = grammar().parse(text, source);
    if diags.has_errors() {
        return Err(diags);
    }
    let mut c = Compiler::new(&tree);
    c.program();
    c.b.finish()
}
// ANCHOR_END: compile

// ANCHOR: runtime
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
// ANCHOR_END: runtime

// ANCHOR: compiler
/// A function or operator declared with `def` or `extern`.
struct Function {
    arity: usize,
    /// `None` for an `extern`, a C function.
    func: Option<FuncId>,
    defined: bool,
}

struct Compiler<'a> {
    tree: &'a SyntaxTree,
    b: HirBuilder,
    /// Functions and operators (`binary|`, `unary!`) declared so far.
    functions: HashMap<String, Function>,
    /// Binary operators and their precedence, built-in ones included.
    binary_ops: HashMap<String, i32>,
    /// The body of `main`, and the top-level expressions it runs.
    main: BodyId,
    toplevel: Vec<ExprId>,
}

impl<'a> Compiler<'a> {
    fn new(tree: &'a SyntaxTree) -> Compiler<'a> {
        let mut b = HirBuilder::new("kaleidoscope");
        let main = b.declare_function("main", &[], &[], TypeId::INT);
        let binary_ops = [("=", 2), ("<", 10), ("+", 20), ("-", 20), ("*", 40)];
        Compiler {
            tree,
            main: b.function_body(main),
            b,
            functions: HashMap::new(),
            binary_ops: binary_ops.map(|(op, p)| (op.to_string(), p)).into(),
            toplevel: Vec::new(),
        }
    }

    // Reading the tree. Fields the grammar always produces are unwrapped.

    fn field(&self, node: NodeId, name: &str) -> NodeId {
        self.tree.field_named(node, name).unwrap()
    }

    fn opt(&self, node: NodeId, name: &str) -> Option<NodeId> {
        self.tree.field_named(node, name)
    }

    fn text(&self, node: NodeId) -> &'a str {
        self.tree.text(node)
    }

    /// Errors and the nodes built next are reported at `node`.
    fn at(&mut self, node: NodeId) {
        self.b.set_span(Some(self.tree.span(node)));
    }

    /// Reports one of Kaleidoscope's own errors; the result stands in for the expression.
    fn error(&mut self, node: NodeId, message: &str) -> ExprId {
        self.at(node);
        self.b.report(message);
        self.b.error_expr()
    }
}
// ANCHOR_END: compiler

// ANCHOR: program
impl Compiler<'_> {
    fn program(&mut self) {
        for item in self.tree.nodes(self.tree.root()) {
            match self.tree.kind_name(item) {
                "definition" => self.definition(item),
                "external" => {
                    let _ = self.prototype(self.field(item, "proto"), true);
                }
                _ => self.toplevel(item),
            }
        }
        let zero = self.b.int(0);
        let root = self.b.block(&self.toplevel, Some(zero));
        self.b.set_root(self.main, root);
    }

    /// A top-level expression runs in `main`, which ignores its value.
    fn toplevel(&mut self, item: NodeId) {
        self.b.begin_body(self.main);
        let e = self.expr(self.field(item, "expr"));
        self.b.end_body();
        self.toplevel.push(e);
    }
}
// ANCHOR_END: program

// ANCHOR: functions
impl Compiler<'_> {
    /// Declares the function or operator a prototype describes, or checks it against an earlier
    /// declaration. Returns its name, or `None` after reporting an error.
    fn prototype(&mut self, proto: NodeId, external: bool) -> Option<String> {
        let params = match self.opt(proto, "params") {
            Some(p) => self.tree.fields_named(p, "param"),
            None => Vec::new(),
        };
        let name = match self.text(self.tree.child(proto, 0)) {
            kind @ ("binary" | "unary") => self.operator(proto, kind, params.len())?,
            _ => self.text(self.field(proto, "name")).to_string(),
        };
        if name == "main" {
            self.error(proto, "'main' is reserved for the program entry point");
            return None;
        }
        let mut seen = HashSet::new();
        for &p in &params {
            if !seen.insert(self.text(p)) {
                let msg = format!("duplicate parameter '{}'", self.text(p));
                self.error(p, &msg);
                return None;
            }
        }
        if let Some(f) = self.functions.get(&name) {
            if f.arity != params.len() {
                let msg = format!(
                    "function '{name}' was declared with {} parameter(s), not {}",
                    f.arity,
                    params.len()
                );
                self.error(proto, &msg);
                return None;
            }
            return Some(name);
        }
        self.at(proto);
        let types = vec![TypeId::FLOAT; params.len()];
        let func = if external {
            self.b.declare_extern(&name, &types, TypeId::FLOAT);
            None
        } else {
            let names: Vec<&str> = params.iter().map(|&p| self.text(p)).collect();
            Some(
                self.b
                    .declare_function(&name, &names, &types, TypeId::FLOAT),
            )
        };
        let arity = params.len();
        let defined = false;
        self.functions.insert(
            name.clone(),
            Function {
                arity,
                func,
                defined,
            },
        );
        Some(name)
    }

    fn definition(&mut self, def: NodeId) {
        let proto = self.field(def, "proto");
        let Some(name) = self.prototype(proto, false) else {
            return;
        };
        let f = &self.functions[&name];
        let (defined, func) = (f.defined, f.func);
        let Some(func) = func.filter(|_| !defined) else {
            self.error(proto, &format!("function '{name}' cannot be redefined"));
            return;
        };
        self.functions.get_mut(&name).unwrap().defined = true;
        let body = self.b.function_body(func);
        self.b.begin_body(body);
        let root = self.expr(self.field(def, "body"));
        self.b.end_body();
        self.b.set_root(body, root);
    }
}
// ANCHOR_END: functions

// ANCHOR: expr
impl Compiler<'_> {
    fn expr(&mut self, node: NodeId) -> ExprId {
        match self.tree.kind_name(node) {
            "expr" => self.chain(node),
            "number" => {
                let value: f64 = self.text(node).parse().unwrap_or(f64::NAN);
                self.b.float(value)
            }
            "variable" => {
                self.at(node);
                self.b.name(self.text(node))
            }
            "paren" => self.expr(self.field(node, "inner")),
            "call" => self.call(node),
            // ANCHOR_END: expr
            // ANCHOR: expr_control
            "if_expr" => self.if_expr(node),
            "for_expr" => self.for_expr(node),
            // ANCHOR_END: expr_control
            // ANCHOR: expr_unary
            "unary" => self.unary(node),
            // ANCHOR_END: expr_unary
            // ANCHOR: expr_var
            "var_expr" => self.var_expr(node),
            // ANCHOR_END: expr_var
            // ANCHOR: expr_end
            _ => self.error(node, "unsupported expression"),
        }
    }

    fn call(&mut self, node: NodeId) -> ExprId {
        let callee = self.text(self.field(node, "callee"));
        let args = match self.opt(node, "args") {
            Some(a) => self.tree.fields_named(a, "arg"),
            None => Vec::new(),
        };
        let args: Vec<ExprId> = args.into_iter().map(|a| self.expr(a)).collect();
        self.at(node);
        self.b.call_named(callee, &args)
    }
}
// ANCHOR_END: expr_end

// ANCHOR: chain
impl Compiler<'_> {
    /// `operand (op operand)*`, grouped with the operator table as it stands at this point.
    fn chain(&mut self, node: NodeId) -> ExprId {
        let operands = self.tree.fields_named(node, "operand");
        let ops = self.tree.fields_named(node, "op");
        let mut precedences = Vec::new();
        for &op in &ops {
            match self.binary_ops.get(self.text(op)) {
                Some(&p) => precedences.push(p),
                None => {
                    let msg = format!("unknown binary operator '{}'", self.text(op));
                    return self.error(op, &msg);
                }
            }
        }
        if ops.is_empty() {
            return self.expr(operands[0]);
        }
        // The tutorial's parser groups equal precedence to the left, '=' included.
        let steps = resolve_precedence(&precedences, &vec![false; ops.len()]);
        let last = Operand::step(steps.len() as u32 - 1);
        self.combine(&steps, &operands, &ops, last)
    }

    /// Builds one operand of a chain: an operand node, or a step that combines two.
    fn combine(
        &mut self,
        steps: &[a0_syntax::Combine],
        operands: &[NodeId],
        ops: &[NodeId],
        operand: Operand,
    ) -> ExprId {
        if !operand.is_step {
            return self.expr(operands[operand.index as usize]);
        }
        let step = steps[operand.index as usize];
        let op = ops[step.op as usize];
        if self.text(op) == "=" {
            return self.assign(steps, operands, ops, step);
        }
        let l = self.combine(steps, operands, ops, step.left);
        let r = self.combine(steps, operands, ops, step.right);
        self.at(op);
        match self.text(op) {
            "+" => self.b.binary(BinOp::Add, l, r),
            "-" => self.b.binary(BinOp::Sub, l, r),
            "*" => self.b.binary(BinOp::Mul, l, r),
            "<" => {
                // The tutorial's `fcmp ult` (true if either is NaN), as 0.0 or 1.0.
                let ge = self.b.binary(BinOp::Ge, l, r);
                let (no, yes) = (self.b.float(0.0), self.b.float(1.0));
                self.b.if_(ge, no, Some(yes))
            }
            // ANCHOR_END: chain
            // ANCHOR: chain_user
            op => self.b.call_named(&format!("binary{op}"), &[l, r]),
            // ANCHOR_END: chain_user
            // ANCHOR: chain_end
        }
    }
}
// ANCHOR_END: chain_end

// ANCHOR: control
impl Compiler<'_> {
    /// Whether `value` is true: neither 0.0 nor NaN, as the tutorial's `fcmp one`.
    fn truthy(&mut self, value: ExprId) -> ExprId {
        let local = self
            .b
            .new_local(self.b.current_body(), "cond", TypeId::FLOAT);
        let set = self.b.let_(local, value);
        let (x, zero) = (self.b.local(local), self.b.float(0.0));
        let below = self.b.binary(BinOp::Lt, x, zero);
        let (x, zero) = (self.b.local(local), self.b.float(0.0));
        let above = self.b.binary(BinOp::Gt, x, zero);
        let either = self.b.or(below, above);
        self.b.block(&[set], Some(either))
    }

    fn if_expr(&mut self, node: NodeId) -> ExprId {
        let cond = self.expr(self.field(node, "cond"));
        let cond = self.truthy(cond);
        let then = self.expr(self.field(node, "then"));
        let els = self.expr(self.field(node, "else"));
        self.b.if_(cond, then, Some(els))
    }

    /// `for i = start, end, step in body`: the body runs, then the step and the end condition are
    /// computed, then `i` steps, and the loop stops if the condition was false. Its value is 0.
    fn for_expr(&mut self, node: NodeId) -> ExprId {
        let start = self.expr(self.field(node, "start"));
        self.b.push_scope();
        let var = self.text(self.field(node, "var"));
        let init = self.b.let_named(var, TypeId::FLOAT, start);
        let i = self.b.lookup_local(var).unwrap();
        let body = self.expr(self.field(node, "body"));
        let step = match self.opt(node, "step") {
            Some(s) => self.expr(s),
            None => self.b.float(1.0),
        };
        let end = self.expr(self.field(node, "end"));
        self.b.pop_scope();
        // The step and the condition live in locals of their own, out of the program's reach.
        let body_id = self.b.current_body();
        let step_local = self.b.new_local(body_id, "step", TypeId::FLOAT);
        let going_local = self.b.new_local(body_id, "going", TypeId::BOOL);
        let set_step = self.b.let_(step_local, step);
        let going = self.truthy(end);
        let set_going = self.b.let_(going_local, going);
        let (current, by) = (self.b.local(i), self.b.local(step_local));
        let next = self.b.binary(BinOp::Add, current, by);
        let advance = self.b.set_local(i, next);
        let going = self.b.local(going_local);
        let stop = self.b.unary(UnOp::Not, going);
        let exit = self.b.break_();
        let exit = self.b.if_(stop, exit, None);
        let iteration = self
            .b
            .block(&[body, set_step, set_going, advance, exit], None);
        let repeat = self.b.loop_(iteration);
        let zero = self.b.float(0.0);
        self.b.block(&[init, repeat], Some(zero))
    }
}
// ANCHOR_END: control

// ANCHOR: operators
impl Compiler<'_> {
    /// The name of an operator a prototype defines (`binary|`, `unary!`), after checking its
    /// operands and precedence. A binary operator's precedence takes effect at once, so its own
    /// body may use it.
    fn operator(&mut self, proto: NodeId, kind: &str, params: usize) -> Option<String> {
        let op = self.text(self.field(proto, "op"));
        let operands = if kind == "binary" { 2 } else { 1 };
        if params != operands {
            let msg = format!("a {kind} operator takes {operands} operand(s), not {params}");
            self.error(proto, &msg);
            return None;
        }
        if kind == "binary" {
            if ["=", "<", "+", "-", "*"].contains(&op) {
                let msg = format!("cannot redefine the built-in operator '{op}'");
                self.error(proto, &msg);
                return None;
            }
            let precedence = match self.opt(proto, "precedence") {
                Some(p) => match self.text(p).parse::<f64>() {
                    Ok(v) if (1.0..=100.0).contains(&v) && v.fract() == 0.0 => v as i32,
                    _ => {
                        self.error(
                            p,
                            "invalid precedence: must be a whole number from 1 to 100",
                        );
                        return None;
                    }
                },
                None => DEFAULT_PRECEDENCE,
            };
            self.binary_ops.insert(op.to_string(), precedence);
        }
        Some(format!("{kind}{op}"))
    }

    fn unary(&mut self, node: NodeId) -> ExprId {
        let op = self.field(node, "op");
        let name = format!("unary{}", self.text(op));
        if !self.functions.contains_key(&name) {
            let msg = format!("unknown unary operator '{}'", self.text(op));
            return self.error(op, &msg);
        }
        let operand = self.expr(self.field(node, "operand"));
        self.at(node);
        self.b.call_named(&name, &[operand])
    }
}
// ANCHOR_END: operators

// ANCHOR: variables
impl Compiler<'_> {
    /// `var a = 1, b in body`: each initializer runs before its own variable is in scope.
    fn var_expr(&mut self, node: NodeId) -> ExprId {
        self.b.push_scope();
        let mut lets = Vec::new();
        for binding in self.tree.fields_named(node, "binding") {
            let init = match self.opt(binding, "init") {
                Some(i) => self.expr(i),
                None => self.b.float(0.0),
            };
            let name = self.text(self.field(binding, "name"));
            self.at(binding);
            lets.push(self.b.let_named(name, TypeId::FLOAT, init));
        }
        let body = self.expr(self.field(node, "body"));
        self.b.pop_scope();
        self.b.block(&lets, Some(body))
    }

    /// `x = value` sets the variable `x` and evaluates to the value.
    fn assign(
        &mut self,
        steps: &[a0_syntax::Combine],
        operands: &[NodeId],
        ops: &[NodeId],
        step: a0_syntax::Combine,
    ) -> ExprId {
        let op = ops[step.op as usize];
        let target = match step.left.is_step {
            false => Some(self.unparen(operands[step.left.index as usize])),
            true => None,
        };
        let Some(target) = target.filter(|&t| self.tree.kind_name(t) == "variable") else {
            return self.error(op, "destination of '=' must be a variable");
        };
        let value = self.combine(steps, operands, ops, step.right);
        let name = self.text(target);
        self.at(target);
        let set = self.b.assign_named(name, value);
        let get = self.b.name(name);
        self.b.block(&[set], Some(get))
    }

    /// `(x)` or a one-operand chain is just its operand.
    fn unparen(&self, mut node: NodeId) -> NodeId {
        loop {
            match self.tree.kind_name(node) {
                "paren" => node = self.field(node, "inner"),
                "expr" if self.tree.fields_named(node, "op").is_empty() => {
                    node = self.field(node, "operand");
                }
                _ => return node,
            }
        }
    }
}
// ANCHOR_END: variables
