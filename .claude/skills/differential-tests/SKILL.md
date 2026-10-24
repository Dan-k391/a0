---
name: differential-tests
description: Run and debug A0's differential tests, which require the interpreter and native executables to produce identical stdout, stderr and exit status (IR and HIR programs, Kaleidoscope, Minnow, the Lox suite, GC stress). Use when a backend test fails or when adding a regression test.
---

# Run and debug the differential tests

## What runs where

| Suite | Harness | Modes |
|---|---|---|
| IR and HIR programs `tests/programs/*.a0ir`, `tests/hir/*.hir` | `crates/a0/tests/programs.rs` | `a0 run`, `a0 build` + run, `a0 run` + `A0_GC_STRESS=1` |
| Objects for every target | `crates/a0-native/tests/objects.rs` | object inspection only |
| Kaleidoscope `examples/kaleidoscope/tests/programs/*.k` | `examples/kaleidoscope/tests/kaleidoscope.rs` | interpreted, native |
| Minnow `examples/minnow/tests/programs/*.mn` | `examples/minnow/tests/minnow.rs` | interpreted, native, interpreted + `A0_GC_STRESS=1` |
| Lox official suite | `examples/lox/tests/suite.rs` | interpreted, native, interpreted + `A0_GC_STRESS=1` |

Every HIR-based mode runs twice: optimized (the default) and with `A0_OPT=0`. A difference between
the two is an optimizer bug: compare `a0 lower prog.hir` with and without `A0_OPT=0`, and shrink the
program until one rewrite in `crates/a0-hir/src/opt.rs` explains it.

```
cargo test -p a0 --test programs
cargo test -p kaleidoscope
cargo test -p minnow
cargo test -p lox --test suite -- --nocapture          # prints the pass counts
cargo test -p lox --test suite suite_native -- --exact # one mode
```

## Expectation formats

- IR programs: `; expect-stdout: <line>`, `; expect-stderr: <line>`, `; expect-exit: <code>`
  (default 0; stderr must match exactly, empty if none declared).
- HIR programs: the same, after `//`.
- Kaleidoscope: `# expect: <line>`.
- Minnow: `// expect: <line>`, `// expect-stderr: <line>`, `// expect-exit: <code>`.
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
- **`a shadow stack frame was popped uncleared`** (GC stress only): some path left a scope without
  clearing its frame slots; lowering clears them in `release`, `pop_frame` and `leave_loop_scopes`
  (`crates/a0-hir/src/lower.rs`, decision D30).
- **Interpreter-only failure** about FFI: the C function's signature is outside the interpreter's FFI
  shape (more than 8 arguments, or `f32`).
- **Native-only crash in deep recursion**: the native stack is smaller than the interpreter's depth
  limit; check the language's own depth limit.

## After fixing

Add the smallest program that showed the bug to `tests/programs/` (or the language's tests) so it
stays fixed, then run the full `cargo test --workspace`.
