//! Friendly construction of IR modules.
//!
//! The [`Builder`] owns the module while it is being built and keeps an insertion point. Control flow
//! is built with matched calls (`if_`/`else_`/`end_if`, `loop_`/`end_loop`) and mutable state uses
//! variables, so frontends never construct SSA by hand. Misuse is recorded as a diagnostic and
//! reported by [`Builder::finish`]; the builder never panics.

use std::collections::HashSet;

use a0_diag::{Diagnostic, Diagnostics, Span};

use crate::module::{GlobalInit, Module, region_terminates};
use crate::types::*;
use crate::verify::verify;

#[derive(Clone, Copy, PartialEq, Eq)]
enum OpenKind {
    Then,
    Else,
    Loop,
}

struct Open {
    kind: OpenKind,
    inst: Inst,
    /// The region to return to when this construct is closed.
    outer: Region,
}

/// Builds a [`Module`]. See the crate documentation for an example.
pub struct Builder {
    module: Module,
    func: FuncId,
    region: Region,
    open: Vec<Open>,
    value_names: HashSet<String>,
    var_names: HashSet<String>,
    span: Option<Span>,
    inline: InlineId,
    diags: Diagnostics,
}

impl Builder {
    pub fn new(module_name: &str) -> Builder {
        Builder::from_module(Module::new(module_name))
    }

    /// Continues building an existing module (for example one parsed from text).
    pub fn from_module(module: Module) -> Builder {
        Builder {
            module,
            func: FuncId::NONE,
            region: Region::NONE,
            open: Vec::new(),
            value_names: HashSet::new(),
            var_names: HashSet::new(),
            span: None,
            inline: InlineId::NONE,
            diags: Diagnostics::new(),
        }
    }

    /// Read access to the module built so far.
    pub fn module(&self) -> &Module {
        &self.module
    }

    fn error(&mut self, message: &str) {
        self.diags
            .push(Diagnostic::error("ir.builder", message).with_span_opt(self.span));
    }

    // ---------------------------------------------------------------- items

    /// Declares a function that is defined outside the module.
    pub fn declare_extern(&mut self, name: &str, params: &[Type], ret: Option<Type>) -> FuncId {
        self.add_func(name, params, ret)
    }

    /// Says that the `ptr` parameter `index` of the C function `f` stands for a C struct of
    /// `size` bytes aligned to `align`, whose scalar fields are `field_types` at `field_offsets`:
    /// passed by value (`ByValue`; the pointer says where its bytes are), or, for parameter 0,
    /// where the struct the function returns goes (`Result`; `f` returns nothing in the IR).
    /// Each backend passes it as the target's C ABI says (`docs/ir.md`, "C structs").
    #[allow(clippy::too_many_arguments)]
    pub fn set_struct_param(
        &mut self,
        f: FuncId,
        index: u32,
        pass: StructPass,
        size: u32,
        align: u32,
        field_types: &[Type],
        field_offsets: &[u32],
    ) {
        if field_types.len() != field_offsets.len() {
            self.error("set_struct_param: as many field offsets as field types are needed");
            return;
        }
        if f.is_none()
            || f.index() >= self.module.funcs.len()
            || index as usize >= self.module.func_params(f).len()
        {
            self.error("set_struct_param: no such function or parameter");
            return;
        }
        let fields = field_types
            .iter()
            .copied()
            .zip(field_offsets.iter().copied())
            .collect();
        self.module.struct_params.insert(
            (f, index),
            (
                pass,
                CStruct {
                    size,
                    align,
                    fields,
                },
            ),
        );
    }

    /// Declares a function to be defined in this module with `begin_function`. Until it has a body
    /// it is treated as extern; declaring first lets functions call each other in any order.
    pub fn declare_function(&mut self, name: &str, params: &[Type], ret: Option<Type>) -> FuncId {
        self.add_func(name, params, ret)
    }

