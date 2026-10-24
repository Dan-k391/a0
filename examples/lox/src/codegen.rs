//! IR generation for Lox.
//!
//! Every Lox value lives in a slot of the function's frame on the runtime's shadow stack, so the
//! garbage collector always sees it: expressions evaluate into a destination slot, and IR values that
//! hold Lox values never live across a runtime call that might allocate. Variables captured by
//! closures live in heap cells (the slot holds the cell); closures carry the cells they capture.
//!
//! Each Lox function becomes an IR function `(ptr window) -> i64`, where the window is the caller's
//! slots `[callee, receiver, args...]` prepared by `lox_call_prepare`. The script becomes
//! `@lox.script`, and `@main` initializes the runtime, registers globals and string constants, and runs
//! it.

use std::collections::HashMap;

use a0_ir::{Builder, FuncId, IntCC, Module, Type, Value as IrValue};
use a0_syntax::{NodeId, SyntaxTree};

use crate::Lines;
use crate::analyze::{Analysis, DeclId, FnId, FnKind, Names, Place, is_variable, statements};

const QNAN: u64 = 0x7ffc_0000_0000_0000;
const NIL: i64 = (QNAN | 1) as i64;
const FALSE: i64 = (QNAN | 2) as i64;
const TRUE: i64 = (QNAN | 3) as i64;

/// The Lox runtime functions, declared as IR externs.
struct Runtime {
    init: FuncId,
    global_name: FuncId,
    string_constant: FuncId,
    constant: FuncId,
    frame_enter: FuncId,
    frame_leave: FuncId,
    global_get: FuncId,
    global_set: FuncId,
    global_define: FuncId,
    print: FuncId,
    equal: FuncId,
    not_equal: FuncId,
    not: FuncId,
    add: FuncId,
    subtract: FuncId,
    multiply: FuncId,
    divide: FuncId,
    less: FuncId,
    less_equal: FuncId,
    greater: FuncId,
    greater_equal: FuncId,
    negate: FuncId,
    cell_new: FuncId,
    cell_get: FuncId,
    cell_set: FuncId,
    closure_new: FuncId,
    closure_set_cell: FuncId,
    closure_cell: FuncId,
    native_clock: FuncId,
    class_new: FuncId,
    check_superclass: FuncId,
    class_inherit: FuncId,
    class_add_method: FuncId,
    get_property: FuncId,
    check_instance: FuncId,
    set_property: FuncId,
    super_get: FuncId,
    call_prepare: FuncId,
}

