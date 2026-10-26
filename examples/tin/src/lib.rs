//! Tin: a small C-like systems language built on A0.
//!
//! ```text
//! struct Vec2 { x: f64, y: f64 }
//! extern fn sqrt(x: f64) -> f64;
//!
//! fn length(v: *Vec2) -> f64 { return sqrt(v->x * v->x + v->y * v->y); }
//!
//! fn main() -> i32 {
//!   var points = alloc(sizeof(Vec2) * 2) as *Vec2;
//!   points[1] = (Vec2){ .x = 3.0, .y = 4.0 };
//!   println(length(&points[1]));
//!   free(points);
//!   return 0;
//! }
//! ```
//!
//! What a systems language needs from A0 (v0.6): structs are inline values laid out as C lays them
//! out, copied when assigned or passed; pointers are raw and unchecked (`*T`, `&x`, `*p`, `p[i]`,
//! `p->f`); memory is the program's (`memory manual`: `alloc` and `free`, no collector); and C
//! functions are declared and called directly, structs passed by value as C passes them. Arrays
//! (`[]T`) are the checked way: their bounds are checked and they grow. The compiler is one walk
//! over the syntax tree building HIR by name, like Minnow's.

use a0_diag::codes as code;
use a0_diag::{Diagnostics, SourceId};
use a0_driver::backend::{A0Symbol, HostSymbols, Library};
use a0_hir::{
    BinOp, BodyId, Builtin, ExprId, FuncId, HirBuilder, HirModule, Item, Memory, TypeId, TypeTag,
    UnOp, Warning,
};
use a0_syntax::{Grammar, NodeId, SyntaxTree};

/// The Tin grammar.
pub const GRAMMAR: &str = include_str!("../tin.a0g");

/// Loads the grammar. It is part of this crate, so failing to load it is a bug.
pub fn grammar() -> Grammar {
    match Grammar::from_str(GRAMMAR, SourceId(u32::MAX)) {
        Ok(g) => g,
        Err(d) => panic!("tin.a0g is invalid:\n{}", d.summary()),
    }
}

/// The C library Tin programs link with: the runtime's few functions, cJSON, and the functions of
/// structs by value the tests call.
pub fn runtime_library() -> Library {
    Library::new(
        "tin_rt",
        include_bytes!(concat!(env!("OUT_DIR"), "/tin_rt.embedded")),
    )
}

unsafe extern "C" {
    static tin_symbols: A0Symbol;
}

/// The C functions of [`runtime_library`], for the interpreter and the JIT.
pub fn host_symbols() -> HostSymbols {
    let mut symbols = HostSymbols::new();
    // SAFETY: `tin_symbols` is a NULL-terminated table defined in runtime/tin.c.
    unsafe { symbols.add_table(&raw const tin_symbols) };
    symbols
}