    fn add_func(&mut self, name: &str, params: &[Type], ret: Option<Type>) -> FuncId {
        match self.module.add_func(name, params, ret, self.span) {
            Some(f) => f,
            None => {
                self.error(&format!("symbol @{name} is already defined"));
                // Keep going with the existing function if there is one, so later calls stay valid.
                self.module.lookup_func(name).unwrap_or(FuncId::NONE)
            }
        }
    }

    /// Adds read-only bytes.
    pub fn add_data(&mut self, name: &str, bytes: &[u8]) -> GlobalId {
        self.add_global_item(name, GlobalInit::Data(bytes.to_vec()))
    }

    /// Adds a mutable integer or pointer global. Pointer globals must be initialized to 0 (null).
    pub fn add_global_int(&mut self, name: &str, ty: Type, init: i64) -> GlobalId {
        if ty.is_float() {
            self.error("add_global_int needs an integer or pointer type");
        }
        self.add_global_item(name, GlobalInit::Scalar(ty, canonical_int(ty, init) as u64))
    }

    /// Adds a mutable float global.
    pub fn add_global_float(&mut self, name: &str, ty: Type, init: f64) -> GlobalId {
        let bits = match ty {
            Type::F32 => (init as f32).to_bits() as u64,
            Type::F64 => init.to_bits(),
            _ => {
                self.error("add_global_float needs a float type");
                0
            }
        };
        self.add_global_item(name, GlobalInit::Scalar(ty, bits))
    }

    fn add_global_item(&mut self, name: &str, init: GlobalInit) -> GlobalId {
        match self.module.add_global(name, init, self.span) {
            Some(g) => g,
            None => {
                self.error(&format!("symbol @{name} is already defined"));
                self.module.lookup_global(name).unwrap_or(GlobalId::NONE)
            }
        }
    }

    // ---------------------------------------------------------------- functions

    /// Starts the body of a declared function and moves the insertion point into it.
    pub fn begin_function(&mut self, f: FuncId) {
        if self.func.is_some() {
            self.error("begin_function called while another function is open");
            return;
        }
        if f.is_none() || f.index() >= self.module.funcs.len() {
            self.error("begin_function called with an invalid function");
            return;
        }
        if !self.module.func_is_extern(f) {
            let name = self.module.func_name(f).to_string();
            self.error(&format!("function @{name} already has a body"));
            return;
        }
        self.func = f;
        self.region = self.module.create_body(f);
        self.open.clear();
        self.value_names.clear();
        self.var_names.clear();
    }

    /// Finishes the current function. Unclosed `if`/`loop` constructs are reported.
    pub fn end_function(&mut self) {
        if self.func.is_none() {
            self.error("end_function called without an open function");
            return;
        }
        if !self.open.is_empty() {
            let name = self.module.func_name(self.func).to_string();
            self.error(&format!("function @{name} ended with unclosed if/loop"));
        }
        self.func = FuncId::NONE;
        self.region = Region::NONE;
        self.open.clear();
    }

    /// The function being built, if any.
    pub fn current_function(&self) -> Option<FuncId> {
        self.func.option()
    }

    /// The `i`th parameter of the current function.
    pub fn param(&mut self, i: u32) -> Value {
        if self.func.is_none() {
            self.error("param called outside a function");
            return Value::NONE;
        }
        match self.module.func_param_values(self.func).get(i as usize) {
            Some(&v) => v,
            None => {
                self.error(&format!("parameter index {i} out of range"));
                Value::NONE
            }
        }
    }

    /// Sets the span attached to instructions built from now on (useful for diagnostics).
    pub fn set_span(&mut self, span: Option<Span>) {
        self.span = span;
    }

    /// The span and inline site instructions get now, to restore with [`Builder::set_location`].
    pub fn location(&self) -> (Option<Span>, InlineId) {
        (self.span, self.inline)
    }

    pub fn set_location(&mut self, (span, inline): (Option<Span>, InlineId)) {
        self.span = span;
        self.inline = inline;
    }

