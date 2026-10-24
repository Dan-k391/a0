# A0

[![CI](https://github.com/Dan-k391/a0/actions/workflows/ci.yml/badge.svg)](https://github.com/Dan-k391/a0/actions/workflows/ci.yml)

A0 is an easy-to-use, front-to-back compiler framework: from a grammar to a native executable, for
general-purpose languages and DSLs alike. Ease of use comes first; efficiency comes mainly from reusing
existing backends (Cranelift) rather than building our own.

The core is written in Rust; the runtime is written in C. Each layer is a separate crate that can be
used on its own:

```
grammar ──a0-syntax──▶ syntax tree ──(frontend + a0-sema)──▶ HIR ──a0-hir──▶ A0 IR ──┬─ a0-interp ──▶ run it
                                                                                     └─ a0-native ──▶ executable
```

The high-level IR (HIR) gives a language strings, arrays, maps, structs, enums, closures, dynamic
values and garbage collection. The core IR is the low-level layer under it; a frontend can also emit
it directly.

## Status

| Milestone | State |
|---|---|
| 1. Core IR + interpreter | done |
| 2. Native executables from IR | done; linked and tested on every CI host |
| 3. Frontend: grammar layer, diagnostics | done |
| 4. Kaleidoscope end to end | done: tutorial programs run interpreted and native |
| 5. Lox end to end | done: all 241 tests of the official `jlox` suite pass, interpreted, native, and under GC stress |
| 6. High-level IR, runtime GC, Lox on the HIR | done: Lox shrank from 2647 to 1544 lines and still passes all 241 tests three ways |
| 7. A shared language driver | done: `a0-driver`, used by every example language |
| 8. CI on Linux, macOS and Windows | done: x86-64 and aarch64 Linux, aarch64 macOS, x86-64 Windows |
| 9. Static types toolkit, a typed example language | done: `a0_sema::Types` and Minnow |

`docs/milestones.md` has the summary of each milestone and the open questions.

## Layout

| Path | What it is |
|---|---|
| `crates/a0-diag` | Source files, spans and diagnostics shared by every layer |
| `crates/a0-syntax` | Grammar layer: grammar DSL, lexer, parser, syntax trees and syntax errors |
| `crates/a0-sema` | Semantic toolkit: scopes, name resolution and static types (inference by unification) |
| `crates/a0-hir` | High-level IR: strings, collections, structs, enums, closures, `any`, GC; lowered to the core IR |
| `crates/a0-ir` | The core IR: data model, builder, text format, verifier |
| `crates/a0-interp` | Interpreter for the IR |
| `crates/a0-target` | Explicit target descriptions |
| `crates/a0-native` | Native code through Cranelift, linked by the system linker |
| `crates/a0-runtime` | The C runtime (`include/a0rt.h`, `c/`) and its Rust build glue |
| `crates/a0-build` | Build-script helper for languages that ship a C runtime library (Kaleidoscope) |
| `crates/a0-driver` | The command-line driver every example language uses (run, build, ir, hir) |
| `crates/a0` | The `a0` command-line tool |
| `examples/kaleidoscope` | Kaleidoscope (LLVM's tutorial language) end to end |
| `examples/lox` | Lox (Crafting Interpreters) end to end, with the official test suite |
| `examples/minnow` | Minnow, a small statically typed language with structs, arrays and closures |
| `tests/programs`, `tests/hir` | Core IR and HIR programs with expected output, run by both backends |
| `AGENTS.md`, `.claude/skills` | How to work on A0: commands, layer rules, conventions, workflows |
| `docs/ir.md`, `docs/hir.md`, `docs/grammar.md` | Core IR, high-level IR and grammar DSL |
| `docs/decisions.md`, `docs/milestones.md` | Design decisions; milestone summaries and open questions |

## Building and testing

Requires a Rust toolchain and a C compiler (MSVC on Windows, `cc` elsewhere).

```
cargo build
cargo test
```

`cargo test` runs everything, including the Lox suite three ways and every IR program and Kaleidoscope
program through both backends.

## Using the tools

```
a0 check prog.a0ir                        # parse and verify IR
a0 fmt prog.a0ir                          # print the canonical form
a0 run prog.a0ir                          # interpret @main
a0 build prog.a0ir                        # native executable for the host
a0 obj --target aarch64-linux prog.a0ir   # object file for another target
a0 parse lang.a0g input.txt               # try a grammar: print the tree or the errors
a0 run prog.hir                           # HIR programs: run, build, fmt, check work the same
a0 lower prog.hir                         # print the core IR a HIR module lowers to

kaleidoscope run examples/kaleidoscope/tests/programs/mandelbrot.k
lox script.lox                            # like jlox; `lox build script.lox` for an executable
minnow run examples/minnow/tests/programs/closures.mn
```

## Writing a language with A0

1. Write a grammar (`docs/grammar.md`): tokens, rules, `@prec` operator tables, `~ "message"` error
   messages, `@recover` for panic-mode recovery. Try it with `a0 parse`.
2. Walk the `SyntaxTree` and emit HIR with `a0_hir::HirBuilder` (`docs/hir.md`): typed expressions,
   with strings, collections, closures, `any` and garbage collection built in. For static types,
   check with `a0_sema::Types` (interning, structs, inference by unification). Runtime support can be
   written in HIR text and parsed with `a0_hir::parse`. Use `a0_sema::Scopes` for name resolution.
   For full control, emit the core IR with `a0_ir::Builder` instead (variables instead of SSA,
   `if_`/`loop_` for control flow).
3. Give `a0_driver::Language` your compile function: it provides `run` (interpreter), `build`
   (native executable), `ir` and `hir`, renders diagnostics and maps errors to your exit codes. Or
   call `a0_hir::lower`, `a0_interp` and `a0_native::build_executable` yourself. A language that
   needs C support code ships it with `a0_build::runtime_library`.

`examples/kaleidoscope` (about 650 lines of Rust, on the core IR) is the small, complete example;
`examples/minnow` is a statically typed language on the HIR, and `examples/lox` a dynamically typed
one.
