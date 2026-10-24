---
name: add-example-language
description: Add a new example language to A0, or extend an existing one (Kaleidoscope, Lox) - grammar, compiler to HIR or IR, optional C runtime, driver, interpreted and native tests, docs. Use when building or changing a language under examples/.
---

# Add or extend an example language

An example language proves the framework. It must run interpreted and natively with identical
output, and report errors with source locations.

## Files to create (new language `foo`)

1. `examples/foo/Cargo.toml` - workspace version/edition/license; depend on the `a0-*` crates it
   uses through `workspace = true`. If it ships C: `build = "build.rs"`, a direct dependency on
   `a0-runtime`, and `a0-build` as a build-dependency.
2. `examples/foo/foo.a0g` - the grammar (`docs/grammar.md`). Try it with
   `a0 parse examples/foo/foo.a0g sample.foo` before writing the compiler.
3. `examples/foo/src/lib.rs` - `compile(text, SourceId) -> Result<HirModule, Diagnostics>`; load
   the grammar with `include_str!`; resolve names with `a0_sema::Scopes`; for static types, check
   with `a0_sema::Types` (interned types, structs, inference by unification, mismatch
   diagnostics; see `examples/minnow`); emit HIR with
   `a0_hir::HirBuilder` (`docs/hir.md`), or core IR with `a0_ir::Builder` if the language needs
   nothing the HIR offers. Runtime support written in HIR goes in a `prelude.hir` parsed with
   `a0_hir::parse` (see `examples/lox`). Return builder failures as diagnostics, never panic on user
   input.
4. `examples/foo/runtime/foo.c` (optional) - C support code; export a NULL-terminated
   `const A0Symbol foo_symbols[]` table. `build.rs` calls
   `a0_build::runtime_library("foo_rt", &["runtime/foo.c"])`; `lib.rs` exposes `runtime_library()`
   (for linking) and `host_symbols()` (for the interpreter). See `examples/kaleidoscope`.
5. `examples/foo/src/main.rs` - an `a0_driver::Language` (name, compile function, exit codes, and
   for a C runtime its host symbols and libraries) and `.main()`. The driver provides `run`,
   `build`, `ir` and `hir`, renders diagnostics and maps failures to the exit codes. Do not write a
   driver by hand.
6. `examples/foo/tests/` - programs with expected output, run both interpreted and as native
   executables (copy the harness in `examples/kaleidoscope/tests/kaleidoscope.rs`), plus
   diagnostic tests that check message text and `file:line:col`.
7. `examples/foo/README.md` - what the language is, the file map, deliberate differences.

## Files to update

- Root `Cargo.toml`: add `examples/foo` to `members`.
- `README.md`: the layout table (and status if it is a milestone).
- `AGENTS.md`: anything that changes how to build or test.
- `docs/milestones.md`: the milestone summary when it lands.

## When extending an existing language

- Kaleidoscope: grammar in `kaleidoscope.a0g`, lowering in `src/lib.rs`; add a program to
  `tests/programs/*.k` with `# expect:` lines.
- Minnow: grammar `minnow.a0g`, names and types in `src/check.rs` (on `a0_sema::Scopes` and
  `a0_sema::Types`), HIR generation in `src/emit.rs`; programs in `tests/programs/*.mn` with
  `// expect:` lines, error cases in `tests/minnow.rs`. It is the template for a typed language.
- Lox: grammar `lox.a0g`, checks and resolver in `src/analyze.rs`, HIR generation in `src/emit.rs`,
  runtime semantics in HIR in `prelude.hir` (prefer changing the prelude over emitting more code).
  The official suite (`tests/suite.rs`) must stay fully green in all three modes.

## Checks that must pass

```
cargo test -p foo
cargo test --workspace
cargo clippy --workspace --all-targets
cargo fmt --all
```