    /// Marks instructions built from now on as coming from the inline site `site` (or from no
    /// inlined function), for stack traces.
    pub fn set_inline(&mut self, site: Option<InlineId>) {
        self.inline = site.unwrap_or(InlineId::NONE);
    }

    /// Records that `callee` was inlined at `call`, inside code itself inlined at `parent`.
    pub fn add_inline_site(
        &mut self,
        callee: &str,
        call: Option<Span>,
        parent: Option<InlineId>,
    ) -> InlineId {
        self.module.inlines.push(crate::InlineSite {
            callee: callee.to_string(),
            call,
            parent,
        });
        InlineId(self.module.inlines.len() as u32 - 1)
    }

    /// What stack traces call `f` (its IR name by default).
    pub fn set_trace_name(&mut self, f: FuncId, name: &str) {
        if let Some(func) = self.module.funcs.get_mut(f.index()) {
            func.trace_name = Some(name.to_string());
        }
    }

    /// Leaves `f` out of stack traces (an entry point or wrapper that is no source-level function).
    pub fn hide_from_traces(&mut self, f: FuncId) {
        if let Some(func) = self.module.funcs.get_mut(f.index()) {
            func.trace_name = None;
        }
    }

    // ---------------------------------------------------------------- names

    /// Gives `v` a readable name. Names are made unique within the function (`x`, `x.1`, ...).
    pub fn name_value(&mut self, v: Value, name: &str) {
        if v.is_none() || v.index() >= self.module.values.len() {
            self.error("name_value called with an invalid value");
            return;
        }
        if let Some(old) = self.module.values[v.index()].name.take() {
            self.value_names.remove(&old);
        }
        let unique = unique_name(&mut self.value_names, name);
        self.module.values[v.index()].name = Some(unique);
    }

    // ---------------------------------------------------------------- variables

    /// Declares a mutable variable in the current function. It starts out as zero.
    /// A source-level type for debuggers, of kind `kind`, written `name`, with element types
    /// `elems` (see [`DebugKind`]). Struct fields and enum variants are added afterwards, so that
    /// types can refer to themselves.
    pub fn debug_type(&mut self, kind: DebugKind, name: &str, elems: &[DebugType]) -> DebugType {
        if elems
            .iter()
            .any(|e| e.index() >= self.module.debug_types.len())
        {
            self.error("debug_type: an element type does not exist");
            return DebugType::NONE;
        }
        let t = DebugType(self.module.debug_types.len() as u32);
        self.module.debug_types.push(crate::module::DebugTypeData {
            kind,
            name: name.to_string(),
            elems: elems.to_vec(),
            fields: Vec::new(),
            variants: Vec::new(),
        });
        t
    }

    /// Adds a field to a struct's debug type.
    pub fn debug_type_add_field(&mut self, t: DebugType, name: &str, ty: DebugType) {
        if !self.debug_type_exists(t) || !self.debug_type_exists(ty) {
            self.error("debug_type_add_field: no such debug type");
            return;
        }
        self.module.debug_types[t.index()]
            .fields
            .push((name.to_string(), ty, None));
    }

    /// Adds a field at `offset` bytes, of `size` bytes, to a struct's or an inline struct's debug
    /// type (fields added with `debug_type_add_field` are the words in order).
    pub fn debug_type_add_field_at(
        &mut self,
        t: DebugType,
        name: &str,
        ty: DebugType,
        offset: u32,
        size: u32,
    ) {
        if !self.debug_type_exists(t) || !self.debug_type_exists(ty) {
            self.error("debug_type_add_field_at: no such debug type");
            return;
        }
        self.module.debug_types[t.index()].fields.push((
            name.to_string(),
            ty,
            Some((offset, size)),
        ));
    }

    /// Adds a variant, with the types of its fields, to an enum's debug type.
    pub fn debug_type_add_variant(&mut self, t: DebugType, name: &str, fields: &[DebugType]) {
        if !self.debug_type_exists(t) || !fields.iter().all(|&f| self.debug_type_exists(f)) {
            self.error("debug_type_add_variant: no such debug type");
            return;
        }
        self.module.debug_types[t.index()]
            .variants
            .push((name.to_string(), fields.to_vec()));
    }

