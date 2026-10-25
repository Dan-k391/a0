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
     (it flushes).
   - A function generated code calls that can fail takes `uint64_t loc, void *frame` after its own
     parameters and fails through `a0_rt_fail(message, loc, frame)` (or `a0_rt_out_of_bounds`), so
     the failure prints where it happened and the stack trace. Add its name to
     `REPORTS_LOCATION` in `crates/a0-runtime/src/lib.rs`: the backends then pass the two arguments,
     and the IR declares the function without them. Put message formatting (a `char` buffer and
     `snprintf`) in a separate `A0_COLD` function, so the fast path needs no stack buffer (it made
     map access 7% slower when it did). `a0_panic` is for failures of the runtime itself, with no
     location.
   - If it allocates GC objects, every GC reference it holds across another allocation must be a
     root (see the GC section of `include/a0rt.h`).
3. `crates/a0-runtime/c/symbols.c` - add `SYM(a0_<name>)` so the interpreter can resolve it. In
   WebAssembly (the playground) the table holds wrappers that `build.rs` generates from the
   declaration in `a0rt.h`, so keep the declaration on one prototype there (a C library function
   goes into `C_LIBRARY` in `build.rs` instead). A field of a runtime structure that generated
   code reads or writes as a pointer is declared with `A0_WORD_PTR` (a 64-bit word on every
   target).
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
7. For the standard library (`docs/std.md`), put it in `c/std.c` and call it from the module's
   `crates/a0-hir/std/<module>.hir` through `extern "a0" fn a0_std_<name>(...)`: it gets and returns
   A0 values (`A0String *`, `A0Array *`, booleans as `int64_t`), and an `a0_` name keeps it private
   to the module. Report failures by returning NULL or 0 after `fail(...)`, so the HIR side can
   return `std.Result::Err(a0_std_error())`. A C library function needs no runtime code at all:
   declare it with `extern fn` (strings and arrays are passed as pointers, D47). Test it from
   `examples/minnow/tests/programs/std.mn`.

## Checks that must pass

```
cargo test -p a0-runtime -p a0-interp -p a0
cargo test -p lox --test suite        # if the GC or anything Lox uses changed
cargo clippy --workspace --all-targets
cargo fmt --all
```

Compiler warnings from the C build show up as `warning:` lines in cargo output; fix them.
