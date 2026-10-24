---
name: add-example-language
description: Add a new example language to A0, or extend an existing one (Kaleidoscope, Lox) - grammar, compiler to IR, optional C runtime, driver, interpreted and native tests, docs. Use when building or changing a language under examples/.
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
3. `examples/foo/src/lib.rs` - `compile(text, SourceId) -> Result<Module, Diagnostics>`; load the
   grammar with `include_str!`; resolve names with `a0_sema::Scopes`; emit IR with
   `a0_ir::Builder`; return verifier failures as diagnostics, never panic on user input.
4. `examples/foo/runtime/foo.c` (optional) - C support code; export a NULL-terminated
   `const A0Symbol foo_symbols[]` table. `build.rs` calls
   `a0_build::runtime_library("foo_rt", &["runtime/foo.c"])`; `lib.rs` exposes `runtime_library()`
   (for linking) and `host_symbols()` (for the interpreter). See `examples/kaleidoscope`.
5. `examples/foo/src/main.rs` - the driver: `run`, `build`, `ir` subcommands.
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