/// Compiles a program to HIR. `source` is the id of `text` in the caller's source map.
pub fn compile(text: &str, source: SourceId) -> Result<HirModule, Diagnostics> {
    let (tree, diags) = grammar().parse(text, source);
    if diags.has_errors() {
        return Err(diags);
    }
    let mut b = HirBuilder::new("tin");
    start(&mut b);
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

/// Sets up a builder for a Tin program: memory without a collector, how types are written, the
/// warnings, and the C functions `alloc` and `free` are made of.
fn start(b: &mut HirBuilder) {
    b.set_memory(Memory::Manual);
    b.set_type_syntax(TypeTag::Ptr, "*{}");
    b.set_type_syntax(TypeTag::Array, "[]{}");
    b.set_type_syntax(TypeTag::Int, "i64");
    b.set_type_syntax(TypeTag::Float, "f64");
    b.enable_warning(Warning::UnusedVariable);
    b.enable_warning(Warning::UnreachableCode);
    let bytes = b.ptr_type(TypeId::U8);
    b.declare_extern("calloc", &[TypeId::INT, TypeId::INT], bytes);
    b.declare_extern("free", &[bytes], TypeId::UNIT);
    // A C string as a string of A0's (copied), for printing.
    b.declare_extern("tin_string", &[bytes], TypeId::STRING);
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

    /// Attributes what is built next to `n`.
    fn at(&mut self, n: NodeId) {
        self.b.set_span(Some(self.tree.span(n)));
    }

    /// Attributes a declaration to its name, which is what messages point at.
    fn at_name(&mut self, n: NodeId) {
        let name = self.opt(n, "name").unwrap_or(n);
        self.at(name);
    }

    // ---------------------------------------------------------------- items

    fn program(&mut self) {
        let items = self.tree.nodes(self.tree.root());
        // Structs first, so that anything can use them, themselves included.
        for &item in &items {
            if self.kind(item) == "struct_decl" {
                self.at_name(item);
                let s = self.b.declare_struct(self.text(self.field(item, "name")));
                self.b.set_struct_inline(s);
            }
        }
        let mut funcs = Vec::new();
        for &item in &items {
            let name = self.text(self.field(item, "name"));
            match (self.kind(item), self.b.lookup_type_name(name)) {
                ("struct_decl", Some(Item::Struct(s))) => {
                    for f in self.tree.nodes_of_kind(item, "field") {
                        let t = self.ty(self.field(f, "type"));
                        self.at_name(f);
                        self.b.add_field(s, self.text(self.field(f, "name")), t);
                    }
                }
                ("extern_decl", _) => {
                    let (_, types, ret) = self.signature(item);
                    self.at_name(item);
                    self.b.declare_extern(name, &types, ret);
                }
                ("global_decl", _) => {
                    let t = self.ty(self.field(item, "type"));
                    self.at_name(item);
                    self.b.declare_global(name, t);
                }
                ("fn_decl", _) => {
                    let (names, types, ret) = self.signature(item);
                    self.at_name(item);
                    let f = self.b.declare_function(name, &names, &types, ret);
                    let body = self.b.function_body(f);
                    self.param_spans(body, item);
                    funcs.push((f, item));
                }
                _ => {}
            }
        }
        for (f, item) in funcs {
            self.function(f, item);
        }
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

    /// The parameter names and types and the result type of a function.
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

    fn function(&mut self, f: FuncId, item: NodeId) {
        let body = self.b.function_body(f);
        self.b.begin_body(body);
        let root = self.block(self.field(item, "body"));
        self.b.end_body();
        self.b.set_root(body, root);
    }

    fn ty(&mut self, node: NodeId) -> TypeId {
        match self.kind(node) {
            "ptr_type" => {
                let elem = self.ty(self.field(node, "elem"));
                self.b.ptr_type(elem)
            }
            "array_type" => {
                let elem = self.ty(self.field(node, "elem"));
                self.b.array_type(elem)
            }
            _ => {
                self.at(node);
                self.b.named_type(self.text(self.field(node, "name")), &[])
            }
        }
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
        match self.kind(node) {
            "var_stmt" => self.var(node),
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
                let value = self.opt(node, "value").map(|v| self.expr(v));
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
            _ => self.simple(self.field(node, "stmt")),
        }
    }

    /// `var x: T = e` (in a block or a `for`).
    fn var(&mut self, node: NodeId) -> ExprId {
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

    /// `for (init; cond; step) body` is
    /// `{ init; first = true; loop { if first { first = false } else { step }; if !cond { break }
    /// body } }`, so that `continue` runs the step; `first` has no name in scope.
    fn for_stmt(&mut self, node: NodeId) -> ExprId {
        self.b.push_scope();
        let init = self.opt(node, "init").map(|i| match self.kind(i) {
            "for_var" => self.var(i),
            _ => self.simple(i),
        });
        self.at(node);
        let first = self
            .b
            .new_local(self.b.current_body(), "first", TypeId::BOOL);
        let yes = self.b.bool_lit(true);
        let bind_first = self.b.let_(first, yes);
        let step = self.opt(node, "step").map(|s| self.simple(s));
        self.at(node);
        let no = self.b.bool_lit(false);
        let clear = self.b.set_local(first, no);
        let is_first = self.b.local(first);
        let advance = self.b.if_(is_first, clear, step);
        let mut items = vec![advance];
        if let Some(c) = self.opt(node, "cond") {
            let cond = self.expr(c);
            self.at(c);
            let stop = self.b.unary(UnOp::Not, cond);
            let leave = self.b.break_();
            items.push(self.b.if_(stop, leave, None));
        }
        items.push(self.block(self.field(node, "body")));
        self.at(node);
        let body = self.b.block(&items, None);
        let lp = self.b.loop_(body);
        self.b.pop_scope();
        let mut all: Vec<ExprId> = init.into_iter().collect();
        all.extend([bind_first, lp]);
        self.b.block(&all, None)
    }

    /// An expression, or an assignment: `place = e`, or `place op= e`, which is
    /// `place = place op e` (the place's operands are evaluated twice).
    fn simple(&mut self, node: NodeId) -> ExprId {
        let target = self.field(node, "target");
        let Some(op) = self.opt(node, "op") else {
            return self.expr(target);
        };
        let mut value = self.expr(self.field(node, "value"));
        let op_text = self.text(op);
        if op_text != "=" {
            let current = self.expr(target);
            self.at(op);
            let bin = BinOp::from_symbol(&op_text[..op_text.len() - 1]).unwrap();
            value = self.b.binary(bin, current, value);
        }
        self.assign(target, value)
    }

    fn assign(&mut self, target: NodeId, value: ExprId) -> ExprId {
        let target = self.unwrap_primary(target);
        match self.kind(target) {
            "name_expr" => {
                self.at(target);
                self.b.assign_named(self.text(target), value)
            }
            "get" | "arrow" => {
                let o = self.object(target);
                let name = self.field(target, "name");
                self.at(name);
                self.b.set_field_named(o, self.text(name), value)
            }
            "index" => {
                let o = self.expr(self.field(target, "operand"));
                let i = self.expr(self.field(target, "index"));
                self.at(target);
                self.b.set_index(o, i, value)
            }
            "unary" if self.text(self.field(target, "op")) == "*" => {
                let p = self.expr(self.field(target, "operand"));
                self.at(target);
                let zero = self.b.int(0);
                self.b.set_index(p, zero, value)
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

    /// The struct whose field `s.f` or `p->f` names: `s`, or what `p` points at.
    fn object(&mut self, node: NodeId) -> ExprId {
        let o = self.expr(self.field(node, "operand"));
        if self.kind(node) == "arrow" {
            self.at(node);
            let zero = self.b.int(0);
            return self.b.index(o, zero);
        }
        o
    }

    // ---------------------------------------------------------------- expressions

    fn unwrap_primary(&self, n: NodeId) -> NodeId {
        if self.kind(n) == "primary" {
            self.tree.child(n, 0)
        } else {
            n
        }
    }

    fn arg_nodes(&self, node: NodeId) -> Vec<NodeId> {
        match self.opt(node, "args") {
            Some(a) => self.tree.fields_named(a, "arg"),
            None => Vec::new(),
        }
    }

    fn expr(&mut self, node: NodeId) -> ExprId {
        let node = self.unwrap_primary(node);
        let sub = |c: &mut Self, name: &str| c.expr(c.field(node, name));
        match self.kind(node) {
            "literal" => self.literal(node),
            "paren" => sub(self, "inner"),
            "sizeof" => {
                let t = self.ty(self.field(node, "type"));
                self.at(node);
                self.b.size_of(t)
            }
            "compound" => self.compound(node),
            "array_lit" => {
                let elems: Vec<ExprId> = self
                    .arg_nodes(node)
                    .into_iter()
                    .map(|a| self.expr(a))
                    .collect();
                self.at(node);
                self.b.array(&elems)
            }
            "binary" | "logical" => {
                let (l, r) = (sub(self, "left"), sub(self, "right"));
                self.at(node);
                let op = self.text(self.field(node, "op"));
                // `p + n` and `p - n` move a pointer by `n` elements, as in C.
                if matches!(op, "+" | "-") && self.is_pointer(l) {
                    let n = match op {
                        "-" => self.b.unary(UnOp::Neg, r),
                        _ => r,
                    };
                    return self.b.builtin(Builtin::Offset, &[l, n]);
                }
                match op {
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
                    "!" => self.b.unary(UnOp::Not, a),
                    "~" => self.b.unary(UnOp::BitNot, a),
                    "*" => {
                        let zero = self.b.int(0);
                        self.b.index(a, zero)
                    }
                    _ => self.b.builtin(Builtin::Addr, &[a]),
                }
            }
            "cast" => {
                let a = sub(self, "operand");
                let t = self.ty(self.field(node, "type"));
                self.at(node);
                self.b.cast(a, t)
            }
            "call" => self.call(node),
            "get" | "arrow" => {
                let o = self.object(node);
                let name = self.field(node, "name");
                self.at(name);
                self.b.field_named(o, self.text(name))
            }
            "index" => {
                let (a, i) = (sub(self, "operand"), sub(self, "index"));
                self.at(node);
                self.b.index(a, i)
            }
            _ => {
                self.at(node);
                self.b.name(self.text(node))
            }
        }
    }

    /// Whether `e` is a pointer, as far as its type is known yet.
    fn is_pointer(&mut self, e: ExprId) -> bool {
        let t = self.b.type_of(e);
        let t = self.b.resolved_type(t);
        self.b.module().type_tag(t) == TypeTag::Ptr
    }

    fn literal(&mut self, node: NodeId) -> ExprId {
        let tok = self.tree.child(node, 0);
        let text = self.text(tok);
        self.at(node);
        let int = |c: &mut Self, v: Option<i64>| match v {
            Some(v) => c.b.int(v),
            None => {
                c.b.report_code(code::INVALID_TOKEN, "this integer does not fit in 64 bits");
                c.b.error_expr()
            }
        };
        match self.kind(tok) {
            "INT" => int(self, text.parse().ok()),
            // Hexadecimal literals may use all 64 bits, as C's unsigned constants do.
            "HEX" => int(
                self,
                u64::from_str_radix(&text[2..], 16).ok().map(|v| v as i64),
            ),
            "FLOAT" => self.b.float(text.parse().unwrap()),
            // `'a'`: the character's byte, a `u8`.
            "CHAR" => match a0_syntax::unquote(text).map(|s| s.into_bytes()) {
                Ok(bytes) if bytes.len() == 1 => {
                    let v = self.b.int(bytes[0] as i64);
                    self.b.cast(v, TypeId::U8)
                }
                _ => {
                    self.b
                        .report_code(code::INVALID_TOKEN, "a character literal is one byte");
                    self.b.error_expr()
                }
            },
            // A string literal is its bytes, NUL-terminated: a `*u8`, as C has them.
            "STRING" => match a0_syntax::unquote(text) {
                Ok(s) => {
                    let s = self.b.string(&s);
                    self.b.builtin(Builtin::Data, &[s])
                }
                Err(msg) => {
                    self.b.report_code(code::INVALID_TOKEN, &msg);
                    self.b.error_expr()
                }
            },
            _ => match text {
                // A null pointer, of whatever pointer type it is used as.
                "null" => {
                    let elem = self.b.fresh_type();
                    let t = self.b.ptr_type(elem);
                    let zero = self.b.int(0);
                    self.b.cast(zero, t)
                }
                _ => self.b.bool_lit(text == "true"),
            },
        }
    }

    /// `(S){ .f = v, ... }`, with values evaluated in the order written; fields not given are 0.
    fn compound(&mut self, node: NodeId) -> ExprId {
        let inits = self.tree.nodes_of_kind(node, "field_init");
        let mut names: Vec<&str> = inits
            .iter()
            .map(|&f| self.text(self.field(f, "name")))
            .collect();
        let mut values: Vec<ExprId> = inits
            .iter()
            .map(|&f| self.expr(self.field(f, "value")))
            .collect();
        let name = self.field(node, "name");
        let sname = self.text(name);
        self.at(name);
        let Some(Item::Struct(s)) = self.b.lookup_type_name(sname) else {
            // Reports the unknown name, with a close one if there is.
            self.b.named_type(sname, &[]);
            return self.b.error_expr();
        };
        // C zeroes the fields a compound literal leaves out.
        self.at(node);
        let fields = self.fields(s);
        for (fname, ft) in &fields {
            if !names.contains(&fname.as_str()) {
                names.push(fname);
                values.push(self.zero(*ft));
            }
        }
        self.b.new_struct_named(s, &names, &values)
    }

    fn fields(&self, s: a0_hir::StructId) -> Vec<(String, TypeId)> {
        let m = self.b.module();
        (0..m.struct_field_count(s))
            .map(|i| {
                let (n, t) = m.struct_field(s, i);
                (n.to_string(), t)
            })
            .collect()
    }

    /// The zero value of `t`: 0, false, a null pointer, or a struct of zeros.
    fn zero(&mut self, t: TypeId) -> ExprId {
        match self.b.module().type_tag(t) {
            TypeTag::Bool => self.b.bool_lit(false),
            TypeTag::Struct => {
                let Some(s) = self.b.module().struct_of(t) else {
                    return self.b.error_expr();
                };
                let fields = self.fields(s);
                let names: Vec<&str> = fields.iter().map(|(n, _)| n.as_str()).collect();
                let values: Vec<ExprId> = fields.iter().map(|&(_, ft)| self.zero(ft)).collect();
                self.b.new_struct_named(s, &names, &values)
            }
            TypeTag::Float | TypeTag::F32 => {
                let z = self.b.float(0.0);
                self.b.cast(z, t)
            }
            _ => {
                let z = self.b.int(0);
                self.b.cast(z, t)
            }
        }
    }

    fn call(&mut self, node: NodeId) -> ExprId {
        let callee = self.unwrap_primary(self.field(node, "operand"));
        let arg_nodes = self.arg_nodes(node);
        if self.kind(callee) != "name_expr" {
            self.at(callee);
            self.b
                .report_code(code::NOT_CALLABLE, "only functions can be called");
            self.b.report_primary_label("not a function");
            return self.b.error_expr();
        }
        let name = self.text(callee);
        if matches!(name, "print" | "println") {
            return self.print(node, &arg_nodes, name == "println");
        }
        let args: Vec<ExprId> = arg_nodes.iter().map(|&a| self.expr(a)).collect();
        self.at(node);
        self.b.set_name_span(Some(self.tree.span(callee)));
        let builtin = |c: &mut Self, op: Builtin, args: &[ExprId]| c.b.builtin(op, args);
        match (name, args.as_slice()) {
            ("len", [_]) => builtin(self, Builtin::Len, &args),
            ("push", [_, _]) => builtin(self, Builtin::Push, &args),
            ("pop", [_]) => builtin(self, Builtin::Pop, &args),
            ("exit", [_]) => builtin(self, Builtin::Exit, &args),
            // `alloc(n)`: `n` zeroed bytes from C's allocator.
            ("alloc", [n]) => {
                let one = self.b.int(1);
                self.b.call_named("calloc", &[*n, one])
            }
            // `free(p)`: a pointer goes back to C's allocator, an array to A0's.
            ("free", [a]) => {
                if self.is_pointer(*a) {
                    let bytes = self.b.ptr_type(TypeId::U8);
                    let p = self.b.cast(*a, bytes);
                    self.b.call_named("free", &[p])
                } else {
                    self.b.builtin(Builtin::Free, &args)
                }
            }
            _ => self.b.call_named(name, &args),
        }
    }

    /// `print(a, b, ...)`: each value in turn (a `*u8` as the C string it points at), and for
    /// `println` a newline after.
    fn print(&mut self, node: NodeId, args: &[NodeId], newline: bool) -> ExprId {
        let mut items = Vec::new();
        let c_string = self.b.ptr_type(TypeId::U8);
        for &a in args {
            let v = self.expr(a);
            let t = self.b.type_of(v);
            let t = self.b.resolved_type(t);
            self.at(a);
            if t == c_string {
                // Copied into a string of A0's to print, then freed.
                let s = self.b.call_named("tin_string", &[v]);
                let body = self.b.current_body();
                let l = self.b.new_local(body, "text", TypeId::STRING);
                items.push(self.b.let_(l, s));
                let text = self.b.local(l);
                items.push(self.b.builtin(Builtin::Print, &[text]));
                let text = self.b.local(l);
                items.push(self.b.builtin(Builtin::Free, &[text]));
            } else {
                items.push(self.b.builtin(Builtin::Print, &[v]));
            }
        }
        self.at(node);
        if newline {
            let nl = self.b.string("\n");
            items.push(self.b.builtin(Builtin::Print, &[nl]));
        }
        self.b.block(&items, None)
    }
}
