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
   - a builder method that pushes the node; intern any type it produces (`fn_type`, `array_type`, ...)
     *before* `push`, because `infer` only looks types up;
   - the typing rule in `infer` (and `builtin_type` for builtins); return `Err(message)` for misuse;
   - `visit_children` if the node has children; `KEYWORDS` if a new name becomes reserved.
3. `crates/a0-hir/src/check.rs` - only for rules that need context (enclosing body, loops, tree shape).
4. Text format, always together:
   - `crates/a0-hir/src/hir.a0g` (grammar; a new operator goes into the `@prec` table);
   - `crates/a0-hir/src/print.rs` (printing; operator levels mirror the `@prec` lines);
   - `crates/a0-hir/src/text.rs` (tree to builder calls).
5. `crates/a0-hir/src/lower.rs`
   - lower in `expr_inner` (or `builtin`/`cast`/`binary`);
   - classify it in `may_gc` if it can allocate, and evaluate operands with `operands(..)` so GC values
     are spilled when needed;
   - runtime calls are declared in `Runtime::declare`.
6. Runtime support, if needed: follow the `add-runtime-function` skill.
7. `docs/hir.md` - the type/expression/builtin tables and lowering notes.
8. Tests:
   - a program in `tests/hir/<name>.hir` with `// expect-stdout:` lines (runs in all three modes);
   - typing errors in `crates/a0-hir/tests/hir.rs` (`type_errors_are_reported_with_locations`);
   - the round-trip test covers the new syntax automatically once a corpus file uses it.

## Checks that must pass

```
cargo test -p a0-hir -p a0
cargo test -p lox --test suite     # if Lox's prelude or lowering paths it uses changed
cargo clippy --workspace --all-targets
cargo fmt --all
```