    /// Says that `var` holds the source-level variable `name` of type `ty`, declared at `decl`,
    /// for debuggers.
    pub fn set_var_debug(&mut self, var: Var, name: &str, ty: DebugType, decl: Option<Span>) {
        if var.index() >= self.module.vars.len() || !self.debug_type_exists(ty) {
            self.error("set_var_debug: no such variable or debug type");
            return;
        }
        self.module.vars[var.index()].debug = Some((name.to_string(), ty, decl));
    }

    fn debug_type_exists(&self, t: DebugType) -> bool {
        t.index() < self.module.debug_types.len()
    }

    pub fn declare_var(&mut self, name: &str, ty: Type) -> Var {
        if self.func.is_none() {
            self.error("declare_var called outside a function");
            return Var::NONE;
        }
        let name = if name.is_empty() {
            None
        } else {
            Some(unique_name(&mut self.var_names, name))
        };
        self.module.new_var(self.func, ty, name)
    }

    /// Reads a variable.
    pub fn get(&mut self, var: Var) -> Value {
        let ty = self.var_ty(var);
        self.inst(Opcode::Get, &[], var.0 as u64, Some(ty))
    }

    /// Writes a variable.
    pub fn set(&mut self, var: Var, value: Value) {
        self.var_ty(var);
        self.inst(Opcode::Set, &[value], var.0 as u64, None);
    }

    fn var_ty(&mut self, var: Var) -> Type {
        if var.is_none() || var.index() >= self.module.vars.len() {
            self.error("invalid variable");
            return Type::I64;
        }
        self.module.var_type(var)
    }

    // ---------------------------------------------------------------- constants

    pub fn iconst(&mut self, ty: Type, value: i64) -> Value {
        self.inst(
            Opcode::Iconst,
            &[],
            canonical_int(ty, value) as u64,
            Some(ty),
        )
    }

    pub fn fconst(&mut self, ty: Type, value: f64) -> Value {
        let bits = match ty {
            Type::F32 => (value as f32).to_bits() as u64,
            _ => value.to_bits(),
        };
        self.inst(Opcode::Fconst, &[], bits, Some(ty))
    }

    /// Changes the value of an `iconst` built earlier. Useful for numbers that are only known after
    /// more code has been built, such as a frame size emitted in a function's prologue.
    pub fn patch_iconst(&mut self, constant: Value, value: i64) {
        let inst = match self.module.values.get(constant.index()) {
            Some(_) => self.module.value_inst(constant),
            None => None,
        };
        match inst {
            Some(i) if self.module.inst_opcode(i) == Opcode::Iconst => {
                let ty = self.module.value_type(constant);
                self.module.insts[i.index()].imm = canonical_int(ty, value) as u64;
            }
            _ => self.error("patch_iconst needs a value produced by iconst"),
        }
    }

    /// A float constant given by its exact bit pattern (f32 uses the low 32 bits).
    pub fn fconst_bits(&mut self, ty: Type, bits: u64) -> Value {
        self.inst(Opcode::Fconst, &[], bits, Some(ty))
    }

    pub fn null(&mut self) -> Value {
        self.inst(Opcode::Null, &[], 0, Some(Type::Ptr))
    }

    /// The address of `size` bytes in the function's frame, aligned to `align` (a power of two,
    /// at most 16): the same for every execution in one call of the function, gone when it
    /// returns. Its contents start undefined.
    pub fn stack_slot(&mut self, size: u32, align: u32) -> Value {
        if !align.is_power_of_two() || align > 16 {
            self.error(&format!(
                "stack_slot alignment {align} is not a power of two up to 16"
            ));
        }
        let imm = size as u64 | (align as u64) << 32;
        self.inst(Opcode::StackSlot, &[], imm, Some(Type::Ptr))
    }

