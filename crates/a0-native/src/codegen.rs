//! Lowering from A0 IR to Cranelift, producing an object file.

use std::collections::HashMap;

use a0_diag::{Diagnostic, Diagnostics};
use a0_ir::{FloatCC, FuncId, GlobalId, Inst, IntCC, Module, Opcode, Region, SymbolKind, Type};
use a0_target::Target;
use cranelift_codegen::ir::{self, InstBuilder, MemFlagsData, TrapCode, condcodes, types};
use cranelift_codegen::isa::OwnedTargetIsa;
use cranelift_codegen::settings::{self, Configurable};
use cranelift_codegen::Context;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_module::{DataDescription, DataId, Linkage, Module as _};
use cranelift_object::{ObjectBuilder, ObjectModule};

/// Symbol under which the IR's `@main` is emitted when an executable entry point is generated.
pub const ENTRY_SYMBOL: &str = "a0.main";

/// Options for object emission.
#[derive(Clone, Debug)]
pub struct CodegenOptions {
    /// Let Cranelift optimize (`opt_level = speed`).
    pub optimize: bool,
    /// Emit a C `main` that initializes the runtime and calls the IR's `@main`. The IR `@main` is then
    /// emitted as [`ENTRY_SYMBOL`].
    pub entry_point: bool,
}

impl Default for CodegenOptions {
    fn default() -> Self {
        CodegenOptions {
            optimize: true,
            entry_point: false,
        }
    }
}

fn backend_error(message: String) -> Diagnostics {
    Diagnostic::error("native.codegen", &message).into()
}

fn make_isa(target: Target, optimize: bool) -> Result<OwnedTargetIsa, Diagnostics> {
    let mut flags = settings::builder();
    let set = |flags: &mut settings::Builder, k: &str, v: &str| {
        flags
            .set(k, v)
            .map_err(|e| backend_error(format!("cannot set Cranelift flag {k}={v}: {e}")))
    };
    set(&mut flags, "opt_level", if optimize { "speed" } else { "none" })?;
    set(&mut flags, "is_pic", if target.requires_pic() { "true" } else { "false" })?;
    let isa = cranelift_codegen::isa::lookup_by_name(target.triple())
        .map_err(|e| backend_error(format!("Cranelift does not support {target}: {e}")))?;
    isa.finish(settings::Flags::new(flags))
        .map_err(|e| backend_error(format!("cannot configure Cranelift for {target}: {e}")))
}

/// The Cranelift type used for an IR type. `i1` is an `i8` holding 0 or 1; `ptr` is the target's
/// pointer type.
fn clif_type(ty: Type, target: Target) -> ir::Type {
    match ty {
        Type::I1 | Type::I8 => types::I8,
        Type::I16 => types::I16,
        Type::I32 => types::I32,
        Type::I64 => types::I64,
        Type::F32 => types::F32,
        Type::F64 => types::F64,
        Type::Ptr => ir::Type::int(target.pointer_bytes() as u16 * 8).unwrap(),
    }
}

/// Targets that Cranelift cannot emit objects for yet, with the reason.
fn unsupported_reason(target: Target) -> Option<&'static str> {
    if target == Target::AARCH64_WINDOWS {
        return Some(
            "cranelift-object cannot write AArch64 relocations into COFF objects, so aarch64-windows              objects cannot be produced yet",
        );
    }
    None
}

/// Compiles a verified module into an object file for `target`.
pub fn compile_object(m: &Module, target: Target, opts: &CodegenOptions) -> Result<Vec<u8>, Diagnostics> {
    if let Some(reason) = unsupported_reason(target) {
        return Err(backend_error(format!("cannot compile for {target}: {reason}")));
    }
    // The module is verified first, so Cranelift should never panic; if it does anyway, report it as
    // a diagnostic rather than taking the caller down.
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| compile_object_inner(m, target, opts))) {
        Ok(result) => result,
        Err(payload) => {
            let detail = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "unknown error".to_string());
            Err(backend_error(format!("internal code generator error for {target}: {detail}")))
        }
    }
}

