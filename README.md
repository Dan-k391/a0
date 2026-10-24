# A0

A0 is an easy-to-use, front-to-back compiler framework: from a grammar to a native executable, for
general-purpose languages and DSLs alike. Ease of use comes first; efficiency comes mainly from reusing
existing backends (Cranelift) rather than building our own.

The core is written in Rust; the runtime is written in C. Each layer is a separate crate that can be
used on its own:

```
grammar file ──a0-syntax──▶ syntax tree ──(your frontend + a0-sema)──▶ A0 IR ──┬─ a0-interp ──▶ run it
                                                                              └─ a0-native ──▶ executable
```

## Status

| Milestone | State |
|---|---|
| 1. Core IR + interpreter | done |
| 2. Native executables from IR | done (x86-64 Windows linked and tested here; see open questions) |
| 3. Frontend: grammar layer, diagnostics | done |
| 4. Kaleidoscope end to end | done: tutorial programs run interpreted and native |
| 5. Lox end to end | done: all 241 tests of the official `jlox` suite pass, interpreted, native, and under GC stress |

`docs/milestones.md` has the summary of each milestone and the open questions.

## Layout

| Path | What it is |
|---|---|
| `crates/a0-diag` | Source files, spans and diagnostics shared by every layer |
| `crates/a0-syntax` | Grammar layer: grammar DSL, lexer, parser, syntax trees and syntax errors |
| `crates/a0-sema` | Semantic toolkit: scopes and name resolution |
| `crates/a0-ir` | The core IR: data model, builder, text format, verifier |
| `crates/a0-interp` | Interpreter for the IR |
| `crates/a0-target` | Explicit target descriptions |
| `crates/a0-native` | Native code through Cranelift, linked by the system linker |
| `crates/a0-runtime` | The C runtime (`include/a0rt.h`, `c/`) and its Rust build glue |
| `crates/a0-build` | Build-script helper for languages that ship a C runtime library |
| `crates/a0` | The `a0` command-line driver |
| `examples/kaleidoscope` | Kaleidoscope (LLVM's tutorial language) end to end |
| `examples/lox` | Lox (Crafting Interpreters) end to end, with the official test suite |
| `tests/programs` | IR programs with expected output, run by both backends |
| `docs/ir.md`, `docs/grammar.md` | IR design and grammar DSL reference |
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

kaleidoscope run examples/kaleidoscope/tests/programs/mandelbrot.k
lox script.lox                            # like jlox; `lox build script.lox` for an executable
```

## Writing a language with A0

1. Write a grammar (`docs/grammar.md`): tokens, rules, `@prec` operator tables, `~ "message"` error
   messages, `@recover` for panic-mode recovery. Try it with `a0 parse`.
2. Walk the `SyntaxTree` and emit IR with `a0_ir::Builder`: variables instead of SSA, `if_`/`loop_`
   for control flow. Use `a0_sema::Scopes` for name resolution.
3. Run the module with `a0_interp`, or build an executable with `a0_native::build_executable`. If the
   language needs C support code, ship it with `a0_build::runtime_library`.

`examples/kaleidoscope` (about 600 lines of Rust) is the small, complete example; `examples/lox` is the
large one.