    /// The address of a function.
    pub fn func_addr(&mut self, f: FuncId) -> Value {
        self.inst(Opcode::Addr, &[], Symbol::func(f).to_imm(), Some(Type::Ptr))
    }

    /// The address of a `data` or `global` item.
    pub fn global_addr(&mut self, g: GlobalId) -> Value {
        self.inst(
            Opcode::Addr,
            &[],
            Symbol::global(g).to_imm(),
            Some(Type::Ptr),
        )
    }

    // ---------------------------------------------------------------- arithmetic

    /// Any two-operand integer or float opcode (`iadd`, `fmul`, `shl`, ...). The result has the
    /// type of `a`.
    pub fn binary(&mut self, op: Opcode, a: Value, b: Value) -> Value {
        if !op.is_int_binary() && !op.is_float_binary() {
            self.error(&format!("{op} is not a binary arithmetic opcode"));
        }
        let ty = self.ty(a);
        self.inst(op, &[a, b], 0, Some(ty))
    }

    pub fn iadd(&mut self, a: Value, b: Value) -> Value {
        self.binary(Opcode::Iadd, a, b)
    }

    pub fn isub(&mut self, a: Value, b: Value) -> Value {
        self.binary(Opcode::Isub, a, b)
    }

    pub fn imul(&mut self, a: Value, b: Value) -> Value {
        self.binary(Opcode::Imul, a, b)
    }

    pub fn sdiv(&mut self, a: Value, b: Value) -> Value {
        self.binary(Opcode::Sdiv, a, b)
    }

    pub fn srem(&mut self, a: Value, b: Value) -> Value {
        self.binary(Opcode::Srem, a, b)
    }

    pub fn and(&mut self, a: Value, b: Value) -> Value {
        self.binary(Opcode::And, a, b)
    }

    pub fn or(&mut self, a: Value, b: Value) -> Value {
        self.binary(Opcode::Or, a, b)
    }

    pub fn xor(&mut self, a: Value, b: Value) -> Value {
        self.binary(Opcode::Xor, a, b)
    }

    pub fn fadd(&mut self, a: Value, b: Value) -> Value {
        self.binary(Opcode::Fadd, a, b)
    }

    pub fn fsub(&mut self, a: Value, b: Value) -> Value {
        self.binary(Opcode::Fsub, a, b)
    }

    pub fn fmul(&mut self, a: Value, b: Value) -> Value {
        self.binary(Opcode::Fmul, a, b)
    }

    pub fn fdiv(&mut self, a: Value, b: Value) -> Value {
        self.binary(Opcode::Fdiv, a, b)
    }

    /// `ineg`, `not` or `fneg`.
    pub fn unary(&mut self, op: Opcode, a: Value) -> Value {
        if !op.is_unary() {
            self.error(&format!("{op} is not a unary opcode"));
        }
        let ty = self.ty(a);
        self.inst(op, &[a], 0, Some(ty))
    }

    pub fn ineg(&mut self, a: Value) -> Value {
        self.unary(Opcode::Ineg, a)
    }

    pub fn not(&mut self, a: Value) -> Value {
        self.unary(Opcode::Not, a)
    }

    pub fn fneg(&mut self, a: Value) -> Value {
        self.unary(Opcode::Fneg, a)
    }

    pub fn icmp(&mut self, cc: IntCC, a: Value, b: Value) -> Value {
        self.inst(Opcode::Icmp, &[a, b], cc as u64, Some(Type::I1))
    }

    pub fn fcmp(&mut self, cc: FloatCC, a: Value, b: Value) -> Value {
        self.inst(Opcode::Fcmp, &[a, b], cc as u64, Some(Type::I1))
    }

    /// Any conversion opcode (`zext`, `sitofp`, `bitcast`, ...) to type `to`.
    pub fn cast(&mut self, op: Opcode, a: Value, to: Type) -> Value {
        if !op.is_cast() {
            self.error(&format!("{op} is not a conversion opcode"));
        }
        self.inst(op, &[a], 0, Some(to))
    }

