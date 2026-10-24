---
name: differential-tests
description: Run and debug A0's differential tests, which require the interpreter and native executables to produce identical stdout, stderr and exit status (IR programs, Kaleidoscope, the Lox suite, GC stress). Use when a backend test fails or when adding a regression test.
---

# Run and debug the differential tests

## What runs where

| Suite | Harness | Modes |
|---|---|---|
| IR programs `tests/programs/*.a0ir` | `crates/a0/tests/programs.rs` | `a0 run`, `a0 build` + run |
| Objects for every target | `crates/a0-native/tests/objects.rs` | object inspection only |
| Kaleidoscope `examples/kaleidoscope/tests/programs/*.k` | `examples/kaleidoscope/tests/kaleidoscope.rs` | interpreted, native |
| Lox official suite | `examples/lox/tests/suite.rs` | interpreted, native, interpreted + `A0_GC_STRESS=1` |

```
cargo test -p a0 --test programs
cargo test -p kaleidoscope
cargo test -p lox --test suite -- --nocapture          # prints the pass counts
cargo test -p lox --test suite suite_native -- --exact # one mode
```

## Expectation formats

- IR programs: `; expect-stdout: <line>`, `; expect-stderr: <line>`, `; expect-exit: <code>`
  (default 0; stderr must match exactly, empty if none declared).
- Kaleidoscope: `# expect: <line>`.
- Lox: the book's comments (`// expect: `, `// Error ...`, `// [line N] Error ...`,
  `// expect runtime error: `), checked exactly like `tool/bin/test.dart`.

## Reproduce one failure by hand

```
a0 run tests/programs/x.a0ir;  echo $?
a0 build tests/programs/x.a0ir -o x.exe && ./x.exe;  echo $?
lox ir file.lox > file.a0ir      # the IR a frontend produced; then a0 run / a0 build it
```

Shrink the program until the two backends disagree on a single instruction.

## Usual causes

- **Interpreter and native disagree on a value**: compare the semantics helper in
  `crates/a0-interp/src/lib.rs` (`int_binary`, `cast`, `float_to_int`, `canon`) with the lowering in
  `crates/a0-native/src/codegen.rs`. Known traps: `i1` lives in an `i8` natively (mask with
  `wrap_i1`); narrow immediates must be masked; float-to-int conversions saturate and map NaN to 0.
- **Output order or missing output**: something wrote to stderr without flushing stdout, or exited
  without `a0_rt_exit`.
- **Crash or wrong value only under GC stress**: a GC reference was held outside a root (a frame slot,
  a registered root or a C temp root) across an allocation.
- **Interpreter-only failure** about FFI: the C function's signature is outside the interpreter's FFI
  shape (more than 8 arguments, or `f32`).
- **Native-only crash in deep recursion**: the native stack is smaller than the interpreter's depth
  limit; check the language's own depth limit.

## After fixing

Add the smallest program that showed the bug to `tests/programs/` (or the language's tests) so it
stays fixed, then run the full `cargo test --workspace`.
