//! Lowering HIR to the core IR plus calls into the default runtime (`a0rt`).
//!
//! Storage: every local is an IR variable. Locals holding references or `any` are also rooted in a
//! slot of the function's frame on the runtime's shadow stack (stored in the `any` encoding, so the
//! collector scans every slot the same way), written on every assignment; locals captured by
//! closures live in heap boxes, rooted the same way. A local declared in a block has its slot only
//! while the block runs, so later blocks reuse the slot. A slot is cleared (set to zero) when its
//! scope ends or is left by `return`, `break` or `continue`, which keeps the shadow stack above its
//! top all zero and lets frames be pushed without clearing them. Temporaries are spilled to frame slots
//! only when something evaluated after them, or the operation itself, may allocate, and reads of
//! locals that are never assigned again are not spilled at all (their slot keeps them alive). The
//! collector never moves objects, so a slot only keeps a value alive; the code keeps using the
//! variable or SSA value.

use std::collections::{BTreeSet, HashMap};

use a0_diag::{Diagnostic, Diagnostics};
use a0_ir::{
    Builder, DebugKind, DebugType, FloatCC, FuncId as IrFunc, GlobalId as IrGlobal, IntCC, Opcode,
    Type, Value, Var,
};

use crate::builder::visit_children;
use crate::model::*;

const QNAN: i64 = 0x7ffc_0000_0000_0000;
const REF_TAG: i64 = 0xfffc_0000_0000_0000_u64 as i64;
const PTR_MASK: i64 = 0x0000_ffff_ffff_ffff;
const ANY_UNIT: i64 = QNAN | 1;
const ANY_FALSE: i64 = QNAN | 2;
const ANY_TRUE: i64 = QNAN | 3;
const CANONICAL_NAN: i64 = 0x7ff8_0000_0000_0000;
/// Where generated code finds an object's type: in the collector's header, before the payload, and
/// the type's family within its descriptor (see `a0rt.h`).
const OBJECT_TYPE_OFFSET: i64 = -8;
const TYPE_FAMILY_OFFSET: i64 = 24;
/// Where a type descriptor keeps its interface table: per interface of the module, the table of
/// the type's methods for it, or null (see `A0Type` in `a0rt.h`).
const TYPE_ITABLE_OFFSET: i64 = 32;

const WORD_RAW: u8 = 0;
const WORD_REF: u8 = 1;
const WORD_ANY: u8 = 2;
/// An array whose elements are bytes (`array<bool>`), not words (`A0_ELEM_BYTE` in `a0rt.h`).
const ELEM_BYTE: u8 = 3;
/// Elements of two and four bytes (`A0_ELEM_16`, `A0_ELEM_32`).
const ELEM_16: u8 = 4;
const ELEM_32: u8 = 5;
const KEY_RAW: i64 = 0;
const KEY_STRING: i64 = 1;
const KEY_BOOL: i64 = 2;

/// How to lower a module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LowerOptions {
    /// Run the HIR optimizer (`opt.rs`) first.
    pub optimize: bool,
}

impl Default for LowerOptions {
    /// Optimized, unless the environment variable `A0_OPT` is `0`.
    fn default() -> Self {
        LowerOptions {
            optimize: std::env::var("A0_OPT").map_or(true, |v| v != "0"),
        }
    }
}

/// Lowers a checked module to a core IR module with a `@main` entry point, optimized unless
/// `A0_OPT=0` (see [`LowerOptions`]).
pub fn lower(m: &HirModule) -> Result<a0_ir::Module, Diagnostics> {
    lower_with(m, LowerOptions::default())
}

pub fn lower_with(m: &HirModule, options: LowerOptions) -> Result<a0_ir::Module, Diagnostics> {
    let entry = crate::check::check_entry(m);
    if entry.has_errors() {
        return Err(entry);
    }
    // Generic items become one copy per use; lowering itself never sees a type parameter.
    let mut owned = crate::mono::monomorphize(m)?;
    if options.optimize {
        let mut o = owned.unwrap_or_else(|| m.clone());
        crate::opt::optimize(&mut o);
        owned = Some(o);
    }
    let m = owned.as_ref().unwrap_or(m);
    let mut l = Lowerer::new(m);
    l.run();
    match l.b.finish() {
        Ok(module) => Ok(module),
        Err(d) => {
            let mut diags: Diagnostics = Diagnostic::error(
                "hir.lower",
                "lowering produced invalid IR (a bug in a0-hir)",
            )
            .into();
            diags.extend(d);
            Err(diags)
        }
    }
}

/// Runtime functions used by generated code.
struct Runtime {
    type_new: IrFunc,
    gc_new: IrFunc,
    frame_push: IrFunc,
    frame_pop: IrFunc,
    add_root: IrFunc,
    panic_bytes: IrFunc,
    any_from_int: IrFunc,
    any_is_int: IrFunc,
    any_is_string: IrFunc,
    any_to_int: IrFunc,
    any_eq: IrFunc,
    any_cast_failed: IrFunc,
    string_new: IrFunc,
    string_concat: IrFunc,
    string_eq: IrFunc,
    string_compare: IrFunc,
    string_substr: IrFunc,
    string_byte_at: IrFunc,
    string_from_int: IrFunc,
    string_from_uint: IrFunc,
    string_from_f32: IrFunc,
    string_from_float: IrFunc,
    string_from_bool: IrFunc,
    string_print: IrFunc,
    string_eprint: IrFunc,
    string_panic: IrFunc,
    array_new: IrFunc,
    array_filled: IrFunc,
    object_new: IrFunc,
    object_has: IrFunc,
    object_get: IrFunc,
    object_set: IrFunc,
    object_names: IrFunc,
    object_find_cached: IrFunc,
    object_set_cached: IrFunc,
    array_get: IrFunc,
    array_set: IrFunc,
    array_push: IrFunc,
    array_pop: IrFunc,
    map_new: IrFunc,
    map_len: IrFunc,
    map_has: IrFunc,
    map_get: IrFunc,
    map_set: IrFunc,
    map_remove: IrFunc,
    map_keys: IrFunc,
    string_from_c: IrFunc,
    alloc: IrFunc,
    throw: IrFunc,
    uncaught: IrFunc,
    exit: IrFunc,
    clock: IrFunc,
}

impl Runtime {
    fn declare(b: &mut Builder) -> Runtime {
        use Type::{F64, I32, I64, Ptr};
        let mut f =
            |name: &str, params: &[Type], ret: Option<Type>| b.declare_extern(name, params, ret);
        Runtime {
            type_new: f("a0_type_new", &[Ptr, I64, I64, Ptr, Ptr], Some(Ptr)),
            gc_new: f("a0_gc_new", &[Ptr], Some(Ptr)),
            frame_push: f("a0_gc_frame_push", &[I64], Some(Ptr)),
            frame_pop: f("a0_gc_frame_pop", &[I64], None),
            add_root: f("a0_gc_add_root", &[Ptr, I64], None),
            panic_bytes: f("a0_panic_bytes", &[Ptr, I64], None),
            any_from_int: f("a0_any_from_int", &[I64], Some(I64)),
            any_is_int: f("a0_any_is_int", &[I64], Some(I64)),
            any_is_string: f("a0_any_is_string", &[I64], Some(I64)),
            any_to_int: f("a0_any_to_int", &[I64], Some(I64)),
            any_eq: f("a0_any_eq", &[I64, I64], Some(I64)),
            any_cast_failed: f("a0_any_cast_failed", &[I64, Ptr, I64], None),
            string_new: f("a0_string_new", &[Ptr, I64], Some(Ptr)),
            string_concat: f("a0_string_concat", &[Ptr, Ptr], Some(Ptr)),
            string_eq: f("a0_string_eq", &[Ptr, Ptr], Some(I64)),
            string_compare: f("a0_string_compare", &[Ptr, Ptr], Some(I64)),
            string_substr: f("a0_string_substr", &[Ptr, I64, I64], Some(Ptr)),
            string_byte_at: f("a0_string_byte_at", &[Ptr, I64], Some(I64)),
            string_from_int: f("a0_string_from_int", &[I64], Some(Ptr)),
            string_from_uint: f("a0_string_from_uint", &[I64], Some(Ptr)),
            string_from_f32: f("a0_string_from_f32", &[F64], Some(Ptr)),
            string_from_float: f("a0_string_from_float", &[F64], Some(Ptr)),
            string_from_bool: f("a0_string_from_bool", &[I64], Some(Ptr)),
            string_print: f("a0_string_print", &[Ptr], None),
            string_eprint: f("a0_string_eprint", &[Ptr], None),
            string_panic: f("a0_string_panic", &[Ptr], None),
            array_new: f("a0_array_new", &[I64, I64], Some(Ptr)),
            array_filled: f("a0_array_filled", &[I64, I64, I64], Some(Ptr)),
            object_new: f("a0_object_new", &[], Some(Ptr)),
            object_has: f("a0_object_has", &[Ptr, Ptr], Some(I64)),
            object_get: f("a0_object_get", &[Ptr, Ptr], Some(I64)),
            object_set: f("a0_object_set", &[Ptr, Ptr, I64], None),
            object_names: f("a0_object_names", &[Ptr], Some(Ptr)),
            object_find_cached: f("a0_object_find_cached", &[Ptr, Ptr, Ptr], Some(I64)),
            object_set_cached: f("a0_object_set_cached", &[Ptr, Ptr, I64, Ptr], None),
            array_get: f("a0_array_get", &[Ptr, I64], Some(I64)),
            array_set: f("a0_array_set", &[Ptr, I64, I64], None),
            array_push: f("a0_array_push", &[Ptr, I64], None),
            array_pop: f("a0_array_pop", &[Ptr], Some(I64)),
            map_new: f("a0_map_new", &[I64, I64], Some(Ptr)),
            map_len: f("a0_map_len", &[Ptr], Some(I64)),
            map_has: f("a0_map_has", &[Ptr, I64], Some(I64)),
            map_get: f("a0_map_get", &[Ptr, I64], Some(I64)),
            map_set: f("a0_map_set", &[Ptr, I64, I64], None),
            map_remove: f("a0_map_remove", &[Ptr, I64], Some(I64)),
            map_keys: f("a0_map_keys", &[Ptr], Some(Ptr)),
            string_from_c: f("a0_string_from_c", &[Ptr, I64], Some(Ptr)),
            alloc: f("a0_alloc", &[I64], Some(Ptr)),
            throw: f("a0_rt_throw", &[], None),
            uncaught: f("a0_rt_uncaught", &[I64], None),
            exit: f("a0_rt_exit", &[I32], None),
            clock: f("a0_clock", &[], Some(F64)),
        }
    }
}

/// Where a local lives.
#[derive(Clone, Copy)]
enum Storage {
    None,
    Var(Var),
    /// A reference or `any`, rooted by a frame slot.
    Rooted(Var, u32),
    /// A captured local: the pointer to its heap box, rooted by a frame slot.
    Boxed(Var, u32),
}

/// The result of lowering an expression.
#[derive(Clone, Copy)]
enum V {
    Val(Value),
    Unit,
    Never,
}

impl V {
    fn val(self) -> Value {
        match self {
            V::Val(v) => v,
            _ => panic!("expected a value"),
        }
    }
}

/// A runtime type descriptor to register at startup.
struct TypeDesc {
    global: IrGlobal,
    name: String,
    layout: Vec<u8>,
    family: Option<IrGlobal>,
}

struct FnState {
    body: BodyId,
    /// The shadow-stack frame, or `None` for a body with nothing to root (see `needs_frame`).
    frame: Option<Value>,
    slot_count: Value,
    next: u32,
    max: u32,
    storage: HashMap<LocalId, Storage>,
    gc_memo: HashMap<ExprId, bool>,
    /// The enclosing loops and `try`s, innermost last, for `break`, `continue` and exceptions.
    exits: Vec<Exit>,
}

/// An IR loop that code can leave: a HIR loop, or the loop a `try` body runs in, which a raised
/// exception leaves. Each holds the first slot of its body, cleared when it is left.
#[derive(Clone, Copy)]
enum Exit {
    Loop(u32),
    Try(u32),
}

struct Lowerer<'a> {
    m: &'a HirModule,
    b: Builder,
    rt: Runtime,
    funcs: Vec<IrFunc>,
    externs: Vec<IrFunc>,
    globals: Vec<Option<IrGlobal>>,
    struct_desc: Vec<IrGlobal>,
    enum_desc: Vec<(IrGlobal, Vec<IrGlobal>)>,
    lambda_funcs: HashMap<BodyId, IrFunc>,
    lambda_desc: HashMap<BodyId, IrGlobal>,
    wrappers: HashMap<FuncId, (IrFunc, IrGlobal)>,
    box_desc: [IrGlobal; 3],
    descs: Vec<TypeDesc>,
    strings: HashMap<String, IrGlobal>,
    string_list: Vec<(String, IrGlobal)>,
    type_names: HashMap<String, IrGlobal>,
    captured: Vec<bool>,
    captures: HashMap<BodyId, Vec<LocalId>>,
    /// Locals declared directly in a block, which get their storage when the block starts.
    scoped: Vec<bool>,
    /// Locals assigned after their declaration.
    reassigned: Vec<bool>,
    f: Option<FnState>,
    /// The IR inline site of each HIR one, made when first needed.
    inline_ids: HashMap<u32, a0_ir::InlineId>,
    /// The debug type of each HIR type, made when a local first needs it.
    debug_types: HashMap<TypeId, DebugType>,
    /// What unwrapping a null says, in the language's words.
    null_message: Option<(IrGlobal, usize)>,
    /// Per body: whether `main` can reach it (others are not lowered), and whether it may raise an
    /// exception into its caller (`analyze_raises`).
    reached: Vec<bool>,
    raises: Vec<bool>,
    /// Whether some closure the program makes may raise one.
    closures_raise: bool,
    /// The pending exception, if the program throws: a flag, and its value (an `any`).
    exception: Option<(IrGlobal, IrGlobal)>,
    /// The caches of the places that name an object's field (`A0FieldCache`, three words each),
    /// allocated together at startup: their number, and the global that points at them.
    field_caches: u32,
    field_cache_base: IrGlobal,
}