    pub fn select(&mut self, cond: Value, a: Value, b: Value) -> Value {
        let ty = self.ty(a);
        self.inst(Opcode::Select, &[cond, a, b], 0, Some(ty))
    }

    // ---------------------------------------------------------------- memory

    pub fn load(&mut self, ty: Type, ptr: Value) -> Value {
        self.inst(Opcode::Load, &[ptr], 0, Some(ty))
    }

    pub fn store(&mut self, value: Value, ptr: Value) {
        self.inst(Opcode::Store, &[value, ptr], 0, None);
    }

    /// `ptr + offset` bytes, with `offset: i64`.
    pub fn ptradd(&mut self, ptr: Value, offset: Value) -> Value {
        self.inst(Opcode::Ptradd, &[ptr, offset], 0, Some(Type::Ptr))
    }

    // ---------------------------------------------------------------- calls

    /// Calls a function directly. Returns its result, or `None` for void functions.
    pub fn call(&mut self, f: FuncId, args: &[Value]) -> Option<Value> {
        if f.is_none() || f.index() >= self.module.funcs.len() {
            self.error("call to an invalid function");
            return None;
        }
        let ret = self.module.func_ret(f);
        let inst = self.inst_raw(Opcode::Call, args, f.0 as u64, ret);
        inst.and_then(|i| self.module.inst_result(i))
    }

    /// Calls through a function pointer with the given signature.
    pub fn call_indirect(
        &mut self,
        callee: Value,
        params: &[Type],
        ret: Option<Type>,
        args: &[Value],
    ) -> Option<Value> {
        let sig = self.module.intern_sig(params, ret);
        let mut all = Vec::with_capacity(args.len() + 1);
        all.push(callee);
        all.extend_from_slice(args);
        let inst = self.inst_raw(Opcode::CallIndirect, &all, sig.0 as u64, ret);
        inst.and_then(|i| self.module.inst_result(i))
    }

    // ---------------------------------------------------------------- control flow

    /// Starts `if cond { ... }`; the insertion point moves into the then-region.
    pub fn if_(&mut self, cond: Value) {
        let Some(inst) = self.inst_raw(Opcode::If, &[cond], 0, None) else {
            return;
        };
        let then = self.module.new_region(self.func, inst);
        self.module.set_inst_region(inst, 0, then);
        self.open.push(Open {
            kind: OpenKind::Then,
            inst,
            outer: self.region,
        });
        self.region = then;
    }

    /// Switches from the then-region to a new else-region.
    pub fn else_(&mut self) {
        match self.open.last_mut() {
            Some(open) if open.kind == OpenKind::Then => {
                let inst = open.inst;
                open.kind = OpenKind::Else;
                let els = self.module.new_region(self.func, inst);
                self.module.set_inst_region(inst, 1, els);
                self.region = els;
            }
            _ => self.error("else_ without a matching if_"),
        }
    }

    /// Closes the innermost `if`.
    pub fn end_if(&mut self) {
        match self.open.last() {
            Some(open) if matches!(open.kind, OpenKind::Then | OpenKind::Else) => {
                self.region = open.outer;
                self.open.pop();
            }
            _ => self.error("end_if without a matching if_"),
        }
    }

    /// Starts `loop { ... }`; the insertion point moves into the body.
    pub fn loop_(&mut self) {
        let Some(inst) = self.inst_raw(Opcode::Loop, &[], 0, None) else {
            return;
        };
        let body = self.module.new_region(self.func, inst);
        self.module.set_inst_region(inst, 0, body);
        self.open.push(Open {
            kind: OpenKind::Loop,
            inst,
            outer: self.region,
        });
        self.region = body;
    }

    /// Closes the innermost `loop`.
    pub fn end_loop(&mut self) {
        match self.open.last() {
            Some(open) if open.kind == OpenKind::Loop => {
                self.region = open.outer;
                self.open.pop();
            }
            _ => self.error("end_loop without a matching loop_"),
        }
    }

