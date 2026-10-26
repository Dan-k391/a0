//! Native code generation for A0 IR.
//!
//! [`compile_object`] lowers a module to Cranelift IR and emits an object file for any supported
//! [`Target`]. [`link_executable`] links such an object with the C runtime using the system linker
//! (host target only). [`build_executable`] does both. [`run_main`] runs a module from memory
//! instead, through Cranelift's JIT, with no linking. [`add_thunks`] lets the interpreter call C
//! functions its own foreign calls cannot.
//!
//! IR variables become Cranelift variables, so Cranelift's frontend builds SSA; `if`/`loop` become
//! blocks. Integer division checks and `unreachable` call the runtime's `a0_rt_trap`, which prints the
//! same message as the interpreter.

mod cabi;
mod codegen;
mod codeview;
mod dwarf;
mod jit;
mod link;
mod thunks;

use std::path::Path;

use a0_diag::Diagnostics;
use a0_ir::Module;
pub use a0_target::Target;
pub use codegen::{CodegenOptions, ENTRY_SYMBOL, compile_object, compile_objects};
pub use jit::{JitOptions, jit_supported, run_main};
pub use link::{Library, LinkOptions, find_pack, link_executable, link_objects, pack_name};
pub use thunks::add_thunks;

#[derive(Clone, Debug, Default)]
pub struct BuildOptions {
    /// Skip Cranelift's optimizations (faster compiles, slower code).
    pub no_optimize: bool,
    /// Describe the program for the platform's debugger (`CodegenOptions::debug_info`).
    pub debug_info: bool,
    pub link: LinkOptions,
}

/// Compiles a module with an `@main` into an executable at `output`.
pub fn build_executable(
    m: &Module,
    target: Target,
    output: &Path,
    opts: &BuildOptions,
) -> Result<(), Diagnostics> {
    let objects = a0_diag::time::stage("code generation", || {
        compile_objects(
            m,
            target,
            &CodegenOptions {
                optimize: !opts.no_optimize,
                entry_point: true,
                debug_info: opts.debug_info,
            },
        )
    })?;
    let mut link = opts.link.clone();
    link.debug_info |= opts.debug_info;
    if opts.debug_info && target.os() == a0_target::Os::Windows {
        link.natvis = codeview::natvis(m);
    }
    a0_diag::time::stage("link", || link_objects(&objects, target, output, &link))
}
