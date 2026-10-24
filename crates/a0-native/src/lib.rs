//! Native code generation for A0 IR.
//!
//! [`compile_object`] lowers a module to Cranelift IR and emits an object file for any supported
//! [`Target`]. [`link_executable`] links such an object with the C runtime using the system linker
//! (host target only). [`build_executable`] does both.
//!
//! IR variables become Cranelift variables, so Cranelift's frontend builds SSA; `if`/`loop` become
//! blocks. Integer division checks and `unreachable` call the runtime's `a0_rt_trap`, which prints the
//! same message as the interpreter.

mod codegen;
mod link;

use std::path::Path;

use a0_diag::Diagnostics;
use a0_ir::Module;
pub use a0_target::Target;
pub use codegen::{CodegenOptions, ENTRY_SYMBOL, compile_object};
pub use link::{Library, LinkOptions, link_executable};

#[derive(Clone, Debug, Default)]
pub struct BuildOptions {
    /// Skip Cranelift's optimizations (faster compiles, slower code).
    pub no_optimize: bool,
    pub link: LinkOptions,
}

/// Compiles a module with an `@main` into an executable at `output`.
pub fn build_executable(
    m: &Module,
    target: Target,
    output: &Path,
    opts: &BuildOptions,
) -> Result<(), Diagnostics> {
    let object = compile_object(
        m,
        target,
        &CodegenOptions {
            optimize: !opts.no_optimize,
            entry_point: true,
        },
    )?;
    link_executable(&object, target, output, &opts.link)
}
