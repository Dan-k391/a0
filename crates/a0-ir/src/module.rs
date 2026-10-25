//! The IR data model and its read-only, handle-based accessors.
//!
//! All entities live in module-wide arenas and are referred to by `u32` handles, so the module is
//! the single owner and every accessor is a plain function of `(module, handle)`.

use std::collections::HashMap;

use a0_diag::{SourceMap, Span};

use crate::types::*;

/// A function signature: parameter types and an optional result type.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Signature {
    pub params: Vec<Type>,
    pub ret: Option<Type>,
}

#[derive(Clone)]
pub(crate) struct FuncData {
    pub name: String,
    pub sig: SigId,
    pub body: Option<BodyData>,
    pub span: Option<Span>,
    /// What stack traces call the function; `None` leaves it out of them.
    pub trace_name: Option<String>,
}

/// A place where a source-level function was inlined: instructions that came from its body name it
/// in stack traces, followed by the function that called it, at `call`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineSite {
    /// The inlined function, as traces name it.
    pub callee: String,
    /// The call that was inlined, in the function around it.
    pub call: Option<Span>,
    /// Where the function around the call was itself inlined, if it was.
    pub parent: Option<InlineId>,
}

#[derive(Clone)]
pub(crate) struct BodyData {
    pub params: Vec<Value>,
    pub vars: Vec<Var>,
    pub entry: Region,
}

#[derive(Clone)]
pub(crate) enum GlobalInit {
    /// Read-only bytes (`data`).
    Data(Vec<u8>),
    /// A mutable scalar (`global`), with the initial value's bits.
    Scalar(Type, u64),
}

#[derive(Clone)]
pub(crate) struct GlobalData {
    pub name: String,
    pub init: GlobalInit,
    pub span: Option<Span>,
}

#[derive(Clone, Copy)]
pub(crate) enum ValueDef {
    Param(u32),
    Inst(Inst),
}

#[derive(Clone)]
pub(crate) struct ValueData {
    pub ty: Type,
    pub name: Option<String>,
    pub func: FuncId,
    pub def: ValueDef,
}

#[derive(Clone)]
pub(crate) struct VarData {
    pub ty: Type,
    pub name: Option<String>,
    pub func: FuncId,
    /// The source-level variable it holds, for debuggers: its name, type and declaration.
    pub debug: Option<(String, DebugType, Option<Span>)>,
}

/// A source-level type, for debuggers.
#[derive(Clone)]
pub(crate) struct DebugTypeData {
    pub kind: DebugKind,
    pub name: String,
    pub elems: Vec<DebugType>,
    pub fields: Vec<(String, DebugType)>,
    pub variants: Vec<(String, Vec<DebugType>)>,
}

#[derive(Clone)]
pub(crate) struct InstData {
    pub opcode: Opcode,
    pub result: Value,
    pub args_start: u32,
    pub args_len: u32,
    /// Opcode-specific immediate: constant bits, condition code, callee, signature, variable, symbol.
    pub imm: u64,
    /// `if`: then and (optional) else region; `loop`: body region.
    pub regions: [Region; 2],
    pub func: FuncId,
    pub parent: Region,
    pub span: Option<Span>,
    /// The inline site the instruction came from (`NONE` if it was not inlined).
    pub inline: InlineId,
}

#[derive(Clone)]
pub(crate) struct RegionData {
    pub insts: Vec<Inst>,
    pub func: FuncId,
    /// The `if`/`loop` that owns this region, or `NONE` for a function body.
    pub owner: Inst,
}

/// Kinds of module items, in declaration order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Func(FuncId),
    Global(GlobalId),
}

/// An IR module. Build one with [`crate::Builder`] or [`crate::parse`].
#[derive(Clone)]
pub struct Module {
    pub(crate) name: String,
    pub(crate) items: Vec<Item>,
    pub(crate) funcs: Vec<FuncData>,
    pub(crate) globals: Vec<GlobalData>,
    pub(crate) sigs: Vec<Signature>,
    pub(crate) sig_map: HashMap<Signature, SigId>,
    pub(crate) values: Vec<ValueData>,
    pub(crate) vars: Vec<VarData>,
    pub(crate) insts: Vec<InstData>,
    pub(crate) regions: Vec<RegionData>,
    pub(crate) args: Vec<Value>,
    pub(crate) symbols: HashMap<String, Symbol>,
    pub(crate) inlines: Vec<InlineSite>,
    pub(crate) debug_types: Vec<DebugTypeData>,
    /// The sources spans point into, so that runtime errors can say where they happened.
    pub(crate) sources: Option<SourceMap>,
}