fn compile_object_inner(m: &Module, target: Target, opts: &CodegenOptions) -> Result<Vec<u8>, Diagnostics> {
    let diags = a0_ir::verify(m);
    if diags.has_errors() {
        return Err(diags);
    }
    if opts.entry_point {
        let diags = a0_ir::check_entry(m);
        if diags.has_errors() {
            return Err(diags);
        }
    }
    let isa = make_isa(target, opts.optimize)?;
    let builder = ObjectBuilder::new(isa, m.name().as_bytes().to_vec(), cranelift_module::default_libcall_names())
        .map_err(|e| backend_error(format!("cannot create an object for {target}: {e}")))?;
    let mut obj = ObjectModule::new(builder);
    let ptr = clif_type(Type::Ptr, target);

    let symbol_name = |f: FuncId| -> String {
        let name = m.func_name(f);
        if opts.entry_point && name == "main" {
            ENTRY_SYMBOL.to_string()
        } else {
            name.to_string()
        }
    };

    // Declare every function and global first so bodies can refer to any of them.
    let mut func_ids = Vec::new();
    for f in 0..m.func_count() {
        let f = FuncId(f);
        let sig = signature(&obj, m.func_params(f), m.func_ret(f), target);
        let linkage = if m.func_is_extern(f) {
            Linkage::Import
        } else {
            Linkage::Export
        };
        let id = obj
            .declare_function(&symbol_name(f), linkage, &sig)
            .map_err(|e| backend_error(format!("cannot declare @{}: {e}", m.func_name(f))))?;
        func_ids.push(id);
    }
    let mut data_ids = Vec::new();
    for g in 0..m.global_count() {
        let g = GlobalId(g);
        let id = define_global(&mut obj, m, g, target)?;
        data_ids.push(id);
    }
    let rt = RuntimeFuncs::declare(&mut obj, target)?;

    let frontend_config = obj.target_config();
    let mut ctx = obj.make_context();
    let mut fctx = FunctionBuilderContext::new();
    for f in 0..m.func_count() {
        let f = FuncId(f);
        if m.func_is_extern(f) {
            continue;
        }
        ctx.func.signature = signature(&obj, m.func_params(f), m.func_ret(f), target);
        {
            let builder = FunctionBuilder::new(&mut ctx.func, &mut fctx);
            let mut lower = Lower {
                m,
                f,
                target,
                ptr,
                obj: &mut obj,
                b: builder,
                func_ids: &func_ids,
                data_ids: &data_ids,
                rt: &rt,
                values: HashMap::new(),
                vars: HashMap::new(),
                func_refs: HashMap::new(),
                trap_blocks: HashMap::new(),
                loops: Vec::new(),
            };
            lower.function();
            lower.b.finalize(frontend_config);
        }
        define(&mut obj, func_ids[f.index()], &mut ctx, m.func_name(f))?;
        obj.clear_context(&mut ctx);
    }

    if opts.entry_point {
        let main = m.lookup_func("main").unwrap();
        emit_c_main(&mut obj, &mut ctx, &mut fctx, func_ids[main.index()], &rt, target)?;
    }

    let product = obj.finish();
    product
        .emit()
        .map_err(|e| backend_error(format!("cannot write the object file: {e}")))
}

fn define(obj: &mut ObjectModule, id: cranelift_module::FuncId, ctx: &mut Context, name: &str) -> Result<(), Diagnostics> {
    obj.define_function(id, ctx).map_err(|e| {
        let detail = match e {
            cranelift_module::ModuleError::Compilation(err) => {
                cranelift_codegen::print_errors::pretty_error(&ctx.func, err)
            }
            other => other.to_string(),
        };
        backend_error(format!("Cranelift failed to compile @{name}: {detail}"))
    })
}

fn signature(obj: &ObjectModule, params: &[Type], ret: Option<Type>, target: Target) -> ir::Signature {
    let mut sig = obj.make_signature();
    for &p in params {
        sig.params.push(ir::AbiParam::new(clif_type(p, target)));
    }
    if let Some(r) = ret {
        sig.returns.push(ir::AbiParam::new(clif_type(r, target)));
    }
    sig
}