impl<'a> Lowerer<'a> {
    fn new(m: &'a HirModule) -> Lowerer<'a> {
        let mut b = Builder::new(m.name());
        let rt = Runtime::declare(&mut b);
        let box_desc = [
            b.add_global_int("hir.type.box.raw", Type::Ptr, 0),
            b.add_global_int("hir.type.box.ref", Type::Ptr, 0),
            b.add_global_int("hir.type.box.any", Type::Ptr, 0),
        ];
        let field_cache_base = b.add_global_int("hir.field_caches", Type::Ptr, 0);
        let descs = (0..3)
            .map(|k| TypeDesc {
                global: box_desc[k],
                name: "box".to_string(),
                layout: vec![k as u8],
                family: None,
            })
            .collect();
        Lowerer {
            m,
            b,
            rt,
            funcs: Vec::new(),
            externs: Vec::new(),
            globals: Vec::new(),
            struct_desc: Vec::new(),
            enum_desc: Vec::new(),
            lambda_funcs: HashMap::new(),
            lambda_desc: HashMap::new(),
            wrappers: HashMap::new(),
            box_desc,
            descs,
            strings: HashMap::new(),
            string_list: Vec::new(),
            field_caches: 0,
            field_cache_base,
            type_names: HashMap::new(),
            captured: Vec::new(),
            captures: HashMap::new(),
            scoped: Vec::new(),
            reassigned: Vec::new(),
            f: None,
            inline_ids: HashMap::new(),
            debug_types: HashMap::new(),
            null_message: None,
            reached: Vec::new(),
            raises: Vec::new(),
            closures_raise: false,
            exception: None,
        }
    }

    /// Attributes the instructions built next to the HIR expression `e`: its span, and the call it
    /// was inlined at, if it was, for stack traces.
    fn at(&mut self, e: ExprId) {
        self.b.set_span(self.m.expr(e).span);
        let site = self.m.inline_of.get(&e).copied();
        let site = site.map(|s| self.ir_site(s));
        self.b.set_inline(site);
    }

    fn ir_site(&mut self, site: u32) -> a0_ir::InlineId {
        if let Some(&id) = self.inline_ids.get(&site) {
            return id;
        }
        let s = self.m.inline_sites[site as usize].clone();
        let parent = s.parent.map(|p| self.ir_site(p));
        let callee = self.m.funcs[s.callee.index()].name.clone();
        let id = self.b.add_inline_site(&callee, s.call, parent);
        self.inline_ids.insert(site, id);
        id
    }

    // ---------------------------------------------------------------- types

    fn ir_type(&self, t: TypeId) -> Option<Type> {
        match self.m.kind(t) {
            TypeKind::Unit | TypeKind::Never => None,
            TypeKind::Bool => Some(Type::I1),
            TypeKind::Int | TypeKind::Any => Some(Type::I64),
            TypeKind::Float => Some(Type::F64),
            TypeKind::SizedInt(bits, _) => Some(int_type(*bits)),
            TypeKind::F32 => Some(Type::F32),
            _ => Some(Type::Ptr),
        }
    }

    /// The width and signedness of an integer type (`int` included).
    fn int_bits(&self, t: TypeId) -> Option<(u32, bool)> {
        crate::builder::int_bits(self.m, t)
    }

    fn word_kind(&self, t: TypeId) -> u8 {
        if self.m.is_reference(t) {
            WORD_REF
        } else if t == TypeId::ANY {
            WORD_ANY
        } else {
            WORD_RAW
        }
    }

    /// How an array stores elements of type `t`: numbers narrower than a word in as many bytes
    /// as they take, booleans as bytes, everything else as words.
    fn elem_kind(&self, t: TypeId) -> u8 {
        match self.m.kind(t) {
            TypeKind::Bool | TypeKind::SizedInt(8, _) => ELEM_BYTE,
            TypeKind::SizedInt(16, _) => ELEM_16,
            TypeKind::SizedInt(32, _) | TypeKind::F32 => ELEM_32,
            _ => self.word_kind(t),
        }
    }

    fn is_gc(&self, t: TypeId) -> bool {
        self.m.is_reference(t) || t == TypeId::ANY
    }

    fn signature(&self, body: BodyId, env: bool) -> (Vec<Type>, Option<Type>) {
        let mut params = Vec::new();
        if env {
            params.push(Type::Ptr);
        }
        for &p in self.m.body_params(body) {
            params.extend(self.ir_type(self.m.local_type(p)));
        }
        (params, self.ir_type(self.m.body_ret(body)))
    }

    fn fn_signature(&self, t: TypeId) -> (Vec<Type>, Option<Type>) {
        match self.m.kind(t) {
            TypeKind::Fn(ps, r) => {
                let mut params = vec![Type::Ptr];
                params.extend(ps.iter().filter_map(|&p| self.ir_type(p)));
                (params, self.ir_type(*r))
            }
            _ => (vec![Type::Ptr], None),
        }
    }

    // ---------------------------------------------------------------- setup

    fn run(&mut self) {
        self.analyze_captures();
        self.analyze_locals();
        self.analyze_raises();
        let m = self.m;
        if m.exprs.iter().any(|e| matches!(e.kind, ExprKind::Throw(_))) {
            let pending = self.b.add_global_int("hir.exception.pending", Type::I64, 0);
            let value = self.b.add_global_int("hir.exception.value", Type::I64, 0);
            self.exception = Some((pending, value));
        }
        for (i, s) in m.structs.iter().enumerate() {
            let g = self
                .b
                .add_global_int(&format!("hir.type.{}", s.name), Type::Ptr, 0);
            let layout = s.fields.iter().map(|&(_, t)| self.word_kind(t)).collect();
            self.descs.push(TypeDesc {
                global: g,
                name: s.name.clone(),
                layout,
                family: None,
            });
            self.struct_desc.push(g);
            let _ = i;
        }
        for e in &m.enums {
            let family = self
                .b
                .add_global_int(&format!("hir.type.{}", e.name), Type::Ptr, 0);
            self.descs.push(TypeDesc {
                global: family,
                name: e.name.clone(),
                layout: Vec::new(),
                family: None,
            });
            let mut variants = Vec::new();
            for v in &e.variants {
                let g =
                    self.b
                        .add_global_int(&format!("hir.type.{}.{}", e.name, v.name), Type::Ptr, 0);
                let mut layout = vec![WORD_RAW];
                layout.extend(v.fields.iter().map(|&t| self.word_kind(t)));
                self.descs.push(TypeDesc {
                    global: g,
                    name: format!("{}::{}", e.name, v.name),
                    layout,
                    family: Some(family),
                });
                variants.push(g);
            }
            self.enum_desc.push((family, variants));
        }
        for g in &m.globals {
            let ir = match self.m.kind(g.ty) {
                TypeKind::Unit | TypeKind::Never => None,
                TypeKind::Float => Some(self.b.add_global_float(
                    &format!("hir.global.{}", g.name),
                    Type::F64,
                    0.0,
                )),
                TypeKind::Any => Some(self.b.add_global_int(
                    &format!("hir.global.{}", g.name),
                    Type::I64,
                    ANY_UNIT,
                )),
                _ => {
                    let t = self.ir_type(g.ty).unwrap();
                    Some(
                        self.b
                            .add_global_int(&format!("hir.global.{}", g.name), t, 0),
                    )
                }
            };
            self.globals.push(ir);
        }
        for x in &m.externs {
            // Modules may declare the same C function (the builder checked their signatures
            // agree), and a runtime function may be one lowering declares itself.
            if let Some(f) = self.b.module().lookup_func(&x.name) {
                self.externs.push(f);
                continue;
            }
            // A C function gets a `bool` as a 64-bit 0 or 1 (`c_argument`), and integers narrower
            // than 32 bits as 32-bit ones, extended as C promotes them; it returns them in 32
            // bits too, which `c_result` narrows.
            let c_type = |me: &Self, t: TypeId| match me.m.kind(t) {
                TypeKind::SizedInt(8 | 16, _) => Some(Type::I32),
                _ => me.ir_type(t),
            };
            let params: Vec<Type> = x
                .params
                .iter()
                .filter_map(|&t| match (x.abi, t) {
                    (Abi::C, TypeId::BOOL) => Some(Type::I64),
                    (Abi::C, t) => c_type(self, t),
                    _ => self.ir_type(t),
                })
                .collect();
            let ret = match x.abi {
                Abi::C => c_type(self, x.ret),
                Abi::A0 => self.ir_type(x.ret),
            };
            let f = self.b.declare_extern(&x.name, &params, ret);
            self.externs.push(f);
        }
        for f in &m.funcs {
            if !self.reached[f.body.index()] {
                self.funcs.push(IrFunc::NONE);
                continue;
            }
            let (params, ret) = self.signature(f.body, false);
            let ir = self
                .b
                .declare_function(&format!("hir.{}", f.name), &params, ret);
            self.b.set_trace_name(ir, &f.name);
            self.funcs.push(ir);
        }
        // Closures: one IR function and one descriptor per closure body.
        let mut lambda_count: HashMap<FuncId, u32> = HashMap::new();
        for (i, body) in m.bodies.iter().enumerate() {
            if body.parent.is_none() || !self.reached[i] {
                continue;
            }
            let id = BodyId(i as u32);
            let n = lambda_count.entry(body.func).or_insert(0);
            let name = format!("hir.{}.closure{}", m.funcs[body.func.index()].name, *n);
            *n += 1;
            let (params, ret) = self.signature(id, true);
            let f = self.b.declare_function(&name, &params, ret);
            let trace_name = match &body.name {
                Some(name) => name.clone(),
                None => format!("{}::{{closure}}", m.funcs[body.func.index()].name),
            };
            self.b.set_trace_name(f, &trace_name);
            self.lambda_funcs.insert(id, f);
            let g = self.b.add_global_int(&format!("{name}.type"), Type::Ptr, 0);
            let mut layout = vec![WORD_RAW];
            layout.extend(std::iter::repeat_n(WORD_REF, self.captures[&id].len()));
            self.descs.push(TypeDesc {
                global: g,
                name: "closure".to_string(),
                layout,
                family: None,
            });
            self.lambda_desc.insert(id, g);
        }
        // Functions used as values get a wrapper with the closure calling convention.
        for e in &m.exprs {
            if let ExprKind::Func(f, _) = e.kind {
                if self.wrappers.contains_key(&f) || !self.reached[m.func_body(f).index()] {
                    continue;
                }
                let body = m.func_body(f);
                let (params, ret) = self.signature(body, true);
                let name = format!("hir.{}.as_closure", m.funcs[f.index()].name);
                let w = self.b.declare_function(&name, &params, ret);
                self.b.hide_from_traces(w);
                let g = self.b.add_global_int(&format!("{name}.type"), Type::Ptr, 0);
                self.descs.push(TypeDesc {
                    global: g,
                    name: "closure".to_string(),
                    layout: vec![WORD_RAW],
                    family: None,
                });
                self.wrappers.insert(f, (w, g));
            }
        }

        for i in 0..m.funcs.len() {
            let body = m.funcs[i].body;
            if self.reached[body.index()] {
                self.function(body, self.funcs[i], false);
            }
        }
        let lambdas: Vec<(BodyId, IrFunc)> =
            self.lambda_funcs.iter().map(|(&b, &f)| (b, f)).collect();
        let mut lambdas = lambdas;
        lambdas.sort_by_key(|(b, _)| *b);
        for (body, f) in lambdas {
            self.function(body, f, true);
        }
        let wrappers: Vec<(FuncId, IrFunc)> =
            self.wrappers.iter().map(|(&f, &(w, _))| (f, w)).collect();
        for (f, w) in wrappers {
            self.wrapper(f, w);
        }
        // An `any` can hold an object of any struct or enum, which a debugger shows by its
        // descriptor's name: describe them all, not only those a typed local needs.
        for (i, k) in m.types.iter().enumerate() {
            if let TypeKind::Struct(_, args) | TypeKind::Enum(_, args) = k
                && args.is_empty()
            {
                self.debug_type(TypeId(i as u32));
            }
        }
        self.entry();
    }

    /// Finds the locals each closure captures and marks them captured.
    fn analyze_captures(&mut self) {
        let m = self.m;
        self.captured = vec![false; m.locals.len()];
        // Free locals of each body, innermost bodies first (children are created after parents).
        for i in (0..m.bodies.len()).rev() {
            let id = BodyId(i as u32);
            let mut free = BTreeSet::new();
            if let Some(root) = m.body_root(id) {
                self.collect_free(root, id, &mut free);
            }
            let list: Vec<LocalId> = free.into_iter().collect();
            for &l in &list {
                self.captured[l.index()] = true;
            }
            self.captures.insert(id, list);
        }
    }

    /// Finds the locals declared directly in a block and those assigned after their declaration.
    fn analyze_locals(&mut self) {
        let m = self.m;
        self.scoped = vec![false; m.locals.len()];
        self.reassigned = vec![false; m.locals.len()];
        let mut stack: Vec<ExprId> = m.bodies.iter().filter_map(|b| b.root).collect();
        while let Some(e) = stack.pop() {
            match &m.expr(e).kind {
                ExprKind::Block(items, _) => {
                    for &item in items {
                        if let ExprKind::Let(l, _) = m.expr(item).kind {
                            self.scoped[l.index()] = true;
                        }
                    }
                }
                ExprKind::SetLocal(l, _) => self.reassigned[l.index()] = true,
                ExprKind::IfLet(l, ..) | ExprKind::Try(_, l, _) => self.scoped[l.index()] = true,
                _ => {}
            }
            visit_children(m, e, &mut |c| stack.push(c));
        }
    }

    fn collect_free(&self, e: ExprId, body: BodyId, free: &mut BTreeSet<LocalId>) {
        let m = self.m;
        match &m.expr(e).kind {
            ExprKind::Local(l) | ExprKind::SetLocal(l, _) if m.locals[l.index()].body != body => {
                free.insert(*l);
            }
            ExprKind::Lambda(inner) => {
                for &l in &self.captures[inner] {
                    if m.locals[l.index()].body != body {
                        free.insert(l);
                    }
                }
            }
            _ => {}
        }
        visit_children(m, e, &mut |c| self.collect_free(c, body, free));
    }

    fn string_global(&mut self, s: &str) -> IrGlobal {
        if let Some(&g) = self.strings.get(s) {
            return g;
        }
        let g = self
            .b
            .add_global_int(&format!("hir.str.{}", self.string_list.len()), Type::Ptr, 0);
        self.strings.insert(s.to_string(), g);
        self.string_list.push((s.to_string(), g));
        g
    }

    /// The type as messages write it (`` `int` ``), for runtime errors.
    fn type_name_data(&mut self, t: TypeId) -> (IrGlobal, usize) {
        let name = self.m.quote_type(t);
        if let Some(&g) = self.type_names.get(&name) {
            return (g, name.len());
        }
        let g = self.b.add_data(
            &format!("hir.typename.{}", self.type_names.len()),
            name.as_bytes(),
        );
        self.type_names.insert(name.clone(), g);
        (g, name.len())
    }

    /// `main`: registers descriptors, roots and string constants, then runs the HIR `main`.
    fn entry(&mut self) {
        let main = self.b.declare_function("main", &[], Some(Type::I32));
        self.b.hide_from_traces(main);
        self.b.set_span(None);
        self.b.set_inline(None);
        let descs: Vec<(IrGlobal, IrGlobal, IrGlobal, usize, usize, Option<IrGlobal>)> = self
            .descs
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let name = self
                    .b
                    .add_data(&format!("hir.typedesc.{i}.name"), d.name.as_bytes());
                let layout = self
                    .b
                    .add_data(&format!("hir.typedesc.{i}.layout"), &d.layout);
                (
                    d.global,
                    name,
                    layout,
                    d.name.len(),
                    d.layout.len(),
                    d.family,
                )
            })
            .collect();
        let strings: Vec<(IrGlobal, IrGlobal, usize)> = self
            .string_list
            .clone()
            .into_iter()
            .enumerate()
            .map(|(i, (s, g))| {
                (
                    g,
                    self.b.add_data(&format!("hir.str.{i}.bytes"), s.as_bytes()),
                    s.len(),
                )
            })
            .collect();
        self.b.begin_function(main);
        for (global, name, layout, name_len, words, family) in descs {
            let np = self.b.global_addr(name);
            let nl = self.b.iconst(Type::I64, name_len as i64);
            let wc = self.b.iconst(Type::I64, words as i64);
            let lp = self.b.global_addr(layout);
            let fam = match family {
                Some(f) => {
                    let a = self.b.global_addr(f);
                    self.b.load(Type::Ptr, a)
                }
                None => self.b.null(),
            };
            let d = self
                .b
                .call(self.rt.type_new, &[np, nl, wc, lp, fam])
                .unwrap();
            let a = self.b.global_addr(global);
            self.b.store(d, a);
        }
        self.interface_tables();
        for (i, g) in self.globals.clone().into_iter().enumerate() {
            let Some(g) = g else { continue };
            let t = self.m.globals[i].ty;
            if self.is_gc(t) {
                let a = self.b.global_addr(g);
                let k = self.b.iconst(Type::I64, self.word_kind(t) as i64);
                self.b.call(self.rt.add_root, &[a, k]);
            }
        }
        if let Some((_, value)) = self.exception {
            let a = self.b.global_addr(value);
            let k = self.b.iconst(Type::I64, WORD_ANY as i64);
            self.b.call(self.rt.add_root, &[a, k]);
        }
        if self.field_caches > 0 {
            let size = self.b.iconst(Type::I64, 24 * self.field_caches as i64);
            let caches = self.b.call(self.rt.alloc, &[size]).unwrap();
            let a = self.b.global_addr(self.field_cache_base);
            self.b.store(caches, a);
        }
        for (g, bytes, len) in strings {
            let a = self.b.global_addr(g);
            let k = self.b.iconst(Type::I64, WORD_REF as i64);
            self.b.call(self.rt.add_root, &[a, k]);
            let p = self.b.global_addr(bytes);
            let n = self.b.iconst(Type::I64, len as i64);
            let s = self.b.call(self.rt.string_new, &[p, n]).unwrap();
            self.b.store(s, a);
        }
        let hir_main = match self.m.lookup_value("main") {
            Some(Item::Func(f)) => f,
            _ => unreachable!("check_entry guarantees main"),
        };
        let r = self.b.call(self.funcs[hir_main.index()], &[]);
        if let Some((pending, value)) = self.exception
            && self.raises[self.m.func_body(hir_main).index()]
        {
            let a = self.b.global_addr(pending);
            let p = self.b.load(Type::I64, a);
            let zero = self.b.iconst(Type::I64, 0);
            let raised = self.b.icmp(IntCC::Ne, p, zero);
            self.b.if_(raised);
            let va = self.b.global_addr(value);
            let w = self.b.load(Type::I64, va);
            self.b.call(self.rt.uncaught, &[w]);
            self.b.unreachable();
            self.b.end_if();
        }
        let code = match r {
            Some(v) => self.b.cast(Opcode::Trunc, v, Type::I32),
            None => self.b.iconst(Type::I32, 0),
        };
        self.b.ret(Some(code));
        self.b.end_function();
    }

    /// Gives every type that implements interfaces its interface table: per interface of the
    /// module, the table of the type's methods for it (null for the others). An enum's tables go
    /// on each variant's descriptor, which is what its objects carry.
    fn interface_tables(&mut self) {
        let m = self.m;
        if m.vtables.is_empty() {
            return;
        }
        let mut itables: HashMap<IrGlobal, Value> = HashMap::new();
        for (owner, i, methods) in m.vtables.clone() {
            let descs = match owner {
                Owner::Struct(s) => vec![self.struct_desc[s.index()]],
                Owner::Enum(e) => self.enum_desc[e.index()].1.clone(),
                Owner::Value(_) => continue,
            };
            let size = self.b.iconst(Type::I64, 8 * methods.len() as i64);
            let vtable = self.b.call(self.rt.alloc, &[size]).unwrap();
            for (k, f) in methods.iter().enumerate() {
                let code = self.b.func_addr(self.funcs[f.index()]);
                let off = self.b.iconst(Type::I64, 8 * k as i64);
                let at = self.b.ptradd(vtable, off);
                self.b.store(code, at);
            }
            for desc in descs {
                let itable = match itables.get(&desc) {
                    Some(&t) => t,
                    None => {
                        let size = self.b.iconst(Type::I64, 8 * m.interfaces.len() as i64);
                        let t = self.b.call(self.rt.alloc, &[size]).unwrap();
                        let a = self.b.global_addr(desc);
                        let d = self.b.load(Type::Ptr, a);
                        let off = self.b.iconst(Type::I64, TYPE_ITABLE_OFFSET);
                        let at = self.b.ptradd(d, off);
                        self.b.store(t, at);
                        itables.insert(desc, t);
                        t
                    }
                };
                let off = self.b.iconst(Type::I64, 8 * i.index() as i64);
                let at = self.b.ptradd(itable, off);
                self.b.store(vtable, at);
            }
        }
    }

    /// The table of methods for interface `i` of `object`'s type.
    fn vtable(&mut self, object: Value, i: InterfaceId) -> Value {
        let off = self.b.iconst(Type::I64, OBJECT_TYPE_OFFSET);
        let at = self.b.ptradd(object, off);
        let ty = self.b.load(Type::Ptr, at);
        let off = self.b.iconst(Type::I64, TYPE_ITABLE_OFFSET);
        let at = self.b.ptradd(ty, off);
        let itable = self.b.load(Type::Ptr, at);
        let off = self.b.iconst(Type::I64, 8 * i.index() as i64);
        let at = self.b.ptradd(itable, off);
        self.b.load(Type::Ptr, at)
    }

    /// `f` called through the closure convention (the environment is ignored).
    fn wrapper(&mut self, f: FuncId, w: IrFunc) {
        let body = self.m.func_body(f);
        let n = self.signature(body, false).0.len();
        self.b.begin_function(w);
        let args: Vec<Value> = (0..n).map(|i| self.b.param(i as u32 + 1)).collect();
        let r = self.b.call(self.funcs[f.index()], &args);
        self.b.ret(r);
        self.b.end_function();
    }

    // ---------------------------------------------------------------- frames and slots

    fn st(&mut self) -> &mut FnState {
        self.f.as_mut().unwrap()
    }

    fn alloc_slot(&mut self) -> u32 {
        let st = self.st();
        let s = st.next;
        st.next += 1;
        st.max = st.max.max(st.next);
        s
    }

    fn mark(&mut self) -> u32 {
        self.st().next
    }

    /// Frees the slots allocated since `mark`, clearing them unless control never gets here.
    fn release(&mut self, mark: u32) {
        let next = self.st().next;
        if !self.b.is_terminated() {
            self.clear_slots(mark, next);
        }
        self.st().next = mark;
    }

    fn clear_slots(&mut self, from: u32, to: u32) {
        if from >= to {
            return;
        }
        let zero = self.b.iconst(Type::I64, 0);
        for s in from..to {
            let a = self.slot_addr(s);
            self.b.store(zero, a);
        }
    }

    fn slot_addr(&mut self, s: u32) -> Value {
        let frame = self.st().frame.expect("a body with slots has a frame");
        let off = self.b.iconst(Type::I64, 8 * s as i64);
        self.b.ptradd(frame, off)
    }

    fn encode(&mut self, v: Value, t: TypeId) -> Value {
        if t == TypeId::ANY {
            return v;
        }
        let w = self.b.cast(Opcode::Ptrtoint, v, Type::I64);
        let tag = self.b.iconst(Type::I64, REF_TAG);
        self.b.or(w, tag)
    }

    fn decode(&mut self, w: Value, t: TypeId) -> Value {
        if t == TypeId::ANY {
            return w;
        }
        let mask = self.b.iconst(Type::I64, PTR_MASK);
        let p = self.b.and(w, mask);
        self.b.cast(Opcode::Inttoptr, p, Type::Ptr)
    }

    fn store_slot(&mut self, s: u32, v: Value, t: TypeId) {
        let w = self.encode(v, t);
        let a = self.slot_addr(s);
        self.b.store(w, a);
    }

    /// Keeps a GC value alive in a fresh temporary slot until the caller releases it.
    fn spill(&mut self, v: Value, t: TypeId) {
        let s = self.alloc_slot();
        self.store_slot(s, v, t);
    }

    /// Spills the value of `e` if it is a GC value that nothing else keeps alive: a local that is
    /// never assigned again stays in its own slot.
    fn spill_operand(&mut self, e: ExprId, v: V) {
        let t = self.m.expr_type(e);
        let V::Val(val) = v else { return };
        if !self.is_gc(t) {
            return;
        }
        if let ExprKind::Local(l) = self.m.expr(e).kind
            && !self.reassigned[l.index()]
            && matches!(self.st().storage[&l], Storage::Rooted(..))
        {
            return;
        }
        self.spill(val, t);
    }

    // ---------------------------------------------------------------- words

    /// A value as a 64-bit word for object fields, array elements and map entries.
    fn word_encode(&mut self, v: Value, t: TypeId) -> Value {
        match self.m.kind(t) {
            TypeKind::Float => self.b.cast(Opcode::Bitcast, v, Type::I64),
            // Extended by their sign, so that a word reads as the number.
            TypeKind::SizedInt(64, _) => v,
            TypeKind::SizedInt(_, true) => self.b.cast(Opcode::Sext, v, Type::I64),
            TypeKind::SizedInt(_, false) => self.b.cast(Opcode::Zext, v, Type::I64),
            // Its 32 bits, as in an array of them.
            TypeKind::F32 => {
                let bits = self.b.cast(Opcode::Bitcast, v, Type::I32);
                self.b.cast(Opcode::Zext, bits, Type::I64)
            }
            TypeKind::Bool => self.b.cast(Opcode::Zext, v, Type::I64),
            TypeKind::Int | TypeKind::Any => v,
            _ => self.b.cast(Opcode::Ptrtoint, v, Type::I64),
        }
    }

    fn word_decode(&mut self, w: Value, t: TypeId) -> V {
        V::Val(match self.m.kind(t) {
            TypeKind::Unit => return V::Unit,
            TypeKind::Never => return V::Never,
            TypeKind::Float => self.b.cast(Opcode::Bitcast, w, Type::F64),
            TypeKind::SizedInt(64, _) => w,
            TypeKind::SizedInt(bits, _) => self.b.cast(Opcode::Trunc, w, int_type(*bits)),
            TypeKind::F32 => {
                let bits = self.b.cast(Opcode::Trunc, w, Type::I32);
                self.b.cast(Opcode::Bitcast, bits, Type::F32)
            }
            TypeKind::Bool => {
                let zero = self.b.iconst(Type::I64, 0);
                self.b.icmp(IntCC::Ne, w, zero)
            }
            TypeKind::Int | TypeKind::Any => w,
            _ => self.b.cast(Opcode::Inttoptr, w, Type::Ptr),
        })
    }

    fn word_of(&mut self, v: V, t: TypeId) -> Value {
        match v {
            V::Val(v) => self.word_encode(v, t),
            _ => self.b.iconst(Type::I64, 0),
        }
    }

    fn null_check(&mut self, p: Value) {
        let null = self.b.null();
        let is_null = self.b.icmp(IntCC::Eq, p, null);
        self.b.if_(is_null);
        let (message, len) = match self.null_message {
            Some(m) => m,
            None => {
                let text = format!(
                    "unwrapped a {} value with {}",
                    self.m.term(Term::Null),
                    self.m.term(Term::Unwrap)
                );
                let g = self.b.add_data("hir.null_message", text.as_bytes());
                self.null_message = Some((g, text.len()));
                (g, text.len())
            }
        };
        let mp = self.b.global_addr(message);
        let ml = self.b.iconst(Type::I64, len as i64);
        self.b.call(self.rt.panic_bytes, &[mp, ml]);
        self.b.unreachable();
        self.b.end_if();
    }

    /// What a C function gets for a value of type `t`: a string's bytes, an array's elements, or
    /// null for null (see `Abi::C`).
    fn c_argument(&mut self, v: Value, t: TypeId) -> Value {
        if t == TypeId::BOOL {
            return self.b.cast(Opcode::Zext, v, Type::I64);
        }
        match self.m.kind(t) {
            TypeKind::SizedInt(8 | 16, true) => return self.b.cast(Opcode::Sext, v, Type::I32),
            TypeKind::SizedInt(8 | 16, false) => return self.b.cast(Opcode::Zext, v, Type::I32),
            _ => {}
        }
        let inner = self.m.non_null(t);
        let pointer = |me: &mut Self, v: Value| match me.m.kind(inner) {
            // `A0String`: the length and the hash, then the bytes, NUL-terminated.
            TypeKind::String => {
                let off = me.b.iconst(Type::I64, 16);
                me.b.ptradd(v, off)
            }
            // `A0Array`: the length, the capacity, then the pointer to the elements.
            _ => {
                let off = me.b.iconst(Type::I64, 16);
                let a = me.b.ptradd(v, off);
                me.b.load(Type::Ptr, a)
            }
        };
        if !matches!(self.m.kind(inner), TypeKind::String | TypeKind::Array(_)) {
            return v;
        }
        if inner == t {
            return pointer(self, v);
        }
        let out = self.b.declare_var("", Type::Ptr);
        let null = self.b.null();
        let is_null = self.b.icmp(IntCC::Eq, v, null);
        self.b.if_(is_null);
        self.b.set(out, null);
        self.b.else_();
        let p = pointer(self, v);
        self.b.set(out, p);
        self.b.end_if();
        self.b.get(out)
    }

    // Element access is inline: a bounds check, then a load or a store. Out of bounds, the runtime
    // function does the access instead, which fails with the message and the stack trace.

    /// Whether `index` is outside `0..len` of the string or array `object` (whose first word is its
    /// length): one unsigned comparison.
    fn out_of_bounds(&mut self, object: Value, index: Value) -> Value {
        let len = self.load_word(object, 0);
        self.b.icmp(IntCC::Uge, index, len)
    }

    /// `byte_at(s, i)`: the byte, after the length and hash words (`A0String`).
    fn string_byte(&mut self, s: Value, i: Value) -> Value {
        let result = self.b.declare_var("byte", Type::I64);
        let bad = self.out_of_bounds(s, i);
        self.b.if_(bad);
        let r = self.b.call(self.rt.string_byte_at, &[s, i]).unwrap();
        self.b.set(result, r);
        self.b.else_();
        let header = self.b.iconst(Type::I64, 16);
        let bytes = self.b.ptradd(s, header);
        let at = self.b.ptradd(bytes, i);
        let byte = self.b.load(Type::I8, at);
        let byte = self.b.cast(Opcode::Zext, byte, Type::I64);
        self.b.set(result, byte);
        self.b.end_if();
        self.b.get(result)
    }

    /// The address of element `i` of an array (`A0Array`: length, capacity, then the pointer to
    /// the elements), whose elements are bytes if `bytes`, words otherwise.
    fn element_address(&mut self, a: Value, i: Value, size: u32) -> Value {
        let data = self.load_word(a, 2);
        let data = self.b.cast(Opcode::Inttoptr, data, Type::Ptr);
        if size == 1 {
            return self.b.ptradd(data, i);
        }
        let size = self.b.iconst(Type::I64, size as i64);
        let offset = self.b.imul(i, size);
        self.b.ptradd(data, offset)
    }

    /// The size of an array's elements of type `elem`, and the IR type of each in memory.
    fn elem_layout(&self, elem: TypeId) -> (u32, Type) {
        match self.elem_kind(elem) {
            ELEM_BYTE => (1, Type::I8),
            ELEM_16 => (2, Type::I16),
            ELEM_32 => (4, Type::I32),
            _ => (8, Type::I64),
        }
    }

    /// `a[i]`, as a word, for an array of `elem`s.
    fn array_element(&mut self, a: Value, i: Value, elem: TypeId) -> Value {
        let (size, mem) = self.elem_layout(elem);
        let result = self.b.declare_var("element", Type::I64);
        let bad = self.out_of_bounds(a, i);
        self.b.if_(bad);
        let r = self.b.call(self.rt.array_get, &[a, i]).unwrap();
        self.b.set(result, r);
        self.b.else_();
        let at = self.element_address(a, i, size);
        let w = if size < 8 {
            let narrow = self.b.load(mem, at);
            self.b.cast(Opcode::Zext, narrow, Type::I64)
        } else {
            self.b.load(Type::I64, at)
        };
        self.b.set(result, w);
        self.b.end_if();
        self.b.get(result)
    }

    /// `a[i] = w`, for an array of `elem`s.
    fn set_array_element(&mut self, a: Value, i: Value, w: Value, elem: TypeId) {
        let (size, mem) = self.elem_layout(elem);
        let bad = self.out_of_bounds(a, i);
        self.b.if_(bad);
        self.b.call(self.rt.array_set, &[a, i, w]);
        self.b.else_();
        let at = self.element_address(a, i, size);
        if size < 8 {
            let narrow = self.b.cast(Opcode::Trunc, w, mem);
            self.b.store(narrow, at);
        } else {
            self.b.store(w, at);
        }
        self.b.end_if();
    }

    /// A new place's field cache (`A0FieldCache`), zero until the runtime fills it.
    fn field_cache(&mut self) -> Value {
        let k = self.field_caches;
        self.field_caches += 1;
        let base = self.b.global_addr(self.field_cache_base);
        let base = self.b.load(Type::Ptr, base);
        let off = self.b.iconst(Type::I64, 24 * k as i64);
        self.b.ptradd(base, off)
    }

    /// The slot of field `name` in the dynamic object `o` (-1 if it has none): from the cache when
    /// `o` has the shape the cache saw last, else from the runtime, which fills the cache.
    fn cached_field(&mut self, o: Value, name: Value, cache: Value) -> Value {
        let result = self.b.declare_var("slot", Type::I64);
        let shape = self.load_word(o, 0);
        let cached = self.load_word(cache, 0);
        let hit = self.b.icmp(IntCC::Eq, shape, cached);
        self.b.if_(hit);
        let index = self.load_word(cache, 1);
        self.b.set(result, index);
        self.b.else_();
        let index = self
            .b
            .call(self.rt.object_find_cached, &[o, name, cache])
            .unwrap();
        self.b.set(result, index);
        self.b.end_if();
        self.b.get(result)
    }

    /// The address of slot `index` of the dynamic object `o` (`A0Object`: shape, capacity, then
    /// the pointer to the slots).
    fn slot_address(&mut self, o: Value, index: Value) -> Value {
        let slots = self.load_word(o, 2);
        let slots = self.b.cast(Opcode::Inttoptr, slots, Type::Ptr);
        let eight = self.b.iconst(Type::I64, 8);
        let offset = self.b.imul(index, eight);
        self.b.ptradd(slots, offset)
    }

    fn load_word(&mut self, base: Value, index: u32) -> Value {
        let off = self.b.iconst(Type::I64, 8 * index as i64);
        let a = self.b.ptradd(base, off);
        self.b.load(Type::I64, a)
    }

    fn store_word(&mut self, base: Value, index: u32, w: Value) {
        let off = self.b.iconst(Type::I64, 8 * index as i64);
        let a = self.b.ptradd(base, off);
        self.b.store(w, a);
    }

    fn new_object(&mut self, desc: IrGlobal) -> Value {
        let a = self.b.global_addr(desc);
        let d = self.b.load(Type::Ptr, a);
        self.b.call(self.rt.gc_new, &[d]).unwrap()
    }

    fn box_desc(&self, t: TypeId) -> IrGlobal {
        self.box_desc[self.word_kind(t) as usize]
    }

    // ---------------------------------------------------------------- functions

    fn function(&mut self, body: BodyId, ir: IrFunc, closure: bool) {
        let m = self.m;
        self.b.begin_function(ir);
        // The prologue and epilogue belong to the function itself.
        let decl = m.funcs[m.bodies[body.index()].func.index()].span;
        self.b.set_span(decl);
        self.b.set_inline(None);
        let slot_count = self.b.iconst(Type::I64, 0);
        let frame = if self.needs_frame(body, closure) {
            Some(self.b.call(self.rt.frame_push, &[slot_count]).unwrap())
        } else {
            None
        };
        self.f = Some(FnState {
            body,
            frame,
            slot_count,
            next: 0,
            max: 0,
            storage: HashMap::new(),
            gc_memo: HashMap::new(),
            exits: Vec::new(),
        });

        // Storage for this body's own locals, except those of blocks, which get theirs later.
        for (i, l) in m.locals.iter().enumerate() {
            if l.body == body && !self.scoped[i] {
                self.declare_local(LocalId(i as u32));
            }
        }
        // A closure copies its captured boxes out of its environment.
        let mut param_index = 0u32;
        if closure {
            let env = self.b.param(0);
            param_index = 1;
            for (k, &l) in self.captures[&body].clone().iter().enumerate() {
                let s = self.alloc_slot();
                let w = self.load_word(env, k as u32 + 1);
                let addr = self.slot_addr(s);
                let tag = self.b.iconst(Type::I64, REF_TAG);
                let tagged = self.b.or(w, tag);
                self.b.store(tagged, addr);
                let var = self.b.declare_var(&m.locals[l.index()].name, Type::Ptr);
                self.describe_local(var, l, true);
                let bx = self.b.cast(Opcode::Inttoptr, w, Type::Ptr);
                self.b.set(var, bx);
                self.st().storage.insert(l, Storage::Boxed(var, s));
            }
        }
        // Parameters: root the GC ones first, then create boxes for captured ones (which allocates).
        let params = m.body_params(body).to_vec();
        let mut values = Vec::new();
        for &p in &params {
            let t = m.local_type(p);
            let v = if self.ir_type(t).is_some() {
                let v = self.b.param(param_index);
                param_index += 1;
                Some(v)
            } else {
                None
            };
            values.push(v);
            match (self.st().storage[&p], v) {
                (Storage::Var(var), Some(v)) => self.b.set(var, v),
                (Storage::Rooted(var, s), Some(v)) => {
                    self.b.set(var, v);
                    self.store_slot(s, v, t);
                }
                (Storage::Boxed(..), Some(v)) if self.is_gc(t) => self.spill(v, t),
                _ => {}
            }
        }
        for (&p, v) in params.iter().zip(values) {
            if let Storage::Boxed(var, s) = self.st().storage[&p] {
                let t = m.local_type(p);
                let bx = self.new_object(self.box_desc(t));
                if let Some(v) = v {
                    let w = self.word_encode(v, t);
                    self.store_word(bx, 0, w);
                }
                self.b.set(var, bx);
                self.store_slot(s, bx, TypeId::STRING);
            }
        }

        let ret = m.body_ret(body);
        let result = match m.body_root(body) {
            Some(root) => self.expr(root),
            None => V::Never,
        };
        if !matches!(result, V::Never) && !self.b.is_terminated() {
            self.pop_frame();
            match (self.ir_type(ret), result) {
                (Some(_), V::Val(v)) => self.b.ret(Some(v)),
                (None, _) => self.b.ret(None),
                (Some(_), _) => self.b.unreachable(),
            }
        } else if !self.b.is_terminated() {
            self.b.unreachable();
        }
        let st = self.f.take().unwrap();
        self.b.patch_iconst(st.slot_count, st.max as i64);
        self.b.end_function();
    }

    /// Creates the storage of a local: a variable, and a frame slot if it needs rooting.
    fn declare_local(&mut self, l: LocalId) {
        let data = &self.m.locals[l.index()];
        let storage = match self.ir_type(data.ty) {
            None => Storage::None,
            Some(_) if self.captured[l.index()] => {
                let var = self.b.declare_var(&data.name, Type::Ptr);
                self.describe_local(var, l, true);
                let null = self.b.null();
                self.b.set(var, null);
                Storage::Boxed(var, self.alloc_slot())
            }
            Some(t) if self.is_gc(data.ty) => {
                let var = self.b.declare_var(&data.name, t);
                self.describe_local(var, l, false);
                let zero = if t == Type::Ptr {
                    self.b.null()
                } else {
                    self.b.iconst(t, 0)
                };
                self.b.set(var, zero);
                Storage::Rooted(var, self.alloc_slot())
            }
            Some(t) => {
                let var = self.b.declare_var(&data.name, t);
                self.describe_local(var, l, false);
                Storage::Var(var)
            }
        };
        self.st().storage.insert(l, storage);
    }

    /// Tells debuggers that `var` holds the local `l` (in a box if `boxed`).
    fn describe_local(&mut self, var: Var, l: LocalId, boxed: bool) {
        let data = &self.m.locals[l.index()];
        let (name, ty, span) = (data.source_name.clone(), data.ty, data.span);
        let mut t = self.debug_type(ty);
        if boxed {
            t = self.b.debug_type(DebugKind::Boxed, "box", &[t]);
        }
        self.b.set_var_debug(var, &name, t, span);
    }

    /// How a debugger shows values of type `t`: the language's name for the type, and how to read
    /// its words (`a0_ir::DebugKind`).
    fn debug_type(&mut self, t: TypeId) -> DebugType {
        if let Some(&d) = self.debug_types.get(&t) {
            return d;
        }
        let m = self.m;
        let name = m.display_type(t);
        let simple = |s: &mut Self, kind: DebugKind, elems: &[TypeId]| {
            let elems: Vec<DebugType> = elems.iter().map(|&e| s.debug_type(e)).collect();
            s.b.debug_type(kind, &name, &elems)
        };
        let d = match m.kind(t) {
            TypeKind::Int | TypeKind::SizedInt(_, true) => simple(self, DebugKind::Int, &[]),
            TypeKind::SizedInt(_, false) => simple(self, DebugKind::UInt, &[]),
            TypeKind::Float => simple(self, DebugKind::Float, &[]),
            TypeKind::F32 => simple(self, DebugKind::Float32, &[]),
            TypeKind::Bool => simple(self, DebugKind::Bool, &[]),
            TypeKind::String => simple(self, DebugKind::String, &[]),
            TypeKind::Any => simple(self, DebugKind::Any, &[]),
            TypeKind::Array(e) => simple(self, DebugKind::Array, &[*e]),
            TypeKind::Map(k, v) => simple(self, DebugKind::Map, &[*k, *v]),
            TypeKind::Nullable(i) => simple(self, DebugKind::Nullable, &[*i]),
            TypeKind::Interface(_) | TypeKind::Object => simple(self, DebugKind::Object, &[]),
            TypeKind::Struct(s, _) => {
                // Named as the object's descriptor is, which interface values are found by.
                let s = *s;
                let d = self.b.debug_type(DebugKind::Struct, m.struct_name(s), &[]);
                self.debug_types.insert(t, d);
                for i in 0..m.struct_field_count(s) {
                    let (field, ft) = m.struct_field(s, i);
                    let ft = self.debug_type(ft);
                    self.b.debug_type_add_field(d, field, ft);
                }
                d
            }
            TypeKind::Enum(e, _) => {
                let e = *e;
                let d = self.b.debug_type(DebugKind::Enum, m.enum_name(e), &[]);
                self.debug_types.insert(t, d);
                for v in 0..m.variant_count(e) {
                    let fields: Vec<DebugType> = m
                        .variant_fields(e, v)
                        .iter()
                        .map(|&f| self.debug_type(f))
                        .collect();
                    self.b
                        .debug_type_add_variant(d, m.variant_name(e, v), &fields);
                }
                d
            }
            _ => simple(self, DebugKind::Opaque, &[]),
        };
        self.debug_types.insert(t, d);
        d
    }

    /// Clears the slots in use and pops the frame, before a return.
    fn pop_frame(&mut self) {
        if self.st().frame.is_none() {
            return;
        }
        let next = self.st().next;
        self.clear_slots(0, next);
        let n = self.st().slot_count;
        self.b.call(self.rt.frame_pop, &[n]);
    }

    /// Whether a body can need a frame slot: only references and `any` values are rooted, so a body
    /// without any (no such local, capture or expression) skips pushing a frame. Numeric code then
    /// makes no runtime calls at all.
    fn needs_frame(&self, body: BodyId, closure: bool) -> bool {
        let m = self.m;
        if closure && !self.captures[&body].is_empty() {
            return true;
        }
        let gc_local = m
            .locals
            .iter()
            .enumerate()
            .any(|(i, l)| l.body == body && (self.captured[i] || self.is_gc(l.ty)));
        if gc_local {
            return true;
        }
        let Some(root) = m.body_root(body) else {
            return false;
        };
        // Closures are bodies of their own; creating one is itself a reference.
        let mut stack = vec![root];
        while let Some(e) = stack.pop() {
            if self.is_gc(m.expr_type(e)) {
                return true;
            }
            visit_children(m, e, &mut |c| stack.push(c));
        }
        false
    }

    // ---------------------------------------------------------------- may allocate?

    /// Whether evaluating `e` may allocate (and so collect).
    fn may_gc(&mut self, e: ExprId) -> bool {
        if let Some(&r) = self.st().gc_memo.get(&e) {
            return r;
        }
        let m = self.m;
        let own = match &m.expr(e).kind {
            ExprKind::Let(l, _) | ExprKind::IfLet(l, ..) | ExprKind::Try(_, l, _) => {
                self.captured[l.index()]
            }
            // Boxing the value may allocate.
            ExprKind::Throw(_) => true,
            ExprKind::Func(..)
            | ExprKind::Lambda(_)
            | ExprKind::Call(..)
            | ExprKind::CallExtern(..)
            | ExprKind::CallValue(..)
            | ExprKind::New(..)
            | ExprKind::Variant(..)
            | ExprKind::Array(_)
            | ExprKind::NewArray
            | ExprKind::NewMap
            | ExprKind::NewObject => true,
            ExprKind::Binary(BinOp::Add, a, _) => m.expr_type(*a) == TypeId::STRING,
            ExprKind::Builtin(op, args) => builtin_allocates(m, *op, args),
            ExprKind::Cast(a) => m.expr_type(*a) == TypeId::INT && m.expr_type(e) == TypeId::ANY,
            ExprKind::Match(_, arms) => arms.iter().any(|a| {
                m.arms[a.index()]
                    .binds
                    .iter()
                    .any(|b| self.captured[b.index()])
            }),
            _ => false,
        };
        let mut children = Vec::new();
        visit_children(m, e, &mut |c| children.push(c));
        let r = own || children.into_iter().any(|c| self.may_gc(c));
        self.st().gc_memo.insert(e, r);
        r
    }

    /// Evaluates operands left to right. A GC value is spilled when an operand after it, or the
    /// operation itself (`op_allocates`), may allocate. Returns `None` if an operand diverges.
    fn operands(&mut self, es: &[ExprId], op_allocates: bool) -> Option<Vec<V>> {
        let later: Vec<bool> = (0..es.len())
            .map(|i| es[i + 1..].iter().any(|&e| self.may_gc(e)))
            .collect();
        let mut out = Vec::new();
        for (i, &e) in es.iter().enumerate() {
            let v = self.expr(e);
            if matches!(v, V::Never) {
                return None;
            }
            if op_allocates || later[i] {
                self.spill_operand(e, v);
            }
            out.push(v);
        }
        Some(out)
    }

    // ---------------------------------------------------------------- expressions

    fn expr(&mut self, e: ExprId) -> V {
        let mark = self.mark();
        let outer = self.b.location();
        self.at(e);
        let r = self.expr_inner(e);
        // What the enclosing expression builds after this one is its own.
        self.b.set_location(outer);
        self.release(mark);
        r
    }

    fn expr_inner(&mut self, e: ExprId) -> V {
        let m = self.m;
        let ty = m.expr_type(e);
        let kind = m.expr(e).kind.clone();
        match kind {
            ExprKind::Unit => V::Unit,
            ExprKind::Bool(b) => V::Val(self.b.iconst(Type::I1, b as i64)),
            ExprKind::Int(v) => {
                let t = self.ir_type(ty).unwrap_or(Type::I64);
                let v = match self.int_bits(ty) {
                    // The literal's bits at its width, as the IR expects.
                    Some((bits, _)) if bits < 64 => v & ((1i64 << bits) - 1),
                    _ => v,
                };
                V::Val(self.b.iconst(t, v))
            }
            ExprKind::Float(bits) if ty == TypeId::F32 => {
                let single = (f64::from_bits(bits) as f32).to_bits();
                V::Val(self.b.fconst_bits(Type::F32, single as u64))
            }
            ExprKind::Float(bits) => V::Val(self.b.fconst_bits(Type::F64, bits)),
            ExprKind::Str(s) => {
                let g = self.string_global(&s);
                let a = self.b.global_addr(g);
                V::Val(self.b.load(Type::Ptr, a))
            }
            ExprKind::Null => V::Val(self.b.null()),
            ExprKind::Local(l) => self.read_local(l),
            ExprKind::Global(g) => match self.globals[g.index()] {
                Some(ir) => {
                    let a = self.b.global_addr(ir);
                    V::Val(self.b.load(self.ir_type(ty).unwrap(), a))
                }
                None => V::Unit,
            },
            ExprKind::Func(f, _) => {
                let (w, desc) = self.wrappers[&f];
                let obj = self.new_object(desc);
                let code = self.b.func_addr(w);
                let word = self.b.cast(Opcode::Ptrtoint, code, Type::I64);
                self.store_word(obj, 0, word);
                V::Val(obj)
            }
            ExprKind::Let(l, init) => {
                let v = self.expr(init);
                if matches!(v, V::Never) {
                    return V::Never;
                }
                self.bind_local(l, v);
                V::Unit
            }
            ExprKind::SetLocal(l, value) => {
                let v = self.expr(value);
                if matches!(v, V::Never) {
                    return V::Never;
                }
                self.write_local(l, v);
                V::Unit
            }
            ExprKind::SetGlobal(g, value) => {
                let v = self.expr(value);
                if matches!(v, V::Never) {
                    return V::Never;
                }
                if let (Some(ir), V::Val(v)) = (self.globals[g.index()], v) {
                    let a = self.b.global_addr(ir);
                    self.b.store(v, a);
                }
                V::Unit
            }
            ExprKind::SetField(o, f, value) => {
                let Some(vs) = self.operands(&[o, value], false) else {
                    return V::Never;
                };
                let obj = vs[0].val();
                let w = self.word_of(vs[1], m.expr_type(value));
                self.store_word(obj, f, w);
                V::Unit
            }
            ExprKind::SetIndex(t, i, value) => {
                let Some(vs) = self.operands(&[t, i, value], false) else {
                    return V::Never;
                };
                let target = vs[0].val();
                let vw = self.word_of(vs[2], m.expr_type(value));
                match *m.kind(m.expr_type(t)) {
                    TypeKind::Array(elem) => self.set_array_element(target, vs[1].val(), vw, elem),
                    _ => {
                        let kw = self.word_of(vs[1], m.expr_type(i));
                        self.b.call(self.rt.map_set, &[target, kw, vw]);
                    }
                }
                V::Unit
            }
            ExprKind::Unary(op, a) => {
                let v = self.expr(a);
                let V::Val(v) = v else { return v };
                V::Val(match op {
                    UnOp::Not | UnOp::BitNot => self.b.not(v),
                    UnOp::Neg if ty == TypeId::FLOAT || ty == TypeId::F32 => self.b.fneg(v),
                    UnOp::Neg => self.b.ineg(v),
                })
            }
            ExprKind::Binary(op, a, b) => self.binary(op, a, b),
            ExprKind::And(a, b) | ExprKind::Or(a, b) => {
                let is_and = matches!(m.expr(e).kind, ExprKind::And(..));
                let av = self.expr(a);
                let V::Val(av) = av else { return av };
                let result = self.b.declare_var("cond", Type::I1);
                self.b.set(result, av);
                let go = if is_and { av } else { self.b.not(av) };
                self.b.if_(go);
                let bv = self.expr(b);
                if let V::Val(bv) = bv {
                    self.b.set(result, bv);
                }
                self.b.end_if();
                V::Val(self.b.get(result))
            }
            ExprKind::Call(f, _, args) => {
                let Some(vs) = self.operands(&args, false) else {
                    return V::Never;
                };
                let ir_args: Vec<Value> = vs
                    .iter()
                    .filter_map(|v| if let V::Val(v) = v { Some(*v) } else { None })
                    .collect();
                let r = self.b.call(self.funcs[f.index()], &ir_args);
                if self.raises[m.func_body(f).index()] {
                    self.check_raised();
                }
                self.call_result(r, ty)
            }
            ExprKind::CallExtern(x, args) => {
                // A runtime function may allocate, so its reference arguments stay rooted; a C
                // function never collects, and gets pointers into the objects.
                let abi = m.extern_abi(x);
                let Some(vs) = self.operands(&args, abi == Abi::A0) else {
                    return V::Never;
                };
                let mut ir_args = Vec::new();
                for (v, &a) in vs.iter().zip(&args) {
                    if let V::Val(v) = *v {
                        ir_args.push(match abi {
                            Abi::A0 => v,
                            Abi::C => self.c_argument(v, m.expr_type(a)),
                        });
                    }
                }
                let mut r = self.b.call(self.externs[x.index()], &ir_args);
                if let (Abi::C, TypeKind::SizedInt(bits @ (8 | 16), _), Some(v)) =
                    (abi, m.kind(ty), r)
                {
                    r = Some(self.b.cast(Opcode::Trunc, v, int_type(*bits)));
                }
                let string = m.non_null(ty) == TypeId::STRING;
                match r {
                    Some(p) if abi == Abi::C && string => {
                        let required = self.b.iconst(Type::I64, (ty == TypeId::STRING) as i64);
                        let s = self.b.call(self.rt.string_from_c, &[p, required]);
                        self.call_result(s, ty)
                    }
                    _ => self.call_result(r, ty),
                }
            }
            ExprKind::CallValue(callee, args) => {
                let mut all = vec![callee];
                all.extend(args.iter().copied());
                let Some(vs) = self.operands(&all, false) else {
                    return V::Never;
                };
                let c = vs[0].val();
                let w = self.load_word(c, 0);
                let code = self.b.cast(Opcode::Inttoptr, w, Type::Ptr);
                let mut ir_args = vec![c];
                ir_args.extend(
                    vs[1..]
                        .iter()
                        .filter_map(|v| if let V::Val(v) = v { Some(*v) } else { None }),
                );
                let (params, ret) = self.fn_signature(m.expr_type(callee));
                let r = self.b.call_indirect(code, &params, ret, &ir_args);
                if self.closures_raise {
                    self.check_raised();
                }
                self.call_result(r, ty)
            }
            ExprKind::Lambda(body) => {
                let desc = self.lambda_desc[&body];
                let obj = self.new_object(desc);
                let code = self.b.func_addr(self.lambda_funcs[&body]);
                let word = self.b.cast(Opcode::Ptrtoint, code, Type::I64);
                self.store_word(obj, 0, word);
                for (k, l) in self.captures[&body].clone().into_iter().enumerate() {
                    let Storage::Boxed(var, _) = self.st().storage[&l] else {
                        unreachable!("captured locals are boxed")
                    };
                    let bx = self.b.get(var);
                    let bw = self.b.cast(Opcode::Ptrtoint, bx, Type::I64);
                    self.store_word(obj, k as u32 + 1, bw);
                }
                V::Val(obj)
            }
            ExprKind::New(s, fields) => {
                let desc = self.struct_desc[s.index()];
                self.construct(
                    desc,
                    None,
                    &fields,
                    &m.structs[s.index()]
                        .fields
                        .iter()
                        .map(|f| f.1)
                        .collect::<Vec<_>>(),
                )
            }
            ExprKind::Variant(en, v, args) => {
                let desc = self.enum_desc[en.index()].1[v as usize];
                let field_types = m.variant_fields(en, v).to_vec();
                self.construct(desc, Some(v), &args, &field_types)
            }
            ExprKind::Field(o, f) => {
                let v = self.expr(o);
                let V::Val(obj) = v else { return v };
                let w = self.load_word(obj, f);
                self.word_decode(w, ty)
            }
            ExprKind::Unwrap(o) => {
                let v = self.expr(o);
                if let V::Val(p) = v {
                    self.null_check(p);
                }
                v
            }
            ExprKind::FieldName(..) | ExprKind::SetFieldName(..) => {
                unreachable!("finish resolves fields named before their object's type was known")
            }
            ExprKind::Match(s, arms) => self.match_(s, &arms, ty),
            ExprKind::Array(elems) => {
                let elem_ty = match m.kind(ty) {
                    TypeKind::Array(t) => *t,
                    _ => TypeId::NEVER,
                };
                let kind = self.b.iconst(Type::I64, self.elem_kind(elem_ty) as i64);
                let cap = self.b.iconst(Type::I64, elems.len() as i64);
                let arr = self.b.call(self.rt.array_new, &[kind, cap]).unwrap();
                self.spill(arr, ty);
                for el in elems {
                    let v = self.expr(el);
                    if matches!(v, V::Never) {
                        return V::Never;
                    }
                    let w = self.word_of(v, elem_ty);
                    self.b.call(self.rt.array_push, &[arr, w]);
                }
                V::Val(arr)
            }
            ExprKind::NewArray => {
                let TypeKind::Array(elem) = *m.kind(ty) else {
                    unreachable!()
                };
                let kind = self.b.iconst(Type::I64, self.elem_kind(elem) as i64);
                let cap = self.b.iconst(Type::I64, 0);
                V::Val(self.b.call(self.rt.array_new, &[kind, cap]).unwrap())
            }
            ExprKind::NewObject => V::Val(self.b.call(self.rt.object_new, &[]).unwrap()),
            ExprKind::NewMap => {
                let TypeKind::Map(k, v) = *m.kind(ty) else {
                    unreachable!()
                };
                let kk = self.b.iconst(
                    Type::I64,
                    if k == TypeId::STRING {
                        KEY_STRING
                    } else if k == TypeId::BOOL {
                        KEY_BOOL
                    } else {
                        KEY_RAW
                    },
                );
                let vk = self.b.iconst(Type::I64, self.word_kind(v) as i64);
                V::Val(self.b.call(self.rt.map_new, &[kk, vk]).unwrap())
            }
            ExprKind::Index(t, i) => {
                let Some(vs) = self.operands(&[t, i], false) else {
                    return V::Never;
                };
                let target = vs[0].val();
                let w = match *m.kind(m.expr_type(t)) {
                    TypeKind::Array(elem) => self.array_element(target, vs[1].val(), elem),
                    _ => {
                        let kw = self.word_of(vs[1], m.expr_type(i));
                        self.b.call(self.rt.map_get, &[target, kw]).unwrap()
                    }
                };
                self.word_decode(w, ty)
            }
            ExprKind::Builtin(op, args) => self.builtin(op, &args, ty),
            ExprKind::Cast(a) if m.unchecked_casts.contains(&e) => {
                // A test of the same `any` guards it: unpack without checking again.
                let v = self.expr(a);
                let V::Val(w) = v else { return v };
                self.unpack_known(w, ty)
            }
            ExprKind::Cast(a) => self.cast(a, ty),
            ExprKind::Is(a, t) => {
                let v = self.expr(a);
                let V::Val(w) = v else { return v };
                V::Val(self.is_type(w, t))
            }
            ExprKind::If(c, t, els) => self.if_(c, t, els, ty),
            ExprKind::IfLet(l, v, t, els) => self.if_let(l, v, t, els, ty),
            ExprKind::While(c, body) => {
                self.b.loop_();
                let mark = self.mark();
                self.st().exits.push(Exit::Loop(mark));
                let cv = self.expr(c);
                if let V::Val(cv) = cv {
                    let stop = self.b.not(cv);
                    self.b.if_(stop);
                    self.b.break_();
                    self.b.end_if();
                    self.expr(body);
                }
                self.st().exits.pop();
                self.b.end_loop();
                V::Unit
            }
            ExprKind::Loop(body) => {
                self.b.loop_();
                let mark = self.mark();
                self.st().exits.push(Exit::Loop(mark));
                self.expr(body);
                self.st().exits.pop();
                self.b.end_loop();
                if ty == TypeId::NEVER {
                    V::Never
                } else {
                    V::Unit
                }
            }
            ExprKind::Break => {
                let depth = self.leave_loop_scopes();
                self.b.break_out(depth);
                V::Never
            }
            ExprKind::Continue => {
                let depth = self.leave_loop_scopes();
                self.b.continue_out(depth);
                V::Never
            }
            ExprKind::Throw(v) => {
                let vt = m.expr_type(v);
                let r = self.expr(v);
                if matches!(r, V::Never) {
                    return V::Never;
                }
                let w = self.box_any(r, vt);
                let (pending, value) = self.exception.expect("a throw makes the program raise");
                let a = self.b.global_addr(value);
                self.b.store(w, a);
                let one = self.b.iconst(Type::I64, 1);
                let a = self.b.global_addr(pending);
                self.b.store(one, a);
                // Remembers where it was thrown, for an exception nothing catches.
                self.b.call(self.rt.throw, &[]);
                self.unwind();
                V::Never
            }
            ExprKind::Try(body, l, handler) => self.try_(body, l, handler, ty),
            ExprKind::MethodCall(..) => {
                unreachable!("calls of methods are resolved before lowering")
            }
            ExprKind::InterfaceCall(receiver, i, k, args) => {
                let mut all = vec![receiver];
                all.extend(args.iter().copied());
                let Some(vs) = self.operands(&all, false) else {
                    return V::Never;
                };
                let object = vs[0].val();
                let vtable = self.vtable(object, i);
                let off = self.b.iconst(Type::I64, 8 * k as i64);
                let at = self.b.ptradd(vtable, off);
                let code = self.b.load(Type::Ptr, at);
                let mut params = vec![Type::Ptr];
                let mut ir_args = vec![object];
                for (v, &a) in vs[1..].iter().zip(&args) {
                    if let V::Val(v) = *v {
                        params.push(self.ir_type(m.expr_type(a)).unwrap());
                        ir_args.push(v);
                    }
                }
                let ret = self.ir_type(ty);
                let r = self.b.call_indirect(code, &params, ret, &ir_args);
                if self.closures_raise {
                    self.check_raised();
                }
                self.call_result(r, ty)
            }
            ExprKind::Return(v) => {
                let r = match v {
                    Some(v) => self.expr(v),
                    None => V::Unit,
                };
                if matches!(r, V::Never) {
                    return V::Never;
                }
                self.pop_frame();
                let body = self.st().body;
                match (self.ir_type(m.body_ret(body)), r) {
                    (Some(_), V::Val(v)) => self.b.ret(Some(v)),
                    _ => self.b.ret(None),
                }
                V::Never
            }
            ExprKind::Block(items, tail) => {
                // The block's locals get their slots now; `expr` releases them when it ends.
                for &item in &items {
                    if let ExprKind::Let(l, _) = m.expr(item).kind {
                        self.declare_local(l);
                    }
                }
                for item in items {
                    if matches!(self.expr(item), V::Never) {
                        return V::Never;
                    }
                }
                match tail {
                    Some(t) => self.expr(t),
                    None => V::Unit,
                }
            }
        }
    }

    /// Clears the slots of the scopes that `break` or `continue` leaves, and returns how many IR
    /// loops out the HIR loop is (the `try`s inside it are loops too).
    fn leave_loop_scopes(&mut self) -> u32 {
        let st = self.st();
        let i = st
            .exits
            .iter()
            .rposition(|x| matches!(x, Exit::Loop(_)))
            .expect("the checker keeps break and continue in loops");
        let Exit::Loop(from) = st.exits[i] else {
            unreachable!()
        };
        let (depth, to) = ((st.exits.len() - 1 - i) as u32, st.next);
        self.clear_slots(from, to);
        depth
    }

    // ---------------------------------------------------------------- exceptions

    /// Which bodies `main` can reach, through calls and the closures it makes (the others are not
    /// lowered, so a library or a prelude costs only what the program uses), and which of those may
    /// raise an exception into their caller: those that throw, call one that may, or call a closure
    /// while some closure the program makes may. Calls to the others need no check, so a program
    /// that never throws pays nothing.
    fn analyze_raises(&mut self) {
        let m = self.m;
        let n = m.bodies.len();
        self.raises = vec![false; n];
        let Some(Item::Func(main)) = m.lookup_value("main") else {
            self.reached = vec![true; n];
            return;
        };
        // What each body does itself: throw, call closures, call bodies, make closures of bodies.
        let mut throws = vec![false; n];
        let mut calls_closures = vec![false; n];
        let mut calls: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut makes: Vec<Vec<usize>> = vec![Vec::new(); n];
        for b in 0..n {
            let mut stack: Vec<ExprId> = m.bodies[b].root.into_iter().collect();
            while let Some(e) = stack.pop() {
                match &m.expr(e).kind {
                    ExprKind::Throw(_) => throws[b] = true,
                    ExprKind::CallValue(..) | ExprKind::InterfaceCall(..) => {
                        calls_closures[b] = true
                    }
                    ExprKind::Call(f, ..) => calls[b].push(m.func_body(*f).index()),
                    ExprKind::Func(f, _) => makes[b].push(m.func_body(*f).index()),
                    ExprKind::Lambda(body) => makes[b].push(body.index()),
                    _ => {}
                }
                visit_children(m, e, &mut |c| stack.push(c));
            }
        }
        let mut reached = vec![false; n];
        let mut closure = vec![false; n];
        let mut stack = vec![m.func_body(main).index()];
        // Methods in tables are called through interfaces, like closures.
        for (_, _, methods) in &m.vtables {
            for &f in methods {
                let b = m.func_body(f).index();
                closure[b] = true;
                stack.push(b);
            }
        }
        while let Some(b) = stack.pop() {
            if std::mem::replace(&mut reached[b], true) {
                continue;
            }
            stack.extend(&calls[b]);
            for &c in &makes[b] {
                closure[c] = true;
                stack.push(c);
            }
        }
        loop {
            let closures = (0..n).any(|b| closure[b] && self.raises[b]);
            let mut changed = false;
            for b in 0..n {
                if !reached[b] || self.raises[b] {
                    continue;
                }
                if throws[b]
                    || (calls_closures[b] && closures)
                    || calls[b].iter().any(|&c| self.raises[c])
                {
                    self.raises[b] = true;
                    changed = true;
                }
            }
            if !changed {
                self.closures_raise = closures;
                break;
            }
        }
        self.reached = reached;
    }

    /// After a call that may raise: if an exception is pending, leaves.
    fn check_raised(&mut self) {
        let Some((pending, _)) = self.exception else {
            return;
        };
        let a = self.b.global_addr(pending);
        let p = self.b.load(Type::I64, a);
        let zero = self.b.iconst(Type::I64, 0);
        let raised = self.b.icmp(IntCC::Ne, p, zero);
        self.b.if_(raised);
        self.unwind();
        self.b.end_if();
    }

    /// Leaves the code where an exception was raised: to the innermost `try` around it in this
    /// function, or else out of the function with the exception still pending, for the caller.
    fn unwind(&mut self) {
        let st = self.st();
        match st.exits.iter().rposition(|x| matches!(x, Exit::Try(_))) {
            Some(i) => {
                let Exit::Try(from) = st.exits[i] else {
                    unreachable!()
                };
                let (depth, to) = ((st.exits.len() - 1 - i) as u32, st.next);
                self.clear_slots(from, to);
                self.b.break_out(depth);
            }
            None => {
                self.pop_frame();
                let ret = self.m.body_ret(self.st().body);
                let zero = match self.ir_type(ret) {
                    None => None,
                    Some(Type::Ptr) => Some(self.b.null()),
                    Some(Type::F64) => Some(self.b.fconst_bits(Type::F64, 0)),
                    Some(t) => Some(self.b.iconst(t, 0)),
                };
                self.b.ret(zero);
            }
        }
    }

    /// `try { body } catch l { handler }`: the body runs in a loop that a raised exception leaves;
    /// after it, a pending exception is taken and handled.
    fn try_(&mut self, body: ExprId, l: LocalId, handler: ExprId, ty: TypeId) -> V {
        let Some((pending, value)) = self.exception else {
            // Nothing in the program throws, so the handler never runs.
            return self.expr(body);
        };
        let result = match self.ir_type(ty) {
            Some(t) if ty != TypeId::NEVER => Some(self.b.declare_var("try", t)),
            _ => None,
        };
        self.b.loop_();
        let mark = self.mark();
        self.st().exits.push(Exit::Try(mark));
        let bv = self.expr(body);
        self.st().exits.pop();
        if let (Some(r), V::Val(x)) = (result, bv) {
            self.b.set(r, x);
        }
        if !self.b.is_terminated() {
            self.b.break_();
        }
        self.b.end_loop();
        if self.b.is_terminated() {
            // The body always returns.
            return V::Never;
        }
        let a = self.b.global_addr(pending);
        let p = self.b.load(Type::I64, a);
        let zero = self.b.iconst(Type::I64, 0);
        let raised = self.b.icmp(IntCC::Ne, p, zero);
        self.b.if_(raised);
        self.b.store(zero, a);
        let va = self.b.global_addr(value);
        let w = self.b.load(Type::I64, va);
        let mark = self.mark();
        self.declare_local(l);
        // Binding a captured local allocates its box; the value stays rooted in the global till then.
        self.bind_local(l, V::Val(w));
        self.b.store(zero, va);
        let hv = self.expr(handler);
        if let (Some(r), V::Val(x)) = (result, hv) {
            self.b.set(r, x);
        }
        self.release(mark);
        self.b.end_if();
        if ty == TypeId::NEVER {
            if !self.b.is_terminated() {
                self.b.unreachable();
            }
            return V::Never;
        }
        match result {
            Some(r) => V::Val(self.b.get(r)),
            None => V::Unit,
        }
    }

    fn call_result(&mut self, r: Option<Value>, ty: TypeId) -> V {
        if ty == TypeId::NEVER {
            self.b.unreachable();
            return V::Never;
        }
        match r {
            Some(v) => V::Val(v),
            None => V::Unit,
        }
    }

    fn read_local(&mut self, l: LocalId) -> V {
        let t = self.m.local_type(l);
        match self.st().storage[&l] {
            Storage::None => V::Unit,
            Storage::Var(v) | Storage::Rooted(v, _) => V::Val(self.b.get(v)),
            Storage::Boxed(var, _) => {
                let bx = self.b.get(var);
                let w = self.load_word(bx, 0);
                self.word_decode(w, t)
            }
        }
    }

    fn write_local(&mut self, l: LocalId, v: V) {
        let t = self.m.local_type(l);
        let V::Val(v) = v else { return };
        match self.st().storage[&l] {
            Storage::None => {}
            Storage::Var(var) => self.b.set(var, v),
            Storage::Rooted(var, s) => {
                self.b.set(var, v);
                self.store_slot(s, v, t);
            }
            Storage::Boxed(var, _) => {
                let bx = self.b.get(var);
                let w = self.word_encode(v, t);
                self.store_word(bx, 0, w);
            }
        }
    }

    /// `let`: like a write, except that a captured local gets a fresh box every time.
    fn bind_local(&mut self, l: LocalId, v: V) {
        let t = self.m.local_type(l);
        match self.st().storage[&l] {
            Storage::Boxed(var, s) => {
                let mark = self.mark();
                if let V::Val(val) = v
                    && self.is_gc(t)
                {
                    self.spill(val, t);
                }
                let bx = self.new_object(self.box_desc(t));
                if let V::Val(val) = v {
                    let w = self.word_encode(val, t);
                    self.store_word(bx, 0, w);
                }
                self.b.set(var, bx);
                self.store_slot(s, bx, TypeId::STRING);
                self.release(mark);
            }
            _ => self.write_local(l, v),
        }
    }

    /// Allocates an object, keeps it rooted, then evaluates and stores its fields (after the tag).
    fn construct(
        &mut self,
        desc: IrGlobal,
        tag: Option<u32>,
        fields: &[ExprId],
        types: &[TypeId],
    ) -> V {
        let obj = self.new_object(desc);
        self.spill(obj, TypeId::STRING);
        let first = match tag {
            Some(t) => {
                let w = self.b.iconst(Type::I64, t as i64);
                self.store_word(obj, 0, w);
                1
            }
            None => 0,
        };
        for (i, (&f, &t)) in fields.iter().zip(types).enumerate() {
            let v = self.expr(f);
            if matches!(v, V::Never) {
                return V::Never;
            }
            let w = self.word_of(v, t);
            self.store_word(obj, first + i as u32, w);
        }
        V::Val(obj)
    }

    fn binary(&mut self, op: BinOp, a: ExprId, b: ExprId) -> V {
        let m = self.m;
        let t = if m.expr_type(a) == TypeId::NEVER {
            m.expr_type(b)
        } else {
            m.expr_type(a)
        };
        // `s == null` with a `string?` still compares strings (the runtime handles null).
        let t = m.non_null(t);
        let allocates = op == BinOp::Add && t == TypeId::STRING;
        let Some(vs) = self.operands(&[a, b], allocates) else {
            return V::Never;
        };
        let cmp_int = |op: BinOp| match op {
            BinOp::Lt => IntCC::Slt,
            BinOp::Le => IntCC::Sle,
            BinOp::Gt => IntCC::Sgt,
            BinOp::Ge => IntCC::Sge,
            BinOp::Eq => IntCC::Eq,
            _ => IntCC::Ne,
        };
        if t == TypeId::UNIT {
            return V::Val(self.b.iconst(Type::I1, (op == BinOp::Eq) as i64));
        }
        let (x, y) = (vs[0].val(), vs[1].val());
        let cmp_uint = |op: BinOp| match op {
            BinOp::Lt => IntCC::Ult,
            BinOp::Le => IntCC::Ule,
            BinOp::Gt => IntCC::Ugt,
            BinOp::Ge => IntCC::Uge,
            BinOp::Eq => IntCC::Eq,
            _ => IntCC::Ne,
        };
        V::Val(match m.kind(t) {
            TypeKind::Int | TypeKind::SizedInt(..) => {
                let signed = self.int_bits(t).is_none_or(|(_, s)| s);
                match op {
                    BinOp::Add => self.b.iadd(x, y),
                    BinOp::Sub => self.b.isub(x, y),
                    BinOp::Mul => self.b.imul(x, y),
                    BinOp::Div if signed => self.b.sdiv(x, y),
                    BinOp::Div => self.b.binary(Opcode::Udiv, x, y),
                    BinOp::Rem if signed => self.b.srem(x, y),
                    BinOp::Rem => self.b.binary(Opcode::Urem, x, y),
                    BinOp::BitAnd => self.b.and(x, y),
                    BinOp::BitOr => self.b.or(x, y),
                    BinOp::BitXor => self.b.xor(x, y),
                    BinOp::Shl => self.b.binary(Opcode::Shl, x, y),
                    BinOp::Shr if signed => self.b.binary(Opcode::Ashr, x, y),
                    BinOp::Shr => self.b.binary(Opcode::Lshr, x, y),
                    _ if signed => self.b.icmp(cmp_int(op), x, y),
                    _ => self.b.icmp(cmp_uint(op), x, y),
                }
            }
            TypeKind::Bool if op == BinOp::BitAnd => self.b.and(x, y),
            TypeKind::Bool if op == BinOp::BitOr => self.b.or(x, y),
            TypeKind::Bool if op == BinOp::BitXor => self.b.xor(x, y),
            TypeKind::Float | TypeKind::F32 => match op {
                BinOp::Add => self.b.fadd(x, y),
                BinOp::Sub => self.b.fsub(x, y),
                BinOp::Mul => self.b.fmul(x, y),
                BinOp::Div => self.b.fdiv(x, y),
                BinOp::Lt => self.b.fcmp(FloatCC::Lt, x, y),
                BinOp::Le => self.b.fcmp(FloatCC::Le, x, y),
                BinOp::Gt => self.b.fcmp(FloatCC::Gt, x, y),
                BinOp::Ge => self.b.fcmp(FloatCC::Ge, x, y),
                BinOp::Eq => self.b.fcmp(FloatCC::Eq, x, y),
                _ => self.b.fcmp(FloatCC::Ne, x, y),
            },
            TypeKind::Bool => self.b.icmp(cmp_int(op), x, y),
            TypeKind::String => match op {
                BinOp::Add => self.b.call(self.rt.string_concat, &[x, y]).unwrap(),
                BinOp::Eq | BinOp::Ne => {
                    let r = self.b.call(self.rt.string_eq, &[x, y]).unwrap();
                    let zero = self.b.iconst(Type::I64, 0);
                    self.b.icmp(
                        if op == BinOp::Eq {
                            IntCC::Ne
                        } else {
                            IntCC::Eq
                        },
                        r,
                        zero,
                    )
                }
                _ => {
                    let c = self.b.call(self.rt.string_compare, &[x, y]).unwrap();
                    let zero = self.b.iconst(Type::I64, 0);
                    self.b.icmp(cmp_int(op), c, zero)
                }
            },
            TypeKind::Any => {
                let eq = self.any_eq(x, y);
                if op == BinOp::Eq { eq } else { self.b.not(eq) }
            }
            // Other references compare by identity.
            _ => self.b.icmp(cmp_int(op), x, y),
        })
    }

    /// A value of a printable type as a string.
    fn format(&mut self, v: Value, t: TypeId) -> Value {
        match t {
            TypeId::INT => self.b.call(self.rt.string_from_int, &[v]).unwrap(),
            TypeId::FLOAT => self.b.call(self.rt.string_from_float, &[v]).unwrap(),
            TypeId::U64 => self.b.call(self.rt.string_from_uint, &[v]).unwrap(),
            // The shortest text that reads back as the same `f32`.
            TypeId::F32 => {
                let d = self.b.cast(Opcode::Fext, v, Type::F64);
                self.b.call(self.rt.string_from_f32, &[d]).unwrap()
            }
            _ if self.int_bits(t).is_some() => {
                let w = self.word_encode(v, t);
                self.b.call(self.rt.string_from_int, &[w]).unwrap()
            }
            TypeId::STRING => v,
            _ => {
                let w = self.b.cast(Opcode::Zext, v, Type::I64);
                self.b.call(self.rt.string_from_bool, &[w]).unwrap()
            }
        }
    }

    fn builtin(&mut self, op: Builtin, args: &[ExprId], ty: TypeId) -> V {
        let m = self.m;
        let allocates = builtin_allocates(m, op, args);
        let Some(vs) = self.operands(args, allocates) else {
            return V::Never;
        };
        let arg_ty = |i: usize| m.expr_type(args[i]);
        match op {
            // Strings and arrays keep their length in their first word (references are never null).
            Builtin::Len => match m.kind(arg_ty(0)) {
                TypeKind::String | TypeKind::Array(_) => V::Val(self.load_word(vs[0].val(), 0)),
                _ => V::Val(self.b.call(self.rt.map_len, &[vs[0].val()]).unwrap()),
            },
            Builtin::Push => {
                let TypeKind::Array(elem) = *m.kind(arg_ty(0)) else {
                    return V::Never;
                };
                let w = self.word_of(vs[1], elem);
                self.b.call(self.rt.array_push, &[vs[0].val(), w]);
                V::Unit
            }
            Builtin::Pop => {
                let w = self.b.call(self.rt.array_pop, &[vs[0].val()]).unwrap();
                self.word_decode(w, ty)
            }
            Builtin::Has | Builtin::Remove => {
                let TypeKind::Map(k, _) = *m.kind(arg_ty(0)) else {
                    return V::Never;
                };
                let kw = self.word_of(vs[1], k);
                let f = if op == Builtin::Has {
                    self.rt.map_has
                } else {
                    self.rt.map_remove
                };
                let r = self.b.call(f, &[vs[0].val(), kw]).unwrap();
                let zero = self.b.iconst(Type::I64, 0);
                V::Val(self.b.icmp(IntCC::Ne, r, zero))
            }
            Builtin::Keys => V::Val(self.b.call(self.rt.map_keys, &[vs[0].val()]).unwrap()),
            Builtin::HasField | Builtin::Field => {
                let (o, name) = (vs[0].val(), vs[1].val());
                if !matches!(m.expr(args[1]).kind, ExprKind::Str(_)) {
                    return if op == Builtin::HasField {
                        let r = self.b.call(self.rt.object_has, &[o, name]).unwrap();
                        let zero = self.b.iconst(Type::I64, 0);
                        V::Val(self.b.icmp(IntCC::Ne, r, zero))
                    } else {
                        V::Val(self.b.call(self.rt.object_get, &[o, name]).unwrap())
                    };
                }
                let cache = self.field_cache();
                let index = self.cached_field(o, name, cache);
                let zero = self.b.iconst(Type::I64, 0);
                if op == Builtin::HasField {
                    return V::Val(self.b.icmp(IntCC::Sge, index, zero));
                }
                let result = self.b.declare_var("field", Type::I64);
                let missing = self.b.icmp(IntCC::Slt, index, zero);
                self.b.if_(missing);
                // Fails with the field's name.
                let r = self.b.call(self.rt.object_get, &[o, name]).unwrap();
                self.b.set(result, r);
                self.b.else_();
                let at = self.slot_address(o, index);
                let w = self.b.load(Type::I64, at);
                self.b.set(result, w);
                self.b.end_if();
                V::Val(self.b.get(result))
            }
            Builtin::SetField => {
                let (o, name) = (vs[0].val(), vs[1].val());
                let w = self.box_any(vs[2], arg_ty(2));
                if !matches!(m.expr(args[1]).kind, ExprKind::Str(_)) {
                    self.b.call(self.rt.object_set, &[o, name, w]);
                    return V::Unit;
                }
                // The cache says which slot the field has in objects of its `shape`, and which
                // shape setting it leaves them with (another one when it adds the field), if the
                // slot is within the object's capacity.
                let cache = self.field_cache();
                let shape = self.load_word(o, 0);
                let cached = self.load_word(cache, 0);
                let index = self.load_word(cache, 1);
                let cap = self.load_word(o, 1);
                let same = self.b.icmp(IntCC::Eq, shape, cached);
                let fits = self.b.icmp(IntCC::Slt, index, cap);
                let fast = self.b.and(same, fits);
                self.b.if_(fast);
                let at = self.slot_address(o, index);
                self.b.store(w, at);
                let to = self.load_word(cache, 2);
                self.store_word(o, 0, to);
                self.b.else_();
                self.b.call(self.rt.object_set_cached, &[o, name, w, cache]);
                self.b.end_if();
                V::Unit
            }
            Builtin::FieldNames => {
                V::Val(self.b.call(self.rt.object_names, &[vs[0].val()]).unwrap())
            }
            Builtin::Filled => {
                let TypeKind::Array(elem) = *m.kind(ty) else {
                    return V::Never;
                };
                let kind = self.b.iconst(Type::I64, self.elem_kind(elem) as i64);
                let w = self.word_of(vs[1], elem);
                let call = self.b.call(self.rt.array_filled, &[kind, vs[0].val(), w]);
                V::Val(call.unwrap())
            }
            Builtin::Substr => V::Val(
                self.b
                    .call(
                        self.rt.string_substr,
                        &[vs[0].val(), vs[1].val(), vs[2].val()],
                    )
                    .unwrap(),
            ),
            Builtin::ByteAt => V::Val(self.string_byte(vs[0].val(), vs[1].val())),
            Builtin::Str => V::Val(self.format(vs[0].val(), arg_ty(0))),
            Builtin::Print => {
                let s = self.format(vs[0].val(), arg_ty(0));
                self.b.call(self.rt.string_print, &[s]);
                V::Unit
            }
            Builtin::Eprint => {
                let s = self.format(vs[0].val(), arg_ty(0));
                self.b.call(self.rt.string_eprint, &[s]);
                V::Unit
            }
            Builtin::Exit => {
                let code = self.b.cast(Opcode::Trunc, vs[0].val(), Type::I32);
                self.b.call(self.rt.exit, &[code]);
                self.b.unreachable();
                V::Never
            }
            Builtin::Panic => {
                self.b.call(self.rt.string_panic, &[vs[0].val()]);
                self.b.unreachable();
                V::Never
            }
            Builtin::Clock => V::Val(self.b.call(self.rt.clock, &[]).unwrap()),
        }
    }

    fn cast(&mut self, a: ExprId, to: TypeId) -> V {
        let m = self.m;
        let from = m.expr_type(a);
        let v = self.expr(a);
        if matches!(v, V::Never) || from == to {
            return v;
        }
        if to == TypeId::ANY {
            return V::Val(self.box_any(v, from));
        }
        // `T` to `T?`, and an object to an interface it implements, change nothing at run time.
        if m.non_null(to) == from
            || (matches!(m.kind(to), TypeKind::Interface(_)) && from != TypeId::ANY)
        {
            return v;
        }
        let V::Val(v) = v else { return V::Unit };
        if let TypeKind::Nullable(inner) = *m.kind(to) {
            // From `any`: `()` is null, anything else must be a `T`.
            let result = self.b.declare_var("cast", Type::Ptr);
            let unit = self.b.iconst(Type::I64, ANY_UNIT);
            let is_unit = self.b.icmp(IntCC::Eq, v, unit);
            self.b.if_(is_unit);
            let null = self.b.null();
            self.b.set(result, null);
            self.b.else_();
            let V::Val(p) = self.unpack_any(v, inner) else {
                unreachable!("nullable types are references")
            };
            self.b.set(result, p);
            self.b.end_if();
            return V::Val(self.b.get(result));
        }
        if let Some(c) = self.numeric_cast(v, from, to) {
            return V::Val(c);
        }
        self.unpack_any(v, to)
    }

    /// Converts between numbers: integers wrap or extend by the source's sign, integers to floats
    /// round, floats to integers truncate and saturate (NaN is 0), floats widen or round. `None`
    /// if `from` or `to` is not a number.
    fn numeric_cast(&mut self, v: Value, from: TypeId, to: TypeId) -> Option<Value> {
        let float = |t: TypeId| match t {
            TypeId::FLOAT => Some(Type::F64),
            TypeId::F32 => Some(Type::F32),
            _ => None,
        };
        let to_ir = self.ir_type(to)?;
        Some(
            match (
                self.int_bits(from),
                float(from),
                self.int_bits(to),
                float(to),
            ) {
                (Some((fb, signed)), _, Some((tb, _)), _) => {
                    if fb == tb {
                        v
                    } else if fb > tb {
                        self.b.cast(Opcode::Trunc, v, to_ir)
                    } else if signed {
                        self.b.cast(Opcode::Sext, v, to_ir)
                    } else {
                        self.b.cast(Opcode::Zext, v, to_ir)
                    }
                }
                (Some((_, signed)), _, _, Some(f)) => {
                    let op = if signed {
                        Opcode::Sitofp
                    } else {
                        Opcode::Uitofp
                    };
                    self.b.cast(op, v, f)
                }
                (_, Some(_), Some((_, signed)), _) => {
                    let op = if signed {
                        Opcode::Fptosi
                    } else {
                        Opcode::Fptoui
                    };
                    self.b.cast(op, v, to_ir)
                }
                (_, Some(ff), _, Some(tf)) if ff == tf => v,
                (_, Some(Type::F32), _, Some(_)) => self.b.cast(Opcode::Fext, v, Type::F64),
                (_, Some(_), _, Some(_)) => self.b.cast(Opcode::Ftrunc, v, Type::F32),
                _ => return None,
            },
        )
    }

    /// Unpacks an `any` word that must hold a `to`.
    fn unpack_any(&mut self, v: Value, to: TypeId) -> V {
        let m = self.m;
        let ok = self.is_type(v, to);
        let bad = self.b.not(ok);
        self.b.if_(bad);
        let (name, len) = self.type_name_data(to);
        let np = self.b.global_addr(name);
        let nl = self.b.iconst(Type::I64, len as i64);
        self.b.call(self.rt.any_cast_failed, &[v, np, nl]);
        self.b.unreachable();
        self.b.end_if();
        let _ = m;
        self.unpack_known(v, to)
    }

    /// Unpacks an `any` word known to hold a `to`.
    fn unpack_known(&mut self, v: Value, to: TypeId) -> V {
        let m = self.m;
        match m.kind(to) {
            TypeKind::Unit => V::Unit,
            TypeKind::Float => V::Val(self.b.cast(Opcode::Bitcast, v, Type::F64)),
            TypeKind::Bool => {
                let t = self.b.iconst(Type::I64, ANY_TRUE);
                V::Val(self.b.icmp(IntCC::Eq, v, t))
            }
            TypeKind::Int => V::Val(self.b.call(self.rt.any_to_int, &[v]).unwrap()),
            _ => V::Val(self.decode(v, to)),
        }
    }

    /// Boxes a value of type `from` into an `any`.
    fn box_any(&mut self, v: V, from: TypeId) -> Value {
        let m = self.m;
        let V::Val(v) = v else {
            return self.b.iconst(Type::I64, ANY_UNIT);
        };
        match m.kind(from) {
            TypeKind::Float => {
                let is_nan = self.b.fcmp(FloatCC::Ne, v, v);
                let bits = self.b.cast(Opcode::Bitcast, v, Type::I64);
                let nan = self.b.iconst(Type::I64, CANONICAL_NAN);
                self.b.select(is_nan, nan, bits)
            }
            TypeKind::Bool => {
                let t = self.b.iconst(Type::I64, ANY_TRUE);
                let f = self.b.iconst(Type::I64, ANY_FALSE);
                self.b.select(v, t, f)
            }
            TypeKind::Int => self.b.call(self.rt.any_from_int, &[v]).unwrap(),
            // Sized numbers become an `int` or a `float`.
            TypeKind::SizedInt(..) => {
                let w = self.word_encode(v, from);
                self.b.call(self.rt.any_from_int, &[w]).unwrap()
            }
            TypeKind::F32 => {
                let d = self.b.cast(Opcode::Fext, v, Type::F64);
                self.box_any(V::Val(d), TypeId::FLOAT)
            }
            TypeKind::Any => v,
            TypeKind::Nullable(_) => {
                let null = self.b.null();
                let is_null = self.b.icmp(IntCC::Eq, v, null);
                let tagged = self.encode(v, from);
                let unit = self.b.iconst(Type::I64, ANY_UNIT);
                self.b.select(is_null, unit, tagged)
            }
            _ => self.encode(v, from),
        }
    }

    /// Whether the `any` word `w` holds a value of type `t`.
    fn is_type(&mut self, w: Value, t: TypeId) -> Value {
        let m = self.m;
        let zero = self.b.iconst(Type::I64, 0);
        match m.kind(t) {
            TypeKind::Float => {
                let q = self.b.iconst(Type::I64, QNAN);
                let bits = self.b.and(w, q);
                self.b.icmp(IntCC::Ne, bits, q)
            }
            TypeKind::Unit => {
                let u = self.b.iconst(Type::I64, ANY_UNIT);
                self.b.icmp(IntCC::Eq, w, u)
            }
            TypeKind::Bool => {
                let tr = self.b.iconst(Type::I64, ANY_TRUE);
                let fa = self.b.iconst(Type::I64, ANY_FALSE);
                let a = self.b.icmp(IntCC::Eq, w, tr);
                let b = self.b.icmp(IntCC::Eq, w, fa);
                self.b.or(a, b)
            }
            TypeKind::Int => {
                let r = self.b.call(self.rt.any_is_int, &[w]).unwrap();
                self.b.icmp(IntCC::Ne, r, zero)
            }
            TypeKind::String => {
                let r = self.b.call(self.rt.any_is_string, &[w]).unwrap();
                self.b.icmp(IntCC::Ne, r, zero)
            }
            TypeKind::Struct(s, _) => {
                let desc = self.struct_desc[s.index()];
                self.is_desc(w, desc)
            }
            TypeKind::Enum(e, _) => {
                let desc = self.enum_desc[e.index()].0;
                self.is_desc(w, desc)
            }
            TypeKind::Any => self.b.iconst(Type::I1, 1),
            TypeKind::Interface(i) => {
                // An object whose type has a table for the interface.
                let i = *i;
                let result = self.b.declare_var("is", Type::I1);
                let no = self.b.iconst(Type::I1, 0);
                self.b.set(result, no);
                let tag = self.b.iconst(Type::I64, REF_TAG);
                let bits = self.b.and(w, tag);
                let is_ref = self.b.icmp(IntCC::Eq, bits, tag);
                self.b.if_(is_ref);
                let object = self.decode(w, TypeId::STRING);
                let off = self.b.iconst(Type::I64, OBJECT_TYPE_OFFSET);
                let at = self.b.ptradd(object, off);
                let ty = self.b.load(Type::Ptr, at);
                let off = self.b.iconst(Type::I64, TYPE_ITABLE_OFFSET);
                let at = self.b.ptradd(ty, off);
                let itable = self.b.load(Type::Ptr, at);
                let null = self.b.null();
                let has = self.b.icmp(IntCC::Ne, itable, null);
                self.b.if_(has);
                let off = self.b.iconst(Type::I64, 8 * i.index() as i64);
                let at = self.b.ptradd(itable, off);
                let vtable = self.b.load(Type::Ptr, at);
                let implements = self.b.icmp(IntCC::Ne, vtable, null);
                self.b.set(result, implements);
                self.b.end_if();
                self.b.end_if();
                self.b.get(result)
            }
            TypeKind::Nullable(inner) => {
                let u = self.b.iconst(Type::I64, ANY_UNIT);
                let is_unit = self.b.icmp(IntCC::Eq, w, u);
                let is_inner = self.is_type(w, *inner);
                self.b.or(is_unit, is_inner)
            }
            _ => self.b.iconst(Type::I1, 0),
        }
    }

    /// Whether the `any` word `w` holds an object whose type family is the descriptor in `desc`:
    /// the object header's type, then the type's family, read inline (layouts in `a0rt.h`).
    fn is_desc(&mut self, w: Value, desc: IrGlobal) -> Value {
        let result = self.b.declare_var("is", Type::I1);
        let no = self.b.iconst(Type::I1, 0);
        self.b.set(result, no);
        let tag = self.b.iconst(Type::I64, REF_TAG);
        let bits = self.b.and(w, tag);
        let is_ref = self.b.icmp(IntCC::Eq, bits, tag);
        self.b.if_(is_ref);
        let object = self.decode(w, TypeId::STRING);
        let header_type = self.b.iconst(Type::I64, OBJECT_TYPE_OFFSET);
        let at = self.b.ptradd(object, header_type);
        let ty = self.b.load(Type::Ptr, at);
        let family = self.b.iconst(Type::I64, TYPE_FAMILY_OFFSET);
        let at = self.b.ptradd(ty, family);
        let family = self.b.load(Type::Ptr, at);
        let family = self.b.cast(Opcode::Ptrtoint, family, Type::I64);
        let a = self.b.global_addr(desc);
        let d = self.b.load(Type::Ptr, a);
        let d = self.b.cast(Opcode::Ptrtoint, d, Type::I64);
        let same = self.b.icmp(IntCC::Eq, family, d);
        self.b.set(result, same);
        self.b.end_if();
        self.b.get(result)
    }

    /// `x == y` on two `any` words: identical words and two floats are decided inline; anything
    /// else (strings, boxed integers) by the runtime.
    fn any_eq(&mut self, x: Value, y: Value) -> Value {
        let result = self.b.declare_var("eq", Type::I1);
        let same = self.b.icmp(IntCC::Eq, x, y);
        self.b.if_(same);
        // The same word is the same value, except NaN, which is not equal to itself.
        let nan = self.b.iconst(Type::I64, CANONICAL_NAN);
        let not_nan = self.b.icmp(IntCC::Ne, x, nan);
        self.b.set(result, not_nan);
        self.b.else_();
        let q = self.b.iconst(Type::I64, QNAN);
        let (xq, yq) = (self.b.and(x, q), self.b.and(y, q));
        let x_float = self.b.icmp(IntCC::Ne, xq, q);
        let y_float = self.b.icmp(IntCC::Ne, yq, q);
        let either = self.b.or(x_float, y_float);
        self.b.if_(either);
        let both = self.b.and(x_float, y_float);
        let xf = self.b.cast(Opcode::Bitcast, x, Type::F64);
        let yf = self.b.cast(Opcode::Bitcast, y, Type::F64);
        let feq = self.b.fcmp(FloatCC::Eq, xf, yf);
        let eq = self.b.and(both, feq);
        self.b.set(result, eq);
        self.b.else_();
        let r = self.b.call(self.rt.any_eq, &[x, y]).unwrap();
        let zero = self.b.iconst(Type::I64, 0);
        let eq = self.b.icmp(IntCC::Ne, r, zero);
        self.b.set(result, eq);
        self.b.end_if();
        self.b.end_if();
        self.b.get(result)
    }

    fn if_(&mut self, c: ExprId, t: ExprId, els: Option<ExprId>, ty: TypeId) -> V {
        let cv = self.expr(c);
        let V::Val(cv) = cv else { return cv };
        let result = match (els, self.ir_type(ty)) {
            (Some(_), Some(irt)) => Some(self.b.declare_var("if", irt)),
            _ => None,
        };
        self.b.if_(cv);
        let tv = self.expr(t);
        if let (Some(r), V::Val(v)) = (result, tv) {
            self.b.set(r, v);
        }
        if let Some(e) = els {
            self.b.else_();
            let ev = self.expr(e);
            if let (Some(r), V::Val(v)) = (result, ev) {
                self.b.set(r, v);
            }
        }
        self.b.end_if();
        if ty == TypeId::NEVER {
            if !self.b.is_terminated() {
                self.b.unreachable();
            }
            return V::Never;
        }
        match result {
            Some(r) => V::Val(self.b.get(r)),
            None => V::Unit,
        }
    }

    fn if_let(&mut self, l: LocalId, v: ExprId, t: ExprId, els: Option<ExprId>, ty: TypeId) -> V {
        let vv = self.expr(v);
        let V::Val(p) = vv else { return vv };
        let result = match (els, self.ir_type(ty)) {
            (Some(_), Some(irt)) => Some(self.b.declare_var("if", irt)),
            _ => None,
        };
        let null = self.b.null();
        let present = self.b.icmp(IntCC::Ne, p, null);
        self.b.if_(present);
        // The binding has storage only while the branch runs, like a block's local.
        let mark = self.mark();
        self.declare_local(l);
        self.bind_local(l, V::Val(p));
        let tv = self.expr(t);
        if let (Some(r), V::Val(x)) = (result, tv) {
            self.b.set(r, x);
        }
        self.release(mark);
        if let Some(e) = els {
            self.b.else_();
            let ev = self.expr(e);
            if let (Some(r), V::Val(x)) = (result, ev) {
                self.b.set(r, x);
            }
        }
        self.b.end_if();
        if ty == TypeId::NEVER {
            if !self.b.is_terminated() {
                self.b.unreachable();
            }
            return V::Never;
        }
        match result {
            Some(r) => V::Val(self.b.get(r)),
            None => V::Unit,
        }
    }

    fn match_(&mut self, s: ExprId, arms: &[ArmId], ty: TypeId) -> V {
        let m = self.m;
        let sv = self.expr(s);
        let V::Val(obj) = sv else { return sv };
        // Keep the scrutinee alive while payloads are bound (binding a captured local allocates).
        self.spill_operand(s, sv);
        let tag = self.load_word(obj, 0);
        let result = self.ir_type(ty).map(|t| self.b.declare_var("match", t));
        let mut open = 0;
        for (i, &a) in arms.iter().enumerate() {
            let arm = &m.arms[a.index()];
            let last = i + 1 == arms.len();
            if let Some(v) = arm.variant {
                if !last {
                    let want = self.b.iconst(Type::I64, v as i64);
                    let hit = self.b.icmp(IntCC::Eq, tag, want);
                    self.b.if_(hit);
                    open += 1;
                }
                for (k, &bind) in arm.binds.iter().enumerate() {
                    let w = self.load_word(obj, k as u32 + 1);
                    let bt = m.local_type(bind);
                    let bv = self.word_decode(w, bt);
                    self.bind_local(bind, bv);
                }
            }
            let bv = self.expr(arm.body);
            if let (Some(r), V::Val(v)) = (result, bv) {
                self.b.set(r, v);
            }
            if !last && arm.variant.is_some() {
                self.b.else_();
            }
            if arm.variant.is_none() {
                break;
            }
        }
        for _ in 0..open {
            self.b.end_if();
        }
        if ty == TypeId::NEVER {
            if !self.b.is_terminated() {
                self.b.unreachable();
            }
            return V::Never;
        }
        match result {
            Some(r) => V::Val(self.b.get(r)),
            None => V::Unit,
        }
    }
}

/// Whether a builtin allocates a GC object: one that makes a string or an array, or prints a value
/// that is not a string yet.
/// The IR integer type of a width.
fn int_type(bits: u32) -> Type {
    match bits {
        8 => Type::I8,
        16 => Type::I16,
        32 => Type::I32,
        _ => Type::I64,
    }
}

fn builtin_allocates(m: &HirModule, op: Builtin, args: &[ExprId]) -> bool {
    match op {
        Builtin::Keys | Builtin::Substr | Builtin::Str | Builtin::Filled | Builtin::FieldNames => {
            true
        }
        // Boxing an `int` may allocate.
        Builtin::SetField => m.expr_type(args[2]) == TypeId::INT,
        Builtin::Print | Builtin::Eprint => m.expr_type(args[0]) != TypeId::STRING,
        _ => false,
    }
}
