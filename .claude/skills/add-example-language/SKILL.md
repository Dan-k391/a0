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
   `a0 parse examples/foo/foo.a0g sample.foo` before writing the compiler. Mark declarations with
   `@def(kind)` / `@def(kind: field)` and scopes with `@scope`: that is all the language server
   needs for its outline, highlighting, go-to-definition and hovers.
3. `examples/foo/src/lib.rs` - `compile(text, SourceId) -> Result<HirModule, Diagnostics>`; load
   the grammar with `include_str!`; build HIR by name with the builder's frontend helpers
   (`begin_body`, `let_named`, `name`, `call_named`, `named_type`, `new_struct_named`; see
   `docs/hir.md` and `examples/minnow`), which resolve names, infer and check types and report
   errors, so the language needs no checker of its own (`a0_sema::Types` remains for type systems
   the HIR cannot express); emit HIR with
   `a0_hir::HirBuilder` (`docs/hir.md`), or core IR with `a0_ir::Builder` if the language needs
   nothing the HIR offers. Runtime support written in HIR goes in a `prelude.hir` parsed with
   `a0_hir::parse` (see `examples/lox`). Return builder failures as diagnostics, never panic on user
   input. For rustc-quality messages (`docs/diagnostics.md`): `set_span` to the node before building
   it, attribute declarations to their names, `set_param_span` after `set_signature`,
   `set_name_span` before a by-name call whose span is not just the name, `set_type_syntax` and
   `set_term` for the language's words, `enable_warning` for the warnings it wants, and report its
   own errors with `report_code` (A0's code for the same kind of problem) plus `report_label` for
   related places.
4. `examples/foo/runtime/foo.c` (optional) - C support code; export a NULL-terminated
   `const A0Symbol foo_symbols[]` table. `build.rs` calls
   `a0_build::runtime_library("foo_rt", &["runtime/foo.c"])`; `lib.rs` exposes `runtime_library()`
   (for linking) and `host_symbols()` (for the interpreter). See `examples/kaleidoscope`.
5. `examples/foo/src/main.rs` - an `a0_driver::Language` (name, compile function, exit codes, the
   grammar text for the language server, and for a C runtime its host symbols and libraries) and
   `.main()`. The driver provides `run`, `build`, `ir`, `hir`, `lsp` and `explain`, renders
   diagnostics (`codes`: A0's, hidden, or the language's own) and maps failures to the exit codes. Do not write a driver by hand. Errors in the language's own format
   (`CompileError::Messages`) carry spans so editors can place them. For programs of several
   files, mark the import rule `@import` (`docs/grammar.md`), split `compile` into a `start` and a
   per-file `compile_file` that declares with `set_public` and reaches other modules with
   `member`/`call_member`/`member_type` (`docs/hir.md`, "Modules"), and set `modules` to an
   `a0_driver::Modules`; the driver and the language server then load the imported files.
6. `examples/foo/tests/` - programs with expected output, run both interpreted and as native
   executables (copy the harness in `examples/kaleidoscope/tests/kaleidoscope.rs`), plus
   diagnostic tests that check message text and `file:line:col`, `tests/diagnostics/` with
   snapshots of full rendered output (compile errors, warnings, runtime errors; copy
   `examples/minnow/tests/diagnostics.rs`, bless with `A0_BLESS=1`), and `tests/lsp.rs`, which runs
   the language server on a small program with `a0_driver::lsp::session` (copy
   `examples/minnow/tests/lsp.rs`) and checks the outline, a definition, a hover, a diagnostic and a
   quick fix; and `tests/dap.rs`, a debugging session with `a0_driver::dap::Client` (copy
   `examples/minnow/tests/dap.rs`: a breakpoint, the stack, locals, a step). The debugger needs a
   span on every HIR expression (literals too), `set_closure_name` for functions that are
   closures, and a C runtime of the language's own that writes with `a0_rt_write`.
7. `examples/foo/README.md` - what the language is, the file map, deliberate differences.

## Files to update

- Root `Cargo.toml`: add `examples/foo` to `members`.
- `README.md`: the repository layout table.
- `AGENTS.md`: anything that changes how to build or test.
- `editors/vscode/package.json`: a `contributes.languages` entry, its `semanticTokenScopes` and an
  `a0.languages` default, so VS Code starts `foo lsp` for its files.
- `docs/milestones.md`: the milestone summary when it lands.

## When extending an existing language

- Kaleidoscope: grammar in `kaleidoscope.a0g`, the compiler to HIR in `src/lib.rs` (by name, like
  Minnow); add a program to `tests/programs/*.k` with `# expect:` lines. `docs/tutorial.md` quotes
  `src/lib.rs` and the grammar between `ANCHOR:` markers, and a test checks the quotes match:
  `A0_BLESS=1 cargo test -p kaleidoscope --test tutorial` rewrites them, then check the prose.
- Minnow: grammar `minnow.a0g`, the whole compiler in `src/lib.rs` (one walk building HIR by name;
  the HIR builder resolves names and checks types); programs in `tests/programs/*.mn` with
  `// expect:` lines, error cases in `tests/minnow.rs`. It is the template for a typed language.
- Lox: grammar `lox.a0g`, checks and resolver in `src/analyze.rs`, HIR generation in `src/emit.rs`,
  runtime semantics in HIR in `prelude.hir` (prefer changing the prelude over emitting more code).
  The official suite (`tests/suite.rs`) must stay fully green in all six modes (interpreted,
  native, GC stress, each optimized and with `A0_OPT=0`). A0's extensions to Lox (exceptions, the
  natives in `natives.hir`) get tests in `tests/a0`, in the suite's form; keep them out of the
  book's behavior (D51).

## Checks that must pass

```
cargo test -p foo
cargo test --workspace
cargo clippy --workspace --all-targets
cargo fmt --all
```