fn define_global(obj: &mut ObjectModule, m: &Module, g: GlobalId, target: Target) -> Result<DataId, Diagnostics> {
    let name = m.global_name(g);
    let writable = !m.global_is_data(g);
    let id = obj
        .declare_data(name, Linkage::Export, writable, false)
        .map_err(|e| backend_error(format!("cannot declare @{name}: {e}")))?;
    let mut desc = DataDescription::new();
    match m.global_type(g) {
        None => {
            let bytes = m.global_bytes(g);
            // Zero-sized data still needs an address.
            let contents = if bytes.is_empty() { vec![0u8] } else { bytes.to_vec() };
            desc.define(contents.into_boxed_slice());
            desc.set_align(1);
        }
        Some(ty) => {
            let size = clif_type(ty, target).bytes() as usize;
            let bits = m.global_init_bits(g);
            let mut contents = bits.to_le_bytes()[..size].to_vec();
            if target_is_big_endian(target) {
                contents.reverse();
            }
            desc.define(contents.into_boxed_slice());
            desc.set_align(size as u64);
        }
    }
    obj.define_data(id, &desc)
        .map_err(|e| backend_error(format!("cannot define @{name}: {e}")))?;
    Ok(id)
}

fn target_is_big_endian(_target: Target) -> bool {
    // Every supported target is little-endian.
    false
}

/// Runtime functions the generated code calls.
struct RuntimeFuncs {
    trap: cranelift_module::FuncId,
    init: cranelift_module::FuncId,
    exit: cranelift_module::FuncId,
}

impl RuntimeFuncs {
    fn declare(obj: &mut ObjectModule, target: Target) -> Result<RuntimeFuncs, Diagnostics> {
        let mut decl = |name: &str, params: &[Type]| {
            let sig = signature(obj, params, None, target);
            obj.declare_function(name, Linkage::Import, &sig)
                .map_err(|e| backend_error(format!("cannot declare runtime function {name}: {e}")))
        };
        Ok(RuntimeFuncs {
            trap: decl("a0_rt_trap", &[Type::I32])?,
            init: decl("a0_rt_init", &[Type::I32, Type::Ptr])?,
            exit: decl("a0_rt_exit", &[Type::I32])?,
        })
    }
}

/// `int main(int argc, char **argv) { a0_rt_init(argc, argv); a0_rt_exit(a0.main()); }`
fn emit_c_main(
    obj: &mut ObjectModule,
    ctx: &mut Context,
    fctx: &mut FunctionBuilderContext,
    user_main: cranelift_module::FuncId,
    rt: &RuntimeFuncs,
    target: Target,
) -> Result<(), Diagnostics> {
    let sig = signature(obj, &[Type::I32, Type::Ptr], Some(Type::I32), target);
    let id = obj
        .declare_function("main", Linkage::Export, &sig)
        .map_err(|e| backend_error(format!("cannot declare main: {e}")))?;
    ctx.func.signature = sig;
    let frontend_config = obj.target_config();
    {
        let mut b = FunctionBuilder::new(&mut ctx.func, fctx);
        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        b.seal_block(entry);
        let argc = b.block_params(entry)[0];
        let argv = b.block_params(entry)[1];
        let init = obj.declare_func_in_func(rt.init, b.func);
        b.ins().call(init, &[argc, argv]);
        let um = obj.declare_func_in_func(user_main, b.func);
        let call = b.ins().call(um, &[]);
        let code = b.inst_results(call)[0];
        let exit = obj.declare_func_in_func(rt.exit, b.func);
        b.ins().call(exit, &[code]);
        b.ins().return_(&[code]);
        b.finalize(frontend_config);
    }
    define(obj, id, ctx, "main")?;
    obj.clear_context(ctx);
    Ok(())
}

struct LoopBlocks {
    header: ir::Block,
    exit: ir::Block,
    exit_used: bool,
}

