---
name: add-ir-instruction
description: Add a new instruction (opcode) to the A0 core IR end to end - model, builder, verifier, text format, interpreter, Cranelift backend, docs and differential tests. Use when the low-level IR needs a new operation.
---

# Add an IR instruction end to end

An opcode is not done until both backends implement it identically and the text format round-trips.

## Files to touch, in order

1. `crates/a0-ir/src/types.rs`
   - Add the variant and its spelling to the `opcodes!` list.
   - Update the classification helpers (`is_int_binary`, `is_cast`, `is_terminator`, `can_trap`, ...)
     if the new opcode belongs to a group.
2. `crates/a0-ir/src/builder.rs` - a builder method that computes the result type from the
   operands. Misuse goes to `self.error(...)`, never a panic.
3. `crates/a0-ir/src/verify.rs` (`FuncVerifier::inst`)
   - operand count (`arity_ok`), whether it produces a value (`produces`), and the type rule.
4. `crates/a0-ir/src/print.rs` (`inst_text`) and `crates/a0-ir/src/parse.rs` (`Parser::inst`):
   print and parse the operands; add the opcode to `needs_result` handling if it has no result.
5. `crates/a0-ir/tests/data/all_ops.a0ir` - use the opcode in canonical form (the round-trip test
   compares the printed text byte for byte).
6. `crates/a0-interp/src/lib.rs` - an `Op` variant (or reuse one), the case in
   `FuncCompiler::inst`, and execution in `Interpreter::exec`. Put semantics in a helper function
   next to `int_binary`/`cast`.
7. `crates/a0-native/src/codegen.rs` (`Lower::inst`) - lower to Cranelift. Remember: `i1` lives in an
   `i8` (mask results with `wrap_i1`), immediates for narrow types go through `self.iconst`, traps
   call the runtime through `trap_if`/`terminate_with_trap`.
8. `docs/ir.md` - the instruction table and, if relevant, "Defined behaviour".
9. `tests/programs/<name>.a0ir` - a program that prints results of the new instruction, including
   edge cases, with `; expect-stdout:` lines. It runs in both backends.

## Checks that must pass

```
cargo test -p a0-ir -p a0-interp -p a0-native -p a0
cargo clippy --workspace --all-targets
cargo fmt --all
```

`a0-native`'s `every_program_compiles_for_every_target` must still pass: the new program is
compiled for every emittable target.