impl Runtime {
    fn declare(b: &mut Builder) -> Runtime {
        use Type::{I64, Ptr};
        let mut f = |name: &str, params: &[Type], ret: Option<Type>| b.declare_extern(name, params, ret);
        Runtime {
            init: f("lox_init", &[I64, I64], None),
            global_name: f("lox_global_name", &[I64, Ptr, I64], None),
            string_constant: f("lox_string_constant", &[I64, Ptr, I64], None),
            constant: f("lox_constant", &[I64], Some(I64)),
            frame_enter: f("lox_frame_enter", &[I64], Some(Ptr)),
            frame_leave: f("lox_frame_leave", &[I64], None),
            global_get: f("lox_global_get", &[I64, I64], Some(I64)),
            global_set: f("lox_global_set", &[I64, I64, I64], None),
            global_define: f("lox_global_define", &[I64, I64], None),
            print: f("lox_print", &[I64], None),
            equal: f("lox_equal", &[I64, I64], Some(I64)),
            not_equal: f("lox_not_equal", &[I64, I64], Some(I64)),
            not: f("lox_not", &[I64], Some(I64)),
            add: f("lox_add", &[I64, I64, I64], Some(I64)),
            subtract: f("lox_subtract", &[I64, I64, I64], Some(I64)),
            multiply: f("lox_multiply", &[I64, I64, I64], Some(I64)),
            divide: f("lox_divide", &[I64, I64, I64], Some(I64)),
            less: f("lox_less", &[I64, I64, I64], Some(I64)),
            less_equal: f("lox_less_equal", &[I64, I64, I64], Some(I64)),
            greater: f("lox_greater", &[I64, I64, I64], Some(I64)),
            greater_equal: f("lox_greater_equal", &[I64, I64, I64], Some(I64)),
            negate: f("lox_negate", &[I64, I64], Some(I64)),
            cell_new: f("lox_cell_new", &[I64], Some(I64)),
            cell_get: f("lox_cell_get", &[I64], Some(I64)),
            cell_set: f("lox_cell_set", &[I64, I64], None),
            closure_new: f("lox_closure_new", &[Ptr, I64, I64, I64], Some(I64)),
            closure_set_cell: f("lox_closure_set_cell", &[I64, I64, I64], None),
            closure_cell: f("lox_closure_cell", &[I64, I64], Some(I64)),
            native_clock: f("lox_native_clock", &[], Some(I64)),
            class_new: f("lox_class_new", &[I64], Some(I64)),
            check_superclass: f("lox_check_superclass", &[I64, I64], None),
            class_inherit: f("lox_class_inherit", &[I64, I64], None),
            class_add_method: f("lox_class_add_method", &[I64, I64, I64], None),
            get_property: f("lox_get_property", &[I64, I64, I64], Some(I64)),
            check_instance: f("lox_check_instance", &[I64, I64], None),
            set_property: f("lox_set_property", &[I64, I64, I64, I64], Some(I64)),
            super_get: f("lox_super_get", &[I64, I64, I64, I64], Some(I64)),
            call_prepare: f("lox_call_prepare", &[Ptr, I64, I64], Some(Ptr)),
        }
    }
}

/// Code generation state for the function being built.
struct FnState {
    kind: FnKind,
    frame: IrValue,
    /// The frame size constant, patched once the body is done.
    slot_count: IrValue,
    window: Option<IrValue>,
    next: u32,
    max: u32,
    slots: HashMap<DeclId, u32>,
}

struct Gen<'a> {
    tree: &'a SyntaxTree,
    n: Names,
    a: &'a Analysis,
    lines: &'a Lines<'a>,
    b: Builder,
    rt: Runtime,
    funcs: Vec<FuncId>,
    constants: Vec<String>,
    constant_index: HashMap<String, u32>,
    f: Option<FnState>,
}

/// Generates the IR module for an analyzed program.
pub fn generate(tree: &SyntaxTree, n: &Names, a: &Analysis, lines: &Lines) -> Module {
    let mut b = Builder::new("lox");
    let rt = Runtime::declare(&mut b);
    let mut funcs = Vec::new();
    for (id, info) in a.functions.iter().enumerate() {
        let f = if info.kind == FnKind::Script {
            b.declare_function("lox.script", &[], None)
        } else {
            b.declare_function(&format!("lox.{}.{id}", info.name), &[Type::Ptr], Some(Type::I64))
        };
        funcs.push(f);
    }
    let mut g = Gen {
        tree,
        n: *n,
        a,
        lines,
        b,
        rt,
        funcs,
        constants: Vec::new(),
        constant_index: HashMap::new(),
        f: None,
    };
    for id in 0..a.functions.len() {
        g.function(id);
    }
    g.main();
    match g.b.finish() {
        Ok(m) => m,
        Err(d) => panic!("the Lox compiler produced invalid IR:\n{}", d.summary()),
    }
}