    /// Exits the innermost loop.
    pub fn break_(&mut self) {
        self.break_out(0);
    }

    /// Jumps to the start of the innermost loop.
    pub fn continue_(&mut self) {
        self.continue_out(0);
    }

    /// Exits the loop `depth` loops out from the innermost (0 is the innermost), and every loop
    /// inside it.
    pub fn break_out(&mut self, depth: u32) {
        match self.loop_count() {
            0 => self.error("break_ outside a loop"),
            n if depth as usize >= n => self.error("break_out of more loops than are open"),
            _ => {}
        }
        self.inst(Opcode::Break, &[], depth as u64, None);
    }

    /// Jumps to the start of the loop `depth` loops out from the innermost.
    pub fn continue_out(&mut self, depth: u32) {
        match self.loop_count() {
            0 => self.error("continue_ outside a loop"),
            n if depth as usize >= n => self.error("continue_out of more loops than are open"),
            _ => {}
        }
        self.inst(Opcode::Continue, &[], depth as u64, None);
    }

    /// Returns from the current function.
    pub fn ret(&mut self, value: Option<Value>) {
        match value {
            Some(v) => self.inst(Opcode::Return, &[v], 0, None),
            None => self.inst(Opcode::Return, &[], 0, None),
        };
    }

    /// Traps.
    pub fn unreachable(&mut self) {
        self.inst(Opcode::Unreachable, &[], 0, None);
    }

    /// Whether the current region can no longer fall through (it already returned, broke, ...).
    /// Frontends use this to avoid emitting dead code or a redundant final `return`.
    pub fn is_terminated(&self) -> bool {
        self.region.is_some() && region_terminates(&self.module, self.region)
    }

    fn loop_count(&self) -> usize {
        self.open
            .iter()
            .filter(|o| o.kind == OpenKind::Loop)
            .count()
    }

    // ---------------------------------------------------------------- finishing

    /// Returns the module if it was built correctly and passes the verifier.
    pub fn finish(mut self) -> Result<Module, Diagnostics> {
        if self.func.is_some() {
            self.end_function();
        }
        let mut diags = self.diags;
        if !diags.has_errors() {
            diags.extend(verify(&self.module));
        }
        if diags.has_errors() {
            Err(diags)
        } else {
            Ok(self.module)
        }
    }

    /// Returns the module without verifying it, together with any builder diagnostics.
    pub fn finish_unverified(self) -> (Module, Diagnostics) {
        (self.module, self.diags)
    }

    // ---------------------------------------------------------------- internals

    fn ty(&mut self, v: Value) -> Type {
        if v.is_none() || v.index() >= self.module.values.len() {
            self.error("invalid value used as operand");
            return Type::I64;
        }
        self.module.value_type(v)
    }

    fn inst(&mut self, op: Opcode, args: &[Value], imm: u64, ty: Option<Type>) -> Value {
        self.inst_raw(op, args, imm, ty)
            .and_then(|i| self.module.inst_result(i))
            .unwrap_or(Value::NONE)
    }

    fn inst_raw(&mut self, op: Opcode, args: &[Value], imm: u64, ty: Option<Type>) -> Option<Inst> {
        if self.region.is_none() {
            self.error(&format!("{op} built outside a function"));
            return None;
        }
        if args
            .iter()
            .any(|a| a.is_none() || a.index() >= self.module.values.len())
        {
            self.error(&format!("{op} has an invalid operand"));
            return None;
        }
        let inst = self.module.push_inst(self.region, op, args, imm, ty);
        self.module.set_inst_span(inst, self.span);
        self.module.set_inst_inline(inst, self.inline);
        Some(inst)
    }
}

fn unique_name(taken: &mut HashSet<String>, base: &str) -> String {
    let mut name = base.to_string();
    let mut n = 1;
    while taken.contains(&name) {
        name = format!("{base}.{n}");
        n += 1;
    }
    taken.insert(name.clone());
    name
}
