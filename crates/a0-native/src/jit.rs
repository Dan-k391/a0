//! Running a module at once, from memory: the code generator's output is loaded into this
//! process by Cranelift's JIT instead of being written to an object file and linked (D63).
//!
//! The code is the same as an executable's, so programs behave the same: the C runtime is the
//! one this process links, extern functions resolve through [`HostSymbols`] as in the
//! interpreter, and runtime errors find their stack through the same tables, here in memory.

use a0_diag::{Diagnostic, Diagnostics};
use a0_ir::{FuncId, Module};
use a0_runtime::HostSymbols;
use a0_target::Target;
use cranelift_jit::{JITBuilder, JITModule};

use crate::codegen::{compile_functions, make_isa};

/// How [`run_main`] compiles and runs a program.
#[derive(Clone, Debug)]
pub struct JitOptions {
    /// Let Cranelift optimize (`opt_level = speed`), as `build` does.
    pub optimize: bool,
    /// The stack of the thread the program runs on, in bytes: an executable's main thread by
    /// default (`LinkOptions::stack_size`).
    pub stack_size: u64,
}

impl Default for JitOptions {
    fn default() -> Self {
        JitOptions {
            optimize: true,
            stack_size: crate::LinkOptions::default().stack_size,
        }
    }
}

fn jit_error(message: &str) -> Diagnostics {
    Diagnostic::error("native.jit", message).into()
}

/// Whether this host can run programs from memory (it is a supported target).
pub fn jit_supported() -> bool {
    Target::host().is_some()
}

/// Compiles a module with an `@main` into memory and runs it on a thread of its own: the runtime
/// is initialized with `args`, and `@main`'s result is returned as the exit status. A program that
/// fails (a trap, a panic) reports it and ends the process, as an executable would.
pub fn run_main(
    m: &Module,
    symbols: HostSymbols,
    args: Vec<String>,
    opts: &JitOptions,
) -> Result<i32, Diagnostics> {
    let Some(target) = Target::host() else {
        return Err(jit_error("native code is not supported on this host"));
    };
    let diags = a0_ir::verify(m);
    if diags.has_errors() {
        return Err(diags);
    }
    let diags = a0_ir::check_entry(m);
    if diags.has_errors() {
        return Err(diags);
    }
    // cranelift-jit panics on a symbol it cannot find, so every C function the program uses is
    // looked up first, and reported as the interpreter reports it.
    let mut unresolved = Diagnostics::new();
    for (f, used) in a0_ir::referenced_externs(m).into_iter().enumerate() {
        let f = FuncId(f as u32);
        if used && m.func_is_extern(f) && symbols.lookup(m.func_name(f)).is_none() {
            unresolved.push(
                Diagnostic::error(
                    "jit.unresolved",
                    &format!("unresolved external function @{}", m.func_name(f)),
                )
                .with_span_opt(m.func_span(f))
                .with_note("register its address in HostSymbols before running"),
            );
        }
    }
    if unresolved.has_errors() {
        return Err(unresolved);
    }

    let (jit, main, tables) = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        compile(m, target, symbols, opts)
    })) {
        Ok(result) => result?,
        Err(payload) => {
            let detail = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "unknown error".to_string());
            return Err(jit_error(&format!(
                "internal code generator error: {detail}"
            )));
        }
    };

    let handle = std::thread::Builder::new()
        .name("a0-main".to_string())
        .stack_size(opts.stack_size as usize)
        .spawn(move || {
            a0_runtime::init(&args);
            // SAFETY: the tables live in the JIT's memory, which outlives the program.
            unsafe { a0_runtime::set_code_tables(tables.0 as *const u8, tables.1 as *const u8) };
            // SAFETY: `main` is the IR's `@main`, `func @main() -> i32`, compiled with the C
            // calling convention.
            let main: extern "C" fn() -> i32 = unsafe { std::mem::transmute(main) };
            main()
        })
        .map_err(|e| jit_error(&format!("cannot start the program's thread: {e}")))?;
    let code = handle
        .join()
        .map_err(|_| jit_error("the program's thread panicked"))?;
    // The program is over; nothing refers to its code any more.
    unsafe { jit.free_memory() };
    Ok(code)
}

/// The finalized module, `@main`'s address, and the addresses of the trace tables.
fn compile(
    m: &Module,
    target: Target,
    symbols: HostSymbols,
    opts: &JitOptions,
) -> Result<(JITModule, usize, (usize, usize)), Diagnostics> {
    let isa = make_isa(target, opts.optimize, true)?;
    let mut builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
    builder.symbol_lookup_fn(Box::new(move |name| {
        symbols.lookup(name).map(|a| a as *const u8)
    }));
    let mut jit = JITModule::new(builder);
    let compiled = compile_functions(&mut jit, m, target, "main", false)?;
    jit.finalize_definitions()
        .map_err(|e| jit_error(&format!("cannot load the compiled code: {e}")))?;
    let main = m.lookup_func("main").expect("checked by check_entry");
    let main = jit.get_finalized_function(compiled.func_ids[main.index()]) as usize;
    let locations = jit.get_finalized_data(compiled.tables.0).0 as usize;
    let code = jit.get_finalized_data(compiled.tables.1).0 as usize;
    Ok((jit, main, (locations, code)))
}
