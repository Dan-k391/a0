//! Thunks: how the interpreter calls C functions its own foreign calls cannot (C structs by value,
//! `f32`, more than `FFI_MAX_ARGS` arguments). Each is compiled in memory, by the same code
//! generator native code comes from, so the interpreter, the JIT and executables share one
//! implementation of the C ABI (D73). A thunk is an IR function
//! `@a0.thunk.NAME(%args: ptr, %ret: ptr)`: it loads each argument from its 64-bit word in
//! `args`, calls the C function, and stores the result at `ret`.

use std::sync::Arc;

use a0_diag::{Diagnostic, Diagnostics};
use a0_ir::{Builder, FuncId, IntCC, Module, Opcode, Type};
use a0_runtime::HostSymbols;
use a0_target::Target;
use cranelift_jit::{JITBuilder, JITModule};

use crate::codegen::{compile_functions, make_isa};

/// Whether the interpreter needs a thunk to call the extern `f`.
fn needs_thunk(m: &Module, f: FuncId) -> bool {
    let params = m.func_params(f);
    m.func_has_struct_params(f)
        || params.len() > a0_runtime::FFI_MAX_ARGS
        || params.contains(&Type::F32)
        || m.func_ret(f) == Some(Type::F32)
}

/// The compiled thunks, kept alive as long as the symbols that point into them.
struct Thunks(#[allow(dead_code)] JITModule);

// SAFETY: the module is never used again after its code is finalized; it is only kept so that its
// memory stays mapped, and dropped (unmapping it) when the last symbol table referring to it goes.
unsafe impl Send for Thunks {}
unsafe impl Sync for Thunks {}

/// Compiles a thunk for every C function `m` calls that the interpreter cannot call by itself,
/// and adds them to `symbols` (`HostSymbols::thunk`). Does nothing when there are none, or when
/// this host cannot run native code (the interpreter then says which function it cannot call).
pub fn add_thunks(m: &Module, symbols: &mut HostSymbols) -> Result<(), Diagnostics> {
    let Some(target) = Target::host() else {
        return Ok(());
    };
    let referenced = a0_ir::referenced_externs(m);
    let wanted: Vec<FuncId> = (0..m.func_count())
        .map(FuncId)
        .filter(|&f| {
            m.func_is_extern(f)
                && referenced[f.index()]
                && needs_thunk(m, f)
                && symbols.thunk(m.func_name(f)).is_none()
                && symbols.lookup(m.func_name(f)).is_some()
        })
        .collect();
    if wanted.is_empty() {
        return Ok(());
    }

    let mut b = Builder::new("a0.thunks");
    let mut thunks = Vec::new();
    for &f in &wanted {
        let name = m.func_name(f);
        let params = m.func_params(f);
        let ret = m.func_ret(f);
        let c = b.declare_extern(name, params, ret);
        for i in 0..params.len() as u32 {
            if let Some((pass, s)) = m.func_struct_param(f, i) {
                let types: Vec<Type> = s.fields.iter().map(|&(t, _)| t).collect();
                let offsets: Vec<u32> = s.fields.iter().map(|&(_, at)| at).collect();
                b.set_struct_param(c, i, pass, s.size, s.align, &types, &offsets);
            }
        }
        let thunk_name = format!("a0.thunk.{name}");
        let t = b.declare_function(&thunk_name, &[Type::Ptr, Type::Ptr], None);
        b.begin_function(t);
        let args_ptr = b.param(0);
        let ret_ptr = b.param(1);
        let mut args = Vec::new();
        for (i, &p) in params.iter().enumerate() {
            let at = b.iconst(Type::I64, 8 * i as i64);
            let word = b.ptradd(args_ptr, at);
            let v = if p == Type::I1 {
                let byte = b.load(Type::I8, word);
                let zero = b.iconst(Type::I8, 0);
                b.icmp(IntCC::Ne, byte, zero)
            } else {
                b.load(p, word)
            };
            args.push(v);
        }
        if let Some(r) = b.call(c, &args) {
            let r = if ret == Some(Type::I1) {
                b.cast(Opcode::Zext, r, Type::I8)
            } else {
                r
            };
            b.store(r, ret_ptr);
        }
        b.ret(None);
        b.end_function();
        thunks.push((name.to_string(), t));
    }
    let tm = b.finish()?;

    let isa = make_isa(target, true, true)?;
    let mut builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
    let lookup = symbols.clone();
    builder.symbol_lookup_fn(Box::new(move |name| {
        lookup.lookup(name).map(|a| a as *const u8)
    }));
    let mut jit = JITModule::new(builder);
    let locations = a0_ir::Locations::build(&tm);
    let compiled = compile_functions(
        &mut jit,
        &tm,
        target,
        "a0.thunks.main",
        false,
        &crate::codegen::Unit::whole(&tm),
        &locations,
    )?;
    jit.finalize_definitions().map_err(|e| {
        Diagnostics::from(Diagnostic::error(
            "native.jit",
            &format!("cannot load the interpreter's thunks: {e}"),
        ))
    })?;
    for (name, t) in thunks {
        let address = jit.get_finalized_function(compiled.func_ids[t.index()]) as usize;
        symbols.add_thunk(&name, address);
    }
    symbols.keep_alive(Arc::new(Thunks(jit)));
    Ok(())
}