impl Module {
    pub fn new(name: &str) -> Module {
        Module {
            name: name.to_string(),
            items: Vec::new(),
            funcs: Vec::new(),
            globals: Vec::new(),
            sigs: Vec::new(),
            sig_map: HashMap::new(),
            values: Vec::new(),
            vars: Vec::new(),
            insts: Vec::new(),
            regions: Vec::new(),
            args: Vec::new(),
            symbols: HashMap::new(),
            inlines: Vec::new(),
            debug_types: Vec::new(),
            sources: None,
        }
    }

    /// Gives the module the sources its spans point into (the map the frontend registered them
    /// in), so that runtime errors say where in them they happened.
    pub fn set_sources(&mut self, sources: SourceMap) {
        self.sources = Some(sources);
    }

    pub fn sources(&self) -> Option<&SourceMap> {
        self.sources.as_ref()
    }

    pub fn inline_site(&self, id: InlineId) -> &InlineSite {
        &self.inlines[id.index()]
    }

    pub fn inline_count(&self) -> u32 {
        self.inlines.len() as u32
    }

    // ---------------------------------------------------------------- module

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn func_count(&self) -> u32 {
        self.funcs.len() as u32
    }

    pub fn global_count(&self) -> u32 {
        self.globals.len() as u32
    }

    /// Finds an item by symbol name.
    pub fn lookup(&self, name: &str) -> Option<Symbol> {
        self.symbols.get(name).copied()
    }

    pub fn lookup_func(&self, name: &str) -> Option<FuncId> {
        self.lookup(name).and_then(Symbol::as_func)
    }

    pub fn lookup_global(&self, name: &str) -> Option<GlobalId> {
        self.lookup(name).and_then(Symbol::as_global)
    }

    pub fn symbol_name(&self, sym: Symbol) -> &str {
        match sym.kind {
            SymbolKind::Func => &self.funcs[sym.index as usize].name,
            SymbolKind::Global => &self.globals[sym.index as usize].name,
        }
    }

    // ---------------------------------------------------------------- signatures

    pub fn sig_params(&self, sig: SigId) -> &[Type] {
        &self.sigs[sig.index()].params
    }

    pub fn sig_ret(&self, sig: SigId) -> Option<Type> {
        self.sigs[sig.index()].ret
    }

    // ---------------------------------------------------------------- functions

    pub fn func_name(&self, f: FuncId) -> &str {
        &self.funcs[f.index()].name
    }

    /// What stack traces call the function, or `None` if they leave it out.
    pub fn func_trace_name(&self, f: FuncId) -> Option<&str> {
        self.funcs[f.index()].trace_name.as_deref()
    }

    pub fn func_sig(&self, f: FuncId) -> SigId {
        self.funcs[f.index()].sig
    }

    pub fn func_params(&self, f: FuncId) -> &[Type] {
        self.sig_params(self.func_sig(f))
    }

    pub fn func_ret(&self, f: FuncId) -> Option<Type> {
        self.sig_ret(self.func_sig(f))
    }

    pub fn func_is_extern(&self, f: FuncId) -> bool {
        self.funcs[f.index()].body.is_none()
    }

    /// The parameter values of a defined function (empty for externs).
    pub fn func_param_values(&self, f: FuncId) -> &[Value] {
        match &self.funcs[f.index()].body {
            Some(b) => &b.params,
            None => &[],
        }
    }

    /// The variables declared in a defined function.
    pub fn func_vars(&self, f: FuncId) -> &[Var] {
        match &self.funcs[f.index()].body {
            Some(b) => &b.vars,
            None => &[],
        }
    }

    /// The body region of a defined function, or `None` for externs.
    pub fn func_body(&self, f: FuncId) -> Option<Region> {
        self.funcs[f.index()].body.as_ref().map(|b| b.entry)
    }

    pub fn func_span(&self, f: FuncId) -> Option<Span> {
        self.funcs[f.index()].span
    }

    // ---------------------------------------------------------------- globals

    pub fn global_name(&self, g: GlobalId) -> &str {
        &self.globals[g.index()].name
    }

    /// True for read-only `data` items, false for mutable scalar `global`s.
    pub fn global_is_data(&self, g: GlobalId) -> bool {
        matches!(self.globals[g.index()].init, GlobalInit::Data(_))
    }

    /// The bytes of a `data` item (empty for scalar globals).
    pub fn global_bytes(&self, g: GlobalId) -> &[u8] {
        match &self.globals[g.index()].init {
            GlobalInit::Data(bytes) => bytes,
            GlobalInit::Scalar(..) => &[],
        }
    }