impl Gen<'_> {
    fn st(&mut self) -> &mut FnState {
        self.f.as_mut().unwrap()
    }

    // ---------------------------------------------------------------- slots

    fn alloc(&mut self) -> u32 {
        let st = self.st();
        let s = st.next;
        st.next += 1;
        st.max = st.max.max(st.next);
        s
    }

    fn mark(&mut self) -> u32 {
        self.st().next
    }

    fn release(&mut self, mark: u32) {
        self.st().next = mark;
    }

    fn slot_ptr(&mut self, s: u32) -> IrValue {
        let frame = self.st().frame;
        let off = self.b.iconst(Type::I64, 8 * s as i64);
        self.b.ptradd(frame, off)
    }

    fn load(&mut self, s: u32) -> IrValue {
        let p = self.slot_ptr(s);
        self.b.load(Type::I64, p)
    }

    fn store(&mut self, s: u32, v: IrValue) {
        let p = self.slot_ptr(s);
        self.b.store(v, p);
    }

    fn konst(&mut self, bits: i64) -> IrValue {
        self.b.iconst(Type::I64, bits)
    }

    fn window(&mut self, i: u32) -> IrValue {
        let w = self.st().window.unwrap();
        let off = self.b.iconst(Type::I64, 8 * i as i64);
        let p = self.b.ptradd(w, off);
        self.b.load(Type::I64, p)
    }

    // ---------------------------------------------------------------- helpers

    fn call(&mut self, f: FuncId, args: &[IrValue]) -> IrValue {
        self.b.call(f, args).expect("runtime function returns a value")
    }

    fn call_void(&mut self, f: FuncId, args: &[IrValue]) {
        self.b.call(f, args);
    }

    fn line(&mut self, node: NodeId) -> IrValue {
        let line = self.lines.of(self.tree, node);
        self.konst(line as i64)
    }

    fn constant_index(&mut self, s: &str) -> u32 {
        if let Some(&i) = self.constant_index.get(s) {
            return i;
        }
        let i = self.constants.len() as u32;
        self.constants.push(s.to_string());
        self.constant_index.insert(s.to_string(), i);
        i
    }

    /// The interned string constant `s` (constants are rooted by the runtime).
    fn constant(&mut self, s: &str) -> IrValue {
        let i = self.constant_index(s);
        let idx = self.konst(i as i64);
        let c = self.rt.constant;
        self.call(c, &[idx])
    }

    fn truthy(&mut self, v: IrValue) -> IrValue {
        let nil = self.konst(NIL);
        let f = self.konst(FALSE);
        let is_nil = self.b.icmp(IntCC::Eq, v, nil);
        let is_false = self.b.icmp(IntCC::Eq, v, f);
        let falsey = self.b.or(is_nil, is_false);
        self.b.not(falsey)
    }

    fn captured(&self, d: DeclId) -> bool {
        self.a.decls[d].captured
    }

    fn slot_of(&mut self, d: DeclId) -> u32 {
        *self
            .st()
            .slots
            .get(&d)
            .expect("every variable in scope has a slot")
    }

    fn read(&mut self, place: Place, token: NodeId, dst: u32) {
        match place {
            Place::Local(d) => {
                let s = self.slot_of(d);
                let mut v = self.load(s);
                if self.captured(d) {
                    let get = self.rt.cell_get;
                    v = self.call(get, &[v]);
                }
                self.store(dst, v);
            }
            Place::Global(i) => {
                let idx = self.konst(i as i64);
                let line = self.line(token);
                let get = self.rt.global_get;
                let v = self.call(get, &[idx, line]);
                self.store(dst, v);
            }
        }
    }

    fn write(&mut self, place: Place, token: NodeId, src: u32) {
        match place {
            Place::Local(d) => {
                let s = self.slot_of(d);
                let v = self.load(src);
                if self.captured(d) {
                    let cell = self.load(s);
                    let set = self.rt.cell_set;
                    self.call_void(set, &[cell, v]);
                } else {
                    self.store(s, v);
                }
            }
            Place::Global(i) => {
                let idx = self.konst(i as i64);
                let v = self.load(src);
                let line = self.line(token);
                let set = self.rt.global_set;
                self.call_void(set, &[idx, v, line]);
            }
        }
    }

    /// Gives a new local its slot, holding `init`'s value (or a cell holding it, if captured).
    fn bind_local(&mut self, d: DeclId, value_slot: u32) -> u32 {
        let s = self.slot_of(d);
        let v = self.load(value_slot);
        if self.captured(d) {
            let new = self.rt.cell_new;
            let cell = self.call(new, &[v]);
            self.store(s, cell);
        } else {
            self.store(s, v);
        }
        s
    }

    fn leave(&mut self) {
        let count = self.st().slot_count;
        let leave = self.rt.frame_leave;
        self.call_void(leave, &[count]);
    }

    // ---------------------------------------------------------------- functions

    fn function(&mut self, id: FnId) {
        let info = &self.a.functions[id];
        let kind = info.kind;
        self.b.begin_function(self.funcs[id]);
        let window = (kind != FnKind::Script).then(|| self.b.param(0));
        let slot_count = self.b.iconst(Type::I64, 0);
        let enter = self.rt.frame_enter;
        let frame = self.call(enter, &[slot_count]);
        self.f = Some(FnState {
            kind,
            frame,
            slot_count,
            window,
            next: 0,
            max: 0,
            slots: HashMap::new(),
        });

        // Captured outer variables: copy the cells out of the closure.
        for (k, &d) in self.a.functions[id].upvalues.iter().enumerate() {
            let s = self.alloc();
            self.st().slots.insert(d, s);
            let closure = self.window(0);
            let k = self.konst(k as i64);
            let get = self.rt.closure_cell;
            let cell = self.call(get, &[closure, k]);
            self.store(s, cell);
        }
        if let Some(this) = self.a.functions[id].this_decl {
            let s = self.alloc();
            self.st().slots.insert(this, s);
            let v = self.window(1);
            let v = if self.captured(this) {
                let new = self.rt.cell_new;
                self.call(new, &[v])
            } else {
                v
            };
            self.store(s, v);
        }
        for (i, &d) in self.a.functions[id].params.iter().enumerate() {
            let s = self.alloc();
            self.st().slots.insert(d, s);
            let v = self.window(2 + i as u32);
            let v = if self.captured(d) {
                let new = self.rt.cell_new;
                self.call(new, &[v])
            } else {
                v
            };
            self.store(s, v);
        }

        let body = match self.a.functions[id].node {
            Some(node) => statements(self.tree, self.tree.field(node, self.n.body).unwrap()),
            None => statements(self.tree, self.tree.root()),
        };
        for s in body {
            self.stmt(s);
        }
        if !self.b.is_terminated() {
            match kind {
                FnKind::Script => {
                    self.leave();
                    self.b.ret(None);
                }
                FnKind::Initializer => self.return_this(),
                FnKind::Function | FnKind::Method => {
                    self.leave();
                    let nil = self.konst(NIL);
                    self.b.ret(Some(nil));
                }
            }
        }
        let st = self.f.take().unwrap();
        self.b.patch_iconst(st.slot_count, st.max as i64);
        self.b.end_function();
    }

    fn return_this(&mut self) {
        let this = self.a.functions[self.current_fn_index()].this_decl.unwrap();
        let t = self.alloc();
        let tok = self.tree.root();
        self.read(Place::Local(this), tok, t);
        let v = self.load(t);
        self.leave();
        self.b.ret(Some(v));
    }

    fn current_fn_index(&self) -> FnId {
        let current = self.b.current_function().unwrap();
        self.funcs.iter().position(|&f| f == current).unwrap()
    }

    /// `@main`: initialize the runtime, register globals and constants, define natives, run the script.
    fn main(&mut self) {
        let main = self.b.declare_function("main", &[], Some(Type::I32));
        let mut name_data = Vec::new();
        for (i, name) in self.a.globals.iter().enumerate() {
            name_data.push(self.b.add_data(&format!("lox.global.{i}"), name.as_bytes()));
        }
        let mut const_data = Vec::new();
        for (i, s) in self.constants.iter().enumerate() {
            const_data.push(self.b.add_data(&format!("lox.str.{i}"), s.as_bytes()));
        }
        self.b.begin_function(main);
        let ng = self.b.iconst(Type::I64, self.a.globals.len() as i64);
        let nc = self.b.iconst(Type::I64, self.constants.len() as i64);
        self.b.call(self.rt.init, &[ng, nc]);
        for (i, data) in name_data.into_iter().enumerate() {
            let idx = self.b.iconst(Type::I64, i as i64);
            let p = self.b.global_addr(data);
            let len = self.b.iconst(Type::I64, self.a.globals[i].len() as i64);
            self.b.call(self.rt.global_name, &[idx, p, len]);
        }
        for (i, data) in const_data.into_iter().enumerate() {
            let idx = self.b.iconst(Type::I64, i as i64);
            let p = self.b.global_addr(data);
            let len = self.b.iconst(Type::I64, self.constants[i].len() as i64);
            self.b.call(self.rt.string_constant, &[idx, p, len]);
        }
        let clock = self.b.call(self.rt.native_clock, &[]).unwrap();
        let idx = self.b.iconst(Type::I64, self.a.clock_global as i64);
        self.b.call(self.rt.global_define, &[idx, clock]);
        self.b.call(self.funcs[0], &[]);
        let zero = self.b.iconst(Type::I32, 0);
        self.b.ret(Some(zero));
        self.b.end_function();
    }

    /// Creates a closure for function `id` in slot `dst`, capturing its upvalues' cells.
    fn closure(&mut self, id: FnId, dst: u32) {
        let info = &self.a.functions[id];
        let arity = info.params.len() as i64;
        let upvalues = info.upvalues.clone();
        let name = info.name.clone();
        let code = self.b.func_addr(self.funcs[id]);
        let name = self.constant(&name);
        let arity = self.konst(arity);
        let count = self.konst(upvalues.len() as i64);
        let new = self.rt.closure_new;
        let c = self.call(new, &[code, name, arity, count]);
        self.store(dst, c);
        for (k, d) in upvalues.into_iter().enumerate() {
            let s = self.slot_of(d);
            let cell = self.load(s);
            let closure = self.load(dst);
            let k = self.konst(k as i64);
            let set = self.rt.closure_set_cell;
            self.call_void(set, &[closure, k, cell]);
        }
    }

    // ---------------------------------------------------------------- statements

    fn stmt(&mut self, node: NodeId) {
        if self.b.is_terminated() {
            // Code after a return in the same block never runs.
            return;
        }
        let n = self.n;
        let tree = self.tree;
        let kind = tree.kind(node);
        if kind == n.print_stmt {
            let t = self.alloc();
            self.expr(tree.field(node, n.value).unwrap(), t);
            let v = self.load(t);
            let print = self.rt.print;
            self.call_void(print, &[v]);
            self.release(t);
        } else if kind == n.expr_stmt {
            let t = self.alloc();
            self.expr(tree.field(node, n.expr).unwrap(), t);
            self.release(t);
        } else if kind == n.var_decl {
            self.var_decl(node);
        } else if kind == n.fun_decl {
            self.fun_decl(node);
        } else if kind == n.class_decl {
            self.class_decl(node);
        } else if kind == n.block {
            let m = self.mark();
            for s in statements(tree, node) {
                self.stmt(s);
            }
            self.release(m);
        } else if kind == n.if_stmt {
            let c = self.condition(tree.field(node, n.cond).unwrap());
            self.b.if_(c);
            self.stmt(tree.field(node, n.then).unwrap());
            if let Some(e) = tree.field(node, n.else_) {
                self.b.else_();
                self.stmt(e);
            }
            self.b.end_if();
        } else if kind == n.while_stmt {
            self.b.loop_();
            self.exit_unless(tree.field(node, n.cond).unwrap());
            self.stmt(tree.field(node, n.body).unwrap());
            self.b.end_loop();
        } else if kind == n.for_stmt {
            let m = self.mark();
            if let Some(init) = tree.field(node, n.init) {
                self.stmt(init);
            }
            self.b.loop_();
            if let Some(cond) = tree.field(node, n.cond) {
                self.exit_unless(cond);
            }
            self.stmt(tree.field(node, n.body).unwrap());
            if let Some(incr) = tree.field(node, n.incr) {
                let t = self.alloc();
                self.expr(incr, t);
                self.release(t);
            }
            self.b.end_loop();
            self.release(m);
        } else if kind == n.return_stmt {
            if self.st().kind == FnKind::Initializer {
                self.return_this();
                return;
            }
            let v = match tree.field(node, n.value) {
                Some(value) => {
                    let t = self.alloc();
                    self.expr(value, t);
                    let v = self.load(t);
                    self.release(t);
                    v
                }
                None => self.konst(NIL),
            };
            self.leave();
            self.b.ret(Some(v));
        }
    }

    /// Evaluates a condition and returns it as an `i1`.
    fn condition(&mut self, cond: NodeId) -> IrValue {
        let t = self.alloc();
        self.expr(cond, t);
        let v = self.load(t);
        self.release(t);
        self.truthy(v)
    }

    /// `if (!cond) break;`
    fn exit_unless(&mut self, cond: NodeId) {
        let c = self.condition(cond);
        let stop = self.b.not(c);
        self.b.if_(stop);
        self.b.break_();
        self.b.end_if();
    }

    fn var_decl(&mut self, node: NodeId) {
        let n = self.n;
        let init = self.tree.field(node, n.init);
        match self.a.defs[&node] {
            Place::Local(d) => {
                let s = self.alloc();
                self.st().slots.insert(d, s);
                let t = self.alloc();
                match init {
                    Some(e) => self.expr(e, t),
                    None => {
                        let nil = self.konst(NIL);
                        self.store(t, nil);
                    }
                }
                self.bind_local(d, t);
                self.release(t);
            }
            Place::Global(i) => {
                let t = self.alloc();
                match init {
                    Some(e) => self.expr(e, t),
                    None => {
                        let nil = self.konst(NIL);
                        self.store(t, nil);
                    }
                }
                let idx = self.konst(i as i64);
                let v = self.load(t);
                let define = self.rt.global_define;
                self.call_void(define, &[idx, v]);
                self.release(t);
            }
        }
    }

    fn fun_decl(&mut self, node: NodeId) {
        let id = self.a.fn_of[&node];
        match self.a.defs[&node] {
            Place::Local(d) => {
                let s = self.alloc();
                self.st().slots.insert(d, s);
                if self.captured(d) {
                    // The cell exists before the closure, so the function can capture itself.
                    let nil = self.konst(NIL);
                    let new = self.rt.cell_new;
                    let cell = self.call(new, &[nil]);
                    self.store(s, cell);
                    let t = self.alloc();
                    self.closure(id, t);
                    let cell = self.load(s);
                    let v = self.load(t);
                    let set = self.rt.cell_set;
                    self.call_void(set, &[cell, v]);
                    self.release(t);
                } else {
                    self.closure(id, s);
                }
            }
            Place::Global(i) => {
                let t = self.alloc();
                self.closure(id, t);
                let idx = self.konst(i as i64);
                let v = self.load(t);
                let define = self.rt.global_define;
                self.call_void(define, &[idx, v]);
                self.release(t);
            }
        }
    }

    fn class_decl(&mut self, node: NodeId) {
        let n = self.n;
        let tree = self.tree;
        let name_tok = tree.field(node, n.name).unwrap();
        let name = tree.text(name_tok).to_string();
        let place = self.a.defs[&node];
        if let Place::Local(d) = place {
            let s = self.alloc();
            self.st().slots.insert(d, s);
            let nil = self.konst(NIL);
            if self.captured(d) {
                let new = self.rt.cell_new;
                let cell = self.call(new, &[nil]);
                self.store(s, cell);
            } else {
                self.store(s, nil);
            }
        }
        let m = self.mark();
        let superclass = tree.field(node, n.superclass);
        let super_decl = self.a.super_decls.get(&node).copied();
        if let (Some(sup), Some(ds)) = (superclass, super_decl) {
            let s = self.alloc();
            self.st().slots.insert(ds, s);
            let t = self.alloc();
            self.read(self.a.uses[&sup], sup, t);
            let v = self.load(t);
            let line = self.line(sup);
            let check = self.rt.check_superclass;
            self.call_void(check, &[v, line]);
            self.bind_local(ds, t);
            self.release(t);
        }
        if let Place::Global(i) = place {
            let idx = self.konst(i as i64);
            let nil = self.konst(NIL);
            let define = self.rt.global_define;
            self.call_void(define, &[idx, nil]);
        }
        let k = self.alloc();
        let name_val = self.constant(&name);
        let new = self.rt.class_new;
        let class = self.call(new, &[name_val]);
        self.store(k, class);
        if let Some(ds) = super_decl {
            let t = self.alloc();
            let tok = superclass.unwrap();
            self.read(Place::Local(ds), tok, t);
            let class = self.load(k);
            let sup = self.load(t);
            let inherit = self.rt.class_inherit;
            self.call_void(inherit, &[class, sup]);
            self.release(t);
        }
        for method in tree.fields(node, n.methods) {
            let t = self.alloc();
            self.closure(self.a.fn_of[&method], t);
            let method_name = tree.text(tree.field(method, n.name).unwrap()).to_string();
            let class = self.load(k);
            let name_val = self.constant(&method_name);
            let closure = self.load(t);
            let add = self.rt.class_add_method;
            self.call_void(add, &[class, name_val, closure]);
            self.release(t);
        }
        match place {
            Place::Local(_) => self.write(place, name_tok, k),
            Place::Global(i) => {
                let idx = self.konst(i as i64);
                let v = self.load(k);
                let define = self.rt.global_define;
                self.call_void(define, &[idx, v]);
            }
        }
        self.release(m);
    }

    // ---------------------------------------------------------------- expressions

    /// Evaluates `node` into slot `dst`.
    fn expr(&mut self, node: NodeId, dst: u32) {
        let n = self.n;
        let tree = self.tree;
        let kind = tree.kind(node);
        if kind == n.primary {
            self.primary(node, dst);
        } else if kind == n.assign {
            let left = tree.field(node, n.left).unwrap();
            let right = tree.field(node, n.right).unwrap();
            if is_variable(tree, &n, left) {
                self.expr(right, dst);
                self.write(self.a.uses[&node], tree.child(left, 0), dst);
            } else {
                // Property assignment: the object is checked before the value is evaluated.
                let object = tree.field(left, n.operand).unwrap();
                let name_tok = tree.field(left, n.name).unwrap();
                let t = self.alloc();
                self.expr(object, t);
                let obj = self.load(t);
                let line = self.line(name_tok);
                let check = self.rt.check_instance;
                self.call_void(check, &[obj, line]);
                self.expr(right, dst);
                let obj = self.load(t);
                let name = self.constant(tree.text(name_tok));
                let value = self.load(dst);
                let line = self.line(name_tok);
                let set = self.rt.set_property;
                self.call(set, &[obj, name, value, line]);
                self.release(t);
            }
        } else if kind == n.logical {
            let op = tree.field(node, n.op).unwrap();
            self.expr(tree.field(node, n.left).unwrap(), dst);
            let v = self.load(dst);
            let c = self.truthy(v);
            let evaluate_right = if tree.kind(op) == n.kw_and { c } else { self.b.not(c) };
            self.b.if_(evaluate_right);
            self.expr(tree.field(node, n.right).unwrap(), dst);
            self.b.end_if();
        } else if kind == n.binary {
            let op = tree.field(node, n.op).unwrap();
            let t1 = self.alloc();
            self.expr(tree.field(node, n.left).unwrap(), t1);
            let t2 = self.alloc();
            self.expr(tree.field(node, n.right).unwrap(), t2);
            let a = self.load(t1);
            let b = self.load(t2);
            let rt = &self.rt;
            let (f, with_line) = match tree.text(op) {
                "+" => (rt.add, true),
                "-" => (rt.subtract, true),
                "*" => (rt.multiply, true),
                "/" => (rt.divide, true),
                "<" => (rt.less, true),
                "<=" => (rt.less_equal, true),
                ">" => (rt.greater, true),
                ">=" => (rt.greater_equal, true),
                "==" => (rt.equal, false),
                _ => (rt.not_equal, false),
            };
            let v = if with_line {
                let line = self.line(op);
                self.call(f, &[a, b, line])
            } else {
                self.call(f, &[a, b])
            };
            self.store(dst, v);
            self.release(t1);
        } else if kind == n.unary {
            let op = tree.field(node, n.op).unwrap();
            self.expr(tree.field(node, n.operand).unwrap(), dst);
            let v = self.load(dst);
            let r = if tree.text(op) == "-" {
                let line = self.line(op);
                let negate = self.rt.negate;
                self.call(negate, &[v, line])
            } else {
                let not = self.rt.not;
                self.call(not, &[v])
            };
            self.store(dst, r);
        } else if kind == n.call {
            self.call_expr(node, dst);
        } else if kind == n.get {
            let name_tok = tree.field(node, n.name).unwrap();
            self.expr(tree.field(node, n.operand).unwrap(), dst);
            let obj = self.load(dst);
            let name = self.constant(tree.text(name_tok));
            let line = self.line(name_tok);
            let get = self.rt.get_property;
            let v = self.call(get, &[obj, name, line]);
            self.store(dst, v);
        }
    }

    fn primary(&mut self, node: NodeId, dst: u32) {
        let n = self.n;
        let tree = self.tree;
        let first = tree.child(node, 0);
        let fk = tree.kind(first);
        if fk == n.number {
            let value: f64 = tree.text(first).parse().expect("the grammar only accepts valid numbers");
            let v = self.konst(value.to_bits() as i64);
            self.store(dst, v);
        } else if fk == n.string {
            let text = tree.text(first);
            let v = self.constant(&text[1..text.len() - 1]);
            self.store(dst, v);
        } else if fk == n.kw_true || fk == n.kw_false || fk == n.kw_nil {
            let bits = if fk == n.kw_true {
                TRUE
            } else if fk == n.kw_false {
                FALSE
            } else {
                NIL
            };
            let v = self.konst(bits);
            self.store(dst, v);
        } else if fk == n.identifier || fk == n.kw_this {
            self.read(self.a.uses[&node], first, dst);
        } else if fk == n.kw_super {
            let method_tok = tree.field(node, n.method).unwrap();
            let t1 = self.alloc();
            self.read(self.a.uses[&node], first, t1);
            let t2 = self.alloc();
            self.read(Place::Local(self.a.super_this[&node]), first, t2);
            let sup = self.load(t1);
            let this = self.load(t2);
            let name = self.constant(tree.text(method_tok));
            let line = self.line(method_tok);
            let get = self.rt.super_get;
            let v = self.call(get, &[sup, this, name, line]);
            self.store(dst, v);
            self.release(t1);
        } else {
            self.expr(tree.field(node, n.inner).unwrap(), dst);
        }
    }

    fn call_expr(&mut self, node: NodeId, dst: u32) {
        let n = self.n;
        let tree = self.tree;
        let args = match tree.field(node, n.args) {
            Some(a) => tree.fields(a, n.arg),
            None => Vec::new(),
        };
        // The window: [callee, receiver, args...] in consecutive slots.
        let w = self.alloc();
        self.alloc();
        for _ in &args {
            self.alloc();
        }
        self.expr(tree.field(node, n.operand).unwrap(), w);
        for (i, &arg) in args.iter().enumerate() {
            self.expr(arg, w + 2 + i as u32);
        }
        let paren = tree.child(node, tree.child_count(node) - 1);
        let window = self.slot_ptr(w);
        let argc = self.konst(args.len() as i64);
        let line = self.line(paren);
        let prepare = self.rt.call_prepare;
        let code = self.call(prepare, &[window, argc, line]);
        let result = self
            .b
            .call_indirect(code, &[Type::Ptr], Some(Type::I64), &[window])
            .unwrap();
        self.store(dst, result);
        self.release(w);
    }
}