struct Lower<'a> {
    m: &'a Module,
    f: FuncId,
    target: Target,
    ptr: ir::Type,
    obj: &'a mut ObjectModule,
    b: FunctionBuilder<'a>,
    func_ids: &'a [cranelift_module::FuncId],
    data_ids: &'a [DataId],
    rt: &'a RuntimeFuncs,
    values: HashMap<a0_ir::Value, ir::Value>,
    vars: HashMap<a0_ir::Var, Variable>,
    func_refs: HashMap<cranelift_module::FuncId, ir::FuncRef>,
    trap_blocks: HashMap<i32, ir::Block>,
    loops: Vec<LoopBlocks>,
}

impl Lower<'_> {
    fn ty(&self, t: Type) -> ir::Type {
        clif_type(t, self.target)
    }

    fn v(&self, v: a0_ir::Value) -> ir::Value {
        self.values[&v]
    }

    fn func_ref(&mut self, id: cranelift_module::FuncId) -> ir::FuncRef {
        if let Some(&r) = self.func_refs.get(&id) {
            return r;
        }
        let r = self.obj.declare_func_in_func(id, self.b.func);
        self.func_refs.insert(id, r);
        r
    }

    fn function(&mut self) {
        let m = self.m;
        let entry = self.b.create_block();
        self.b.append_block_params_for_function_params(entry);
        self.b.switch_to_block(entry);
        self.b.seal_block(entry);
        let params = self.b.block_params(entry).to_vec();
        for (&p, v) in m.func_param_values(self.f).iter().zip(params) {
            self.values.insert(p, v);
        }
        // Variables start out as zero.
        for &var in m.func_vars(self.f) {
            let ty = self.ty(m.var_type(var));
            let cv = self.b.declare_var(ty);
            let zero = self.zero(ty);
            self.b.def_var(cv, zero);
            self.vars.insert(var, cv);
        }
        let body = m.func_body(self.f).unwrap();
        if self.region(body) {
            // Only void functions can fall off the end (the verifier guarantees it).
            self.b.ins().return_(&[]);
        }
        self.finish_trap_blocks();
    }

    fn zero(&mut self, ty: ir::Type) -> ir::Value {
        if ty == types::F32 {
            self.b.ins().f32const(0.0)
        } else if ty == types::F64 {
            self.b.ins().f64const(0.0)
        } else {
            self.iconst(ty, 0)
        }
    }

    /// Lowers a region; returns whether control can fall off its end.
    fn region(&mut self, r: Region) -> bool {
        for &inst in self.m.region_insts(r) {
            if !self.inst(inst) {
                // Everything after a terminator in this region is dead and cannot be referenced from
                // outside it, so it is simply not emitted.
                return false;
            }
        }
        true
    }

    /// A block that reports trap `code` through the runtime. Shared by all checks in the function and
    /// filled in by `finish_trap_blocks` once the body is done.
    fn trap_block(&mut self, code: i32) -> ir::Block {
        if let Some(&b) = self.trap_blocks.get(&code) {
            return b;
        }
        let block = self.b.create_block();
        self.b.set_cold_block(block);
        self.trap_blocks.insert(code, block);
        block
    }

    fn finish_trap_blocks(&mut self) {
        let mut blocks: Vec<(i32, ir::Block)> = self.trap_blocks.iter().map(|(&c, &b)| (c, b)).collect();
        blocks.sort();
        for (code, block) in blocks {
            self.b.switch_to_block(block);
            self.b.seal_block(block);
            self.terminate_with_trap(code);
        }
    }

    /// `iconst` with the immediate truncated to the type's width, as Cranelift expects.
    fn iconst(&mut self, ty: ir::Type, value: i64) -> ir::Value {
        let bits = ty.bits();
        let masked = if bits >= 64 { value } else { value & ((1i64 << bits) - 1) };
        self.b.ins().iconst(ty, masked)
    }

    /// Branches to the trap block for `code` when `cond` is nonzero.
    fn trap_if(&mut self, cond: ir::Value, code: i32) {
        let trap = self.trap_block(code);
        let cont = self.b.create_block();
        self.b.ins().brif(cond, trap, &[], cont, &[]);
        self.b.seal_block(cont);
        self.b.switch_to_block(cont);
    }

    fn terminate_with_trap(&mut self, code: i32) {
        let trap = self.func_ref(self.rt.trap);
        let c = self.iconst(types::I32, code as i64);
        self.b.ins().call(trap, &[c]);
        self.b.ins().trap(TrapCode::unwrap_user(1));
    }

    /// An `i1` lives in an `i8`; arithmetic on it must wrap to one bit.
    fn wrap_i1(&mut self, ty: Type, v: ir::Value) -> ir::Value {
        if ty == Type::I1 {
            self.b.ins().band_imm_u(v, 1)
        } else {
            v
        }
    }

    fn set(&mut self, inst: Inst, v: ir::Value) {
        if let Some(r) = self.m.inst_result(inst) {
            self.values.insert(r, v);
        }
    }

    /// Lowers one instruction; returns whether control continues after it.
    fn inst(&mut self, inst: Inst) -> bool {
        let m = self.m;
        let op = m.inst_opcode(inst);
        let args: Vec<ir::Value> = m.inst_args(inst).iter().map(|&a| self.v(a)).collect();
        let arg_ty = |n: usize| m.value_type(m.inst_args(inst)[n]);
        let result_ty = m.inst_result(inst).map(|v| m.value_type(v));
        match op {
            Opcode::Iconst => {
                let ty = self.ty(result_ty.unwrap());
                let v = self.iconst(ty, m.inst_iconst(inst));
                self.set(inst, v);
            }
            Opcode::Fconst => {
                let bits = m.inst_fconst_bits(inst);
                let v = if result_ty == Some(Type::F32) {
                    self.b.ins().f32const(ir::immediates::Ieee32::with_bits(bits as u32))
                } else {
                    self.b.ins().f64const(ir::immediates::Ieee64::with_bits(bits))
                };
                self.set(inst, v);
            }
            Opcode::Null => {
                let v = self.iconst(self.ptr, 0);
                self.set(inst, v);
            }
            Opcode::Addr => {
                let sym = m.inst_symbol(inst);
                let v = match sym.kind {
                    SymbolKind::Func => {
                        let fr = self.func_ref(self.func_ids[sym.index as usize]);
                        self.b.ins().func_addr(self.ptr, fr)
                    }
                    SymbolKind::Global => {
                        let gv = self.obj.declare_data_in_func(self.data_ids[sym.index as usize], self.b.func);
                        self.b.ins().symbol_value(self.ptr, gv)
                    }
                };
                self.set(inst, v);
            }
            Opcode::Iadd | Opcode::Isub | Opcode::Imul => {
                let v = match op {
                    Opcode::Iadd => self.b.ins().iadd(args[0], args[1]),
                    Opcode::Isub => self.b.ins().isub(args[0], args[1]),
                    _ => self.b.ins().imul(args[0], args[1]),
                };
                let v = self.wrap_i1(arg_ty(0), v);
                self.set(inst, v);
            }
            Opcode::Sdiv | Opcode::Udiv | Opcode::Srem | Opcode::Urem => {
                let (a, b) = (args[0], args[1]);
                let ty = self.ty(arg_ty(0));
                let zero = self.iconst(ty, 0);
                let is_zero = self.b.ins().icmp(condcodes::IntCC::Equal, b, zero);
                self.trap_if(is_zero, a0_runtime::trap::DIVIDE_BY_ZERO);
                if op == Opcode::Sdiv {
                    let min = self.iconst(ty, i64::MIN >> (64 - ty.bits()));
                    let neg1 = self.iconst(ty, -1);
                    let a_min = self.b.ins().icmp(condcodes::IntCC::Equal, a, min);
                    let b_neg1 = self.b.ins().icmp(condcodes::IntCC::Equal, b, neg1);
                    let both = self.b.ins().band(a_min, b_neg1);
                    self.trap_if(both, a0_runtime::trap::INTEGER_OVERFLOW);
                }
                let v = match op {
                    Opcode::Sdiv => self.b.ins().sdiv(a, b),
                    Opcode::Udiv => self.b.ins().udiv(a, b),
                    Opcode::Srem => self.b.ins().srem(a, b),
                    _ => self.b.ins().urem(a, b),
                };
                let v = self.wrap_i1(arg_ty(0), v);
                self.set(inst, v);
            }
            Opcode::And => {
                let v = self.b.ins().band(args[0], args[1]);
                self.set(inst, v);
            }
            Opcode::Or => {
                let v = self.b.ins().bor(args[0], args[1]);
                self.set(inst, v);
            }
            Opcode::Xor => {
                let v = self.b.ins().bxor(args[0], args[1]);
                self.set(inst, v);
            }
            Opcode::Shl | Opcode::Lshr | Opcode::Ashr => {
                let ty = arg_ty(0);
                // Cranelift shifts by the amount modulo the width, like the IR. An i1 lives in an i8,
                // so its shift amount must be reduced modulo 1 (always 0) explicitly.
                let amount = if ty == Type::I1 {
                    self.iconst(types::I8, 0)
                } else {
                    args[1]
                };
                let v = match op {
                    Opcode::Shl => self.b.ins().ishl(args[0], amount),
                    Opcode::Lshr => self.b.ins().ushr(args[0], amount),
                    _ => self.b.ins().sshr(args[0], amount),
                };
                let v = if ty == Type::I1 { self.b.ins().band_imm_u(v, 1) } else { v };
                self.set(inst, v);
            }
            Opcode::Ineg => {
                let v = self.b.ins().ineg(args[0]);
                let v = if arg_ty(0) == Type::I1 { self.b.ins().band_imm_u(v, 1) } else { v };
                self.set(inst, v);
            }
            Opcode::Not => {
                let v = if arg_ty(0) == Type::I1 {
                    self.b.ins().bxor_imm_u(args[0], 1)
                } else {
                    self.b.ins().bnot(args[0])
                };
                self.set(inst, v);
            }
            Opcode::Fadd => {
                let v = self.b.ins().fadd(args[0], args[1]);
                self.set(inst, v);
            }
            Opcode::Fsub => {
                let v = self.b.ins().fsub(args[0], args[1]);
                self.set(inst, v);
            }
            Opcode::Fmul => {
                let v = self.b.ins().fmul(args[0], args[1]);
                self.set(inst, v);
            }
            Opcode::Fdiv => {
                let v = self.b.ins().fdiv(args[0], args[1]);
                self.set(inst, v);
            }
            Opcode::Fneg => {
                let v = self.b.ins().fneg(args[0]);
                self.set(inst, v);
            }
            Opcode::Icmp => {
                let cc = int_cc(m.inst_int_cc(inst));
                let v = self.b.ins().icmp(cc, args[0], args[1]);
                self.set(inst, v);
            }
            Opcode::Fcmp => {
                let cc = float_cc(m.inst_float_cc(inst));
                let v = self.b.ins().fcmp(cc, args[0], args[1]);
                self.set(inst, v);
            }
            op if op.is_cast() => {
                let v = self.cast(op, arg_ty(0), result_ty.unwrap(), args[0]);
                self.set(inst, v);
            }
            Opcode::Select => {
                let v = self.b.ins().select(args[0], args[1], args[2]);
                self.set(inst, v);
            }
            Opcode::Load => {
                let ty = result_ty.unwrap();
                let cty = self.ty(ty);
                let v = self.b.ins().load(cty, MemFlagsData::new(), args[0], 0);
                let v = if ty == Type::I1 { self.b.ins().band_imm_u(v, 1) } else { v };
                self.set(inst, v);
            }
            Opcode::Store => {
                self.b.ins().store(MemFlagsData::new(), args[0], args[1], 0);
            }
            Opcode::Ptradd => {
                let v = self.b.ins().iadd(args[0], args[1]);
                self.set(inst, v);
            }
            Opcode::Call => {
                let callee = m.inst_callee(inst);
                let fr = self.func_ref(self.func_ids[callee.index()]);
                let call = self.b.ins().call(fr, &args);
                if let Some(&r) = self.b.inst_results(call).first() {
                    self.set(inst, r);
                }
            }
            Opcode::CallIndirect => {
                let sig_id = m.inst_sig(inst);
                let sig = signature(self.obj, m.sig_params(sig_id), m.sig_ret(sig_id), self.target);
                let sig_ref = self.b.import_signature(sig);
                let call = self.b.ins().call_indirect(sig_ref, args[0], &args[1..]);
                if let Some(&r) = self.b.inst_results(call).first() {
                    self.set(inst, r);
                }
            }
            Opcode::Get => {
                let var = self.vars[&m.inst_var(inst)];
                let v = self.b.use_var(var);
                self.set(inst, v);
            }
            Opcode::Set => {
                let var = self.vars[&m.inst_var(inst)];
                self.b.def_var(var, args[0]);
            }
            Opcode::If => return self.lower_if(inst, args[0]),
            Opcode::Loop => return self.lower_loop(inst),
            Opcode::Break => {
                let lp = self.loops.last_mut().unwrap();
                lp.exit_used = true;
                let exit = lp.exit;
                self.b.ins().jump(exit, &[]);
                return false;
            }
            Opcode::Continue => {
                let header = self.loops.last().unwrap().header;
                self.b.ins().jump(header, &[]);
                return false;
            }
            Opcode::Return => {
                self.b.ins().return_(&args);
                return false;
            }
            Opcode::Unreachable => {
                self.terminate_with_trap(a0_runtime::trap::UNREACHABLE);
                return false;
            }
            _ => unreachable!("verified IR has no other opcodes"),
        }
        true
    }

    fn lower_if(&mut self, inst: Inst, cond: ir::Value) -> bool {
        let m = self.m;
        let then_b = self.b.create_block();
        let merge = self.b.create_block();
        let else_region = m.inst_region(inst, 1);
        let else_b = if else_region.is_some() {
            self.b.create_block()
        } else {
            merge
        };
        self.b.ins().brif(cond, then_b, &[], else_b, &[]);
        let mut merge_reachable = else_region.is_none();

        self.b.switch_to_block(then_b);
        self.b.seal_block(then_b);
        if self.region(m.inst_region(inst, 0).unwrap()) {
            self.b.ins().jump(merge, &[]);
            merge_reachable = true;
        }
        if let Some(els) = else_region {
            self.b.switch_to_block(else_b);
            self.b.seal_block(else_b);
            if self.region(els) {
                self.b.ins().jump(merge, &[]);
                merge_reachable = true;
            }
        }
        self.b.switch_to_block(merge);
        self.b.seal_block(merge);
        if !merge_reachable {
            // No predecessors: keep the block well-formed and report that control stops here.
            self.b.ins().trap(TrapCode::unwrap_user(1));
            return false;
        }
        true
    }

    fn lower_loop(&mut self, inst: Inst) -> bool {
        let header = self.b.create_block();
        let exit = self.b.create_block();
        self.b.ins().jump(header, &[]);
        self.b.switch_to_block(header);
        self.loops.push(LoopBlocks {
            header,
            exit,
            exit_used: false,
        });
        if self.region(self.m.inst_region(inst, 0).unwrap()) {
            self.b.ins().jump(header, &[]);
        }
        let lp = self.loops.pop().unwrap();
        self.b.seal_block(header);
        self.b.switch_to_block(exit);
        self.b.seal_block(exit);
        if !lp.exit_used {
            self.b.ins().trap(TrapCode::unwrap_user(1));
            return false;
        }
        true
    }

    fn cast(&mut self, op: Opcode, from: Type, to: Type, a: ir::Value) -> ir::Value {
        let (cf, ct) = (self.ty(from), self.ty(to));
        match op {
            Opcode::Zext => {
                if cf == ct {
                    a
                } else {
                    self.b.ins().uextend(ct, a)
                }
            }
            Opcode::Sext => {
                // An i1 is 0 or 1 in an i8; sign extension turns 1 into all ones.
                let a = if from == Type::I1 { self.b.ins().ineg(a) } else { a };
                if cf == ct { a } else { self.b.ins().sextend(ct, a) }
            }
            Opcode::Trunc => {
                let v = if cf == ct { a } else { self.b.ins().ireduce(ct, a) };
                if to == Type::I1 { self.b.ins().band_imm_u(v, 1) } else { v }
            }
            Opcode::Sitofp => self.b.ins().fcvt_from_sint(ct, a),
            Opcode::Uitofp => self.b.ins().fcvt_from_uint(ct, a),
            Opcode::Fptosi | Opcode::Fptoui => self.float_to_int(op == Opcode::Fptosi, to, a),
            Opcode::Fext => self.b.ins().fpromote(ct, a),
            Opcode::Ftrunc => self.b.ins().fdemote(ct, a),
            Opcode::Bitcast => self.b.ins().bitcast(ct, MemFlagsData::new(), a),
            Opcode::Ptrtoint => {
                let v = if cf == ct {
                    a
                } else if ct.bits() < cf.bits() {
                    self.b.ins().ireduce(ct, a)
                } else {
                    self.b.ins().uextend(ct, a)
                };
                if to == Type::I1 { self.b.ins().band_imm_u(v, 1) } else { v }
            }
            Opcode::Inttoptr => {
                if cf == ct {
                    a
                } else if cf.bits() < ct.bits() {
                    self.b.ins().uextend(ct, a)
                } else {
                    self.b.ins().ireduce(ct, a)
                }
            }
            _ => unreachable!(),
        }
    }

    /// Saturating float-to-int conversion (NaN converts to 0), for every integer width.
    fn float_to_int(&mut self, signed: bool, to: Type, a: ir::Value) -> ir::Value {
        match to {
            Type::I32 | Type::I64 => {
                let ct = self.ty(to);
                if signed {
                    self.b.ins().fcvt_to_sint_sat(ct, a)
                } else {
                    self.b.ins().fcvt_to_uint_sat(ct, a)
                }
            }
            Type::I1 => {
                let wide = self.b.ins().fcvt_to_sint_sat(types::I64, a);
                let zero = self.iconst(types::I64, 0);
                self.b.ins().icmp(condcodes::IntCC::NotEqual, wide, zero)
            }
            _ => {
                // Convert to i32 with saturation, then clamp to the narrow range.
                let bits = to.bits().unwrap();
                let wide = if signed {
                    self.b.ins().fcvt_to_sint_sat(types::I32, a)
                } else {
                    self.b.ins().fcvt_to_uint_sat(types::I32, a)
                };
                let clamped = if signed {
                    let max = self.iconst(types::I32, (1i64 << (bits - 1)) - 1);
                    let min = self.iconst(types::I32, -(1i64 << (bits - 1)));
                    let v = self.b.ins().smin(wide, max);
                    self.b.ins().smax(v, min)
                } else {
                    let max = self.iconst(types::I32, (1i64 << bits) - 1);
                    self.b.ins().umin(wide, max)
                };
                let cto = self.ty(to);
                self.b.ins().ireduce(cto, clamped)
            }
        }
    }
}

fn int_cc(cc: IntCC) -> condcodes::IntCC {
    use condcodes::IntCC as C;
    match cc {
        IntCC::Eq => C::Equal,
        IntCC::Ne => C::NotEqual,
        IntCC::Slt => C::SignedLessThan,
        IntCC::Sle => C::SignedLessThanOrEqual,
        IntCC::Sgt => C::SignedGreaterThan,
        IntCC::Sge => C::SignedGreaterThanOrEqual,
        IntCC::Ult => C::UnsignedLessThan,
        IntCC::Ule => C::UnsignedLessThanOrEqual,
        IntCC::Ugt => C::UnsignedGreaterThan,
        IntCC::Uge => C::UnsignedGreaterThanOrEqual,
    }
}

fn float_cc(cc: FloatCC) -> condcodes::FloatCC {
    use condcodes::FloatCC as C;
    match cc {
        FloatCC::Eq => C::Equal,
        FloatCC::Ne => C::NotEqual,
        FloatCC::Lt => C::LessThan,
        FloatCC::Le => C::LessThanOrEqual,
        FloatCC::Gt => C::GreaterThan,
        FloatCC::Ge => C::GreaterThanOrEqual,
    }
}