    /// The type of a scalar global (`None` for `data`).
    pub fn global_type(&self, g: GlobalId) -> Option<Type> {
        match self.globals[g.index()].init {
            GlobalInit::Data(_) => None,
            GlobalInit::Scalar(ty, _) => Some(ty),
        }
    }

    /// The initial value bits of a scalar global: canonical integer bits, float bits, or 0 for null.
    pub fn global_init_bits(&self, g: GlobalId) -> u64 {
        match self.globals[g.index()].init {
            GlobalInit::Data(_) => 0,
            GlobalInit::Scalar(_, bits) => bits,
        }
    }

    pub fn global_span(&self, g: GlobalId) -> Option<Span> {
        self.globals[g.index()].span
    }

    // ---------------------------------------------------------------- values and variables

    pub fn value_type(&self, v: Value) -> Type {
        self.values[v.index()].ty
    }

    pub fn value_name(&self, v: Value) -> Option<&str> {
        self.values[v.index()].name.as_deref()
    }

    pub fn value_func(&self, v: Value) -> FuncId {
        self.values[v.index()].func
    }

    /// The instruction that defines `v`, or `None` for parameters.
    pub fn value_inst(&self, v: Value) -> Option<Inst> {
        match self.values[v.index()].def {
            ValueDef::Inst(i) => Some(i),
            ValueDef::Param(_) => None,
        }
    }

    /// The parameter index of `v`, or `None` if an instruction defines it.
    pub fn value_param_index(&self, v: Value) -> Option<u32> {
        match self.values[v.index()].def {
            ValueDef::Param(i) => Some(i),
            ValueDef::Inst(_) => None,
        }
    }

    pub fn var_type(&self, v: Var) -> Type {
        self.vars[v.index()].ty
    }

    pub fn var_name(&self, v: Var) -> Option<&str> {
        self.vars[v.index()].name.as_deref()
    }

    pub fn var_func(&self, v: Var) -> FuncId {
        self.vars[v.index()].func
    }

    // ---------------------------------------------------------------- debug information

    /// The source-level variable `v` holds, if the frontend said: its name and type. Variables
    /// without one are the compiler's own, unless the module has no debug types at all.
    pub fn var_debug(&self, v: Var) -> Option<(&str, DebugType)> {
        self.vars[v.index()]
            .debug
            .as_ref()
            .map(|(n, t, _)| (n.as_str(), *t))
    }

    /// Where the source-level variable `v` holds is declared, if known.
    pub fn var_debug_span(&self, v: Var) -> Option<Span> {
        self.vars[v.index()].debug.as_ref().and_then(|d| d.2)
    }

    pub fn debug_type_count(&self) -> u32 {
        self.debug_types.len() as u32
    }

    pub fn debug_type_kind(&self, t: DebugType) -> DebugKind {
        self.debug_types[t.index()].kind
    }

    /// The type as the language writes it (`[int]`, `Point`).
    pub fn debug_type_name(&self, t: DebugType) -> &str {
        &self.debug_types[t.index()].name
    }

    /// Element types: an array's, a map's key and value, what a nullable or a box holds.
    pub fn debug_type_elems(&self, t: DebugType) -> &[DebugType] {
        &self.debug_types[t.index()].elems
    }

    pub fn debug_type_field_count(&self, t: DebugType) -> u32 {
        self.debug_types[t.index()].fields.len() as u32
    }

    pub fn debug_type_field(&self, t: DebugType, i: u32) -> (&str, DebugType) {
        let (n, ty) = &self.debug_types[t.index()].fields[i as usize];
        (n, *ty)
    }

    pub fn debug_type_variant_count(&self, t: DebugType) -> u32 {
        self.debug_types[t.index()].variants.len() as u32
    }

    pub fn debug_type_variant(&self, t: DebugType, i: u32) -> (&str, &[DebugType]) {
        let (n, fields) = &self.debug_types[t.index()].variants[i as usize];
        (n, fields)
    }

    /// The struct or enum type named `name`, for objects whose descriptors say their type.
    pub fn debug_type_named(&self, name: &str) -> Option<DebugType> {
        self.debug_types
            .iter()
            .position(|d| matches!(d.kind, DebugKind::Struct | DebugKind::Enum) && d.name == name)
            .map(|i| DebugType(i as u32))
    }

    // ---------------------------------------------------------------- regions and instructions

    pub fn region_insts(&self, r: Region) -> &[Inst] {
        &self.regions[r.index()].insts
    }

