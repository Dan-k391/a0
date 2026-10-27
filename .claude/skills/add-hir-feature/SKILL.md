---
name: add-hir-feature
description: Add a type, expression or builtin to the A0 high-level IR (HIR) end to end - model, builder typing, checker, text grammar and printer, lowering to the core IR, runtime support, docs and tests. Use when a language needs something the HIR does not have yet.
---

# Add a HIR feature end to end

A HIR feature is done when it can be built, printed, parsed back to the same text, lowered, and run
identically interpreted, natively and under `A0_GC_STRESS=1`.

## Files to touch, in order

1. `crates/a0-hir/src/model.rs` - the `ExprKind` variant (or `Builtin` entry, or `TypeKind`).
2. `crates/a0-hir/src/builder.rs`
   - a builder method that pushes the node (generic items get fresh type arguments there);
   - the typing rule in `infer` (and `builtin_type` for builtins). Use `expect` / `coerce` / `unify`
     (`types.rs`) rather than comparing `TypeId`s, since types may be inference variables. Return
     `TypeErr::At(Some(sub_expr), message)` for misuse (messages in source terms, types in
     backquotes) and `TypeErr::Open(sub_expr)` when the rule needs a type that is not inferred yet
     (`open_or` does both); the node is then typed again at `finish`;
   - `visit_children` if the node has children; `KEYWORDS` if a new name becomes reserved.
3. `crates/a0-hir/src/check.rs` - only for rules that need context (enclosing body, loops, tree shape).
4. Text format, always together:
   - `crates/a0-hir/src/hir.a0g` (grammar; a new operator goes into the `@prec` table);
   - `crates/a0-hir/src/print.rs` (printing; operator levels mirror the `@prec` lines);
   - `crates/a0-hir/src/text.rs` (tree to builder calls).
5. `crates/a0-hir/src/mono.rs` - copy the node in `Mono::expr` (mapping any ids and types it holds,
   locals with `self.local`); lowering only ever sees monomorphized modules.
   `crates/a0-hir/src/opt.rs` - `map_children` rebuilds the node; a node that binds a local needs a
   case in `copy` (inlining gives the copy fresh locals) and `pure` must stay false if it has effects.
   `crates/a0-hir/src/frontend.rs` - a by-name helper if frontends need one (`declare_if_let`).
6. `crates/a0-hir/src/lower.rs`
   - lower in `expr_inner` (or `builtin`/`cast`/`binary`);
   - classify it in `may_gc` if it can allocate, and evaluate operands with `operands(..)` so GC values
     are spilled when needed;
   - runtime calls are declared in `Runtime::declare`.
7. Runtime support, if needed: follow the `add-runtime-function` skill. A builder method also gets
   a binding in `crates/a0-capi` (`src/hir.rs`, `include/a0.h`).
8. `docs/hir.md` - the type/expression/builtin tables and lowering notes.
9. Tests:
   - a program in `tests/hir/<name>.hir` with `// expect-stdout:` lines (runs in all eight modes:
     interpreted, JIT, native, GC stress, each with and without the optimizer);
   - typing errors in `crates/a0-hir/tests/hir.rs` (`type_errors_are_reported_with_locations`),
     each with its code: a new kind of error gets a code and explanation in `a0_diag::codes` and a
     row in `docs/diagnostics.md`, a short message with a primary label, related labels where an
     expectation comes from, and `Term`s instead of HIR syntax (build it as a `Problem` in
     `builder.rs`);
   - a new runtime failure: its message with the values involved, and a program in `tests/hir`
     that expects the message and the stack trace;
   - the round-trip test covers the new syntax automatically once a corpus file uses it.

## Checks that must pass

```
cargo test -p a0-hir -p a0-cli
cargo test -p lox -p minnow        # both languages are built on the HIR
cargo clippy --workspace --all-targets
cargo fmt --all
```
