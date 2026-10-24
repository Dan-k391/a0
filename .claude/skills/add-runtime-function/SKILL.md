---
name: add-runtime-function
description: Add a C function to the A0 runtime (a0rt) so IR programs and languages can call it from both the interpreter and native executables. Use when IR or a language needs new runtime support (I/O, memory, strings, collections, GC).
---

# Add a runtime function

The runtime is C, compiled once into a static library that the interpreter calls through
`a0_ffi_call` and that native executables link. Both paths must behave identically.

## Files to touch

1. `crates/a0-runtime/include/a0rt.h` - the declaration with a comment saying what it does, what it
   allocates, and what it does on errors. Name it `a0_<area>_<verb>`.
2. `crates/a0-runtime/c/<area>.c` - the implementation. A new `.c` file must be added to the
   `cc::Build` file list in `crates/a0-runtime/build.rs`.
   - Write output with stdio; flush stdout before writing to stderr; exit through `a0_rt_exit`
     (it flushes) or `a0_rt_trap`/`a0_panic` (exit status 101).
   - If it allocates GC objects, every GC reference it holds across another allocation must be a
     root (see the GC section of `include/a0rt.h`).
3. `crates/a0-runtime/c/symbols.c` - add `SYM(a0_<name>)` so the interpreter can resolve it.
4. Interpreter FFI limits: at most 8 arguments, each an integer, pointer or `double`; the result void,
   integer, pointer or `double`. No `float` (f32) and no structs by value. Keep signatures in this
   shape or the interpreter cannot call them.
5. Tests:
   - `crates/a0-runtime/src/lib.rs` (`mod tests`) for direct behavior (call it through `ffi_call`
     or an `extern "C"` declaration).
   - `tests/programs/<name>.a0ir` calling it via `extern func @a0_<name>(...)`, so both backends
     run it.
6. If the HIR lowering uses it, declare it in `Runtime::declare` in `crates/a0-hir/src/lower.rs`,
   classify it in `may_gc` there if it allocates, and update `docs/hir.md` if it changes the HIR's
   semantics.

## Checks that must pass

```
cargo test -p a0-runtime -p a0-interp -p a0
cargo test -p lox --test suite        # if the GC or anything Lox uses changed
cargo clippy --workspace --all-targets
cargo fmt --all
```

Compiler warnings from the C build show up as `warning:` lines in cargo output; fix them.