    /// The `if` or `loop` instruction owning `r`, or `None` for a function body.
    pub fn region_owner(&self, r: Region) -> Option<Inst> {
        self.regions[r.index()].owner.option()
    }

    pub fn inst_opcode(&self, i: Inst) -> Opcode {
        self.insts[i.index()].opcode
    }

    pub fn inst_args(&self, i: Inst) -> &[Value] {
        let d = &self.insts[i.index()];
        &self.args[d.args_start as usize..(d.args_start + d.args_len) as usize]
    }

    pub fn inst_result(&self, i: Inst) -> Option<Value> {
        self.insts[i.index()].result.option()
    }

    pub fn inst_func(&self, i: Inst) -> FuncId {
        self.insts[i.index()].func
    }

    /// The region that contains `i`.
    pub fn inst_parent(&self, i: Inst) -> Region {
        self.insts[i.index()].parent
    }

    pub fn inst_span(&self, i: Inst) -> Option<Span> {
        self.insts[i.index()].span
    }

    /// Where the instruction was inlined, if it came from an inlined function.
    pub fn inst_inline(&self, i: Inst) -> Option<InlineId> {
        self.insts[i.index()].inline.option()
    }

    /// The constant of an `iconst`, in canonical form (see [`canonical_int`]).
    pub fn inst_iconst(&self, i: Inst) -> i64 {
        self.insts[i.index()].imm as i64
    }

    /// The bits of an `fconst` (f32 constants use the low 32 bits).
    pub fn inst_fconst_bits(&self, i: Inst) -> u64 {
        self.insts[i.index()].imm
    }

    /// The symbol of an `addr`.
    pub fn inst_symbol(&self, i: Inst) -> Symbol {
        Symbol::from_imm(self.insts[i.index()].imm)
    }

    pub fn inst_int_cc(&self, i: Inst) -> IntCC {
        int_cc_from_imm(self.insts[i.index()].imm)
    }

    pub fn inst_float_cc(&self, i: Inst) -> FloatCC {
        float_cc_from_imm(self.insts[i.index()].imm)
    }

    /// The callee of a `call`.
    pub fn inst_callee(&self, i: Inst) -> FuncId {
        FuncId(self.insts[i.index()].imm as u32)
    }

    /// The signature of a `call_indirect`.
    pub fn inst_sig(&self, i: Inst) -> SigId {
        SigId(self.insts[i.index()].imm as u32)
    }

    /// How many loops out from the innermost a `break` or `continue` goes (0: the innermost).
    pub fn inst_loop_depth(&self, i: Inst) -> u32 {
        self.insts[i.index()].imm as u32
    }

    /// The variable of a `get` or `set`.
    pub fn inst_var(&self, i: Inst) -> Var {
        Var(self.insts[i.index()].imm as u32)
    }

    /// The `n`th region of an `if` (0 = then, 1 = else) or `loop` (0 = body).
    pub fn inst_region(&self, i: Inst, n: u32) -> Option<Region> {
        self.insts[i.index()].regions[n as usize].option()
    }

    // ---------------------------------------------------------------- construction (crate-internal)

    pub(crate) fn intern_sig(&mut self, params: &[Type], ret: Option<Type>) -> SigId {
        let sig = Signature {
            params: params.to_vec(),
            ret,
        };
        if let Some(&id) = self.sig_map.get(&sig) {
            return id;
        }
        let id = SigId(self.sigs.len() as u32);
        self.sigs.push(sig.clone());
        self.sig_map.insert(sig, id);
        id
    }

    /// Adds a function. Returns `None` if the name is already taken (nothing is added then).
    pub(crate) fn add_func(
        &mut self,
        name: &str,
        params: &[Type],
        ret: Option<Type>,
        span: Option<Span>,
    ) -> Option<FuncId> {
        if self.symbols.contains_key(name) {
            return None;
        }
        let sig = self.intern_sig(params, ret);
        let id = FuncId(self.funcs.len() as u32);
        self.funcs.push(FuncData {
            name: name.to_string(),
            sig,
            body: None,
            span,
            trace_name: Some(name.to_string()),
        });
        self.items.push(Item::Func(id));
        self.symbols.insert(name.to_string(), Symbol::func(id));
        Some(id)
    }

    pub(crate) fn add_global(
        &mut self,
        name: &str,
        init: GlobalInit,
        span: Option<Span>,
    ) -> Option<GlobalId> {
        if self.symbols.contains_key(name) {
            return None;
        }
        let id = GlobalId(self.globals.len() as u32);
        self.globals.push(GlobalData {
            name: name.to_string(),
            init,
            span,
        });
        self.items.push(Item::Global(id));
        self.symbols.insert(name.to_string(), Symbol::global(id));
        Some(id)
    }

    /// Creates the body of `f`: its parameter values and an empty entry region.
    pub(crate) fn create_body(&mut self, f: FuncId) -> Region {
        let entry = self.new_region(f, Inst::NONE);
        let params: Vec<Type> = self.func_params(f).to_vec();
        let mut param_values = Vec::with_capacity(params.len());
        for (i, ty) in params.into_iter().enumerate() {
            param_values.push(self.new_value(f, ty, ValueDef::Param(i as u32)));
        }
        self.funcs[f.index()].body = Some(BodyData {
            params: param_values,
            vars: Vec::new(),
            entry,
        });
        entry
    }

    pub(crate) fn new_region(&mut self, func: FuncId, owner: Inst) -> Region {
        let r = Region(self.regions.len() as u32);
        self.regions.push(RegionData {
            insts: Vec::new(),
            func,
            owner,
        });
        r
    }

    pub(crate) fn new_value(&mut self, func: FuncId, ty: Type, def: ValueDef) -> Value {
        let v = Value(self.values.len() as u32);
        self.values.push(ValueData {
            ty,
            name: None,
            func,
            def,
        });
        v
    }

    pub(crate) fn new_var(&mut self, func: FuncId, ty: Type, name: Option<String>) -> Var {
        let v = Var(self.vars.len() as u32);
        self.vars.push(VarData {
            ty,
            name,
            func,
            debug: None,
        });
        if let Some(body) = self.funcs[func.index()].body.as_mut() {
            body.vars.push(v);
        }
        v
    }

    /// Appends an instruction to `region`. `result_ty` creates a result value when present.
    pub(crate) fn push_inst(
        &mut self,
        region: Region,
        opcode: Opcode,
        args: &[Value],
        imm: u64,
        result_ty: Option<Type>,
    ) -> Inst {
        let func = self.regions[region.index()].func;
        let inst = Inst(self.insts.len() as u32);
        let args_start = self.args.len() as u32;
        self.args.extend_from_slice(args);
        let result = match result_ty {
            Some(ty) => self.new_value(func, ty, ValueDef::Inst(inst)),
            None => Value::NONE,
        };
        self.insts.push(InstData {
            opcode,
            result,
            args_start,
            args_len: args.len() as u32,
            imm,
            regions: [Region::NONE, Region::NONE],
            func,
            parent: region,
            span: None,
            inline: InlineId::NONE,
        });
        self.regions[region.index()].insts.push(inst);
        inst
    }

    pub(crate) fn set_inst_region(&mut self, inst: Inst, n: usize, r: Region) {
        self.insts[inst.index()].regions[n] = r;
    }

    pub(crate) fn set_inst_span(&mut self, inst: Inst, span: Option<Span>) {
        self.insts[inst.index()].span = span;
    }

    pub(crate) fn set_inst_inline(&mut self, inst: Inst, inline: InlineId) {
        self.insts[inst.index()].inline = inline;
    }
}

/// Whether control can never fall off the end of `region` (see `docs/ir.md`, "termination").
pub fn region_terminates(m: &Module, region: Region) -> bool {
    m.region_insts(region)
        .iter()
        .any(|&i| inst_terminates(m, i))
}

fn inst_terminates(m: &Module, i: Inst) -> bool {
    match m.inst_opcode(i) {
        op if op.is_terminator() => true,
        Opcode::If => match (m.inst_region(i, 0), m.inst_region(i, 1)) {
            (Some(t), Some(e)) => region_terminates(m, t) && region_terminates(m, e),
            _ => false,
        },
        Opcode::Loop => m
            .inst_region(i, 0)
            .is_some_and(|body| !region_breaks(m, body, 0)),
        _ => false,
    }
}

/// Whether `region`, `depth` loops inside a loop, contains a `break` that exits that loop.
fn region_breaks(m: &Module, region: Region, depth: u32) -> bool {
    m.region_insts(region)
        .iter()
        .any(|&i| match m.inst_opcode(i) {
            Opcode::Break => m.inst_loop_depth(i) == depth,
            Opcode::If => (0..2).any(|n| {
                m.inst_region(i, n)
                    .is_some_and(|r| region_breaks(m, r, depth))
            }),
            Opcode::Loop => m
                .inst_region(i, 0)
                .is_some_and(|r| region_breaks(m, r, depth + 1)),
            _ => false,
        })
}
