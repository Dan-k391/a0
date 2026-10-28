# A0

[![CI](https://github.com/Dan-k391/a0/actions/workflows/ci.yml/badge.svg)](https://github.com/Dan-k391/a0/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/Dan-k391/a0)](https://github.com/Dan-k391/a0/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**A0 turns a grammar into a real programming language: write the syntax, and your language gets
native speed, executables, garbage collection, rustc-quality errors, a language server, a debugger
and a REPL, without writing any of them.**

![A terminal session: a Calc program using a `times` loop fails with "unknown name `times`"; one
line added to calc.a0g in nano defines the loop; the program then runs, and builds into a native
executable that prints the same output.](docs/demo/calc.gif)

<sub>Above, [Calc](examples/calc) (a language that is nothing but a 54-line grammar) learns a new
loop from one line of grammar, then runs in the JIT and as a native executable. Recorded live by
[`docs/demo/record.py`](docs/demo/record.py).</sub>

**[Quick start](#quick-start)** · **[Playground](https://dan-k391.github.io/a0/)** ·
**[Tutorial](docs/tutorial.md)** · **[The book](docs/book/README.md)** ·
**[Documentation](#documentation)** · **[Benchmarks](docs/benchmarks.md)**

## Quick start

**No install:** open **[the playground](https://dan-k391.github.io/a0/)**. It is A0 itself compiled
to WebAssembly: edit a grammar on one side and a program on the other, and run it in your browser.
It also shows the syntax tree, the HIR and the core IR.

**On your machine** you need Rust and a C compiler (`cc` on Linux and macOS, MSVC on Windows):

```
git clone https://github.com/Dan-k391/a0 && cd a0
cargo install --path crates/a0                 # the `a0` command
a0 lang examples/calc/calc.a0g run examples/calc/programs/loops.calc
a0 lang examples/calc/calc.a0g build examples/calc/programs/loops.calc -o loops
./loops
```

Prebuilt tools for Linux, macOS and Windows (x86-64 and aarch64) are on the
[release page](https://github.com/Dan-k391/a0/releases/latest): `a0`, the example languages, a
linker and the cross-compilation packs, with nothing else to install ([docs/releases.md](docs/releases.md)).

## A language in one file

[Calc](examples/calc) is a calculator language with functions, loops, the standard mathematics,
native executables and a language server. All of it is [`calc.a0g`](examples/calc/calc.a0g): each
rule says what it means with an `@hir` form, and `a0 lang` does the rest. An excerpt:

```
while_stmt @hir(while) = "while" cond:expr ~ "expected a condition" body:block ~ "expected '{'";
repeat_stmt @hir(repeat) = "repeat" count:expr ~ "expected a count" body:block ~ "expected '{'";

expr @inline = @prec primary ~ "expected an expression" {
    left "or" => binary;
    left "+" "-" => binary;
    left "*" "/" => binary;
    prefix "-" "not" => unary;
};
```

The same grammar gives every command a language has:

```
a0 lang examples/calc/calc.a0g run   prog.calc    # compile into memory and run (the JIT)
a0 lang examples/calc/calc.a0g build prog.calc    # a native executable
a0 lang examples/calc/calc.a0g hir   prog.calc    # the typed IR it became
a0 lang examples/calc/calc.a0g lsp                # a language server for your editor
```

A mistake is reported in the program's own terms, with a code that `a0 explain` describes:

```
error[E2001]: mismatched types
 --> bad.calc:2:11
  |
2 | print x + "a"
  |       -   ^^^ expected `float`, found `string`
  |       |
  |       expected because this is `float`
```

[Lispy](examples/lispy) uses the same forms with a Lisp syntax. Grammar-only languages are
described in [docs/grammar.md](docs/grammar.md), "Languages without host code".

## A language with a compiler

When a language needs more than the `@hir` forms (its own types, modules, a C runtime), you write
a compiler from syntax trees to A0's typed high-level IR. `a0 new` starts one: a grammar, a
compiler for a small typed language, its command, tests that pass from the start, and VS Code
settings.

```
a0 new mylang
cd mylang
cargo run -- run examples/hello.mylang
cargo test
```

```
grammar ──a0-syntax──▶ syntax tree ──your compiler──▶ HIR ──a0-hir──▶ core IR ──┬─ JIT / native code ──▶ run it, or an executable
                                                                                └─ interpreter ───────▶ the reference, and the debugger
```

1. **Write a grammar** ([docs/grammar.md](docs/grammar.md)): tokens, rules, `@prec` operator
   tables, `~ "message"` errors, `@recover` for error recovery, and `@def`/`@scope` for editors.
   Try it with `a0 parse lang.a0g input`.
2. **Walk the syntax tree and build HIR** with `a0_hir::HirBuilder` ([docs/hir.md](docs/hir.md)).
   Build by name (`let_named`, `call_named`, `named_type`, ...) and the builder resolves names,
   infers and checks types and reports errors, so a typed language needs no checker of its own.
3. **Hand your compile function to `a0_driver::Language`.** It provides `run`, `build` (`-g` for
   gdb and lldb), `ir`, `hir`, `lsp`, `dap`, `repl` and `explain`.

[The tutorial](docs/tutorial.md) builds Kaleidoscope this way, step by step; its listings are cut
from the real code, and a test runs its commands. A language can also be written in Python over
the C API ([docs/python.md](docs/python.md)), with the same driver.

## What every language gets

- **A parser from a grammar**, loaded at run time, with error messages and recovery, so one run
  reports every syntax error.
- **A high-level IR** with strings, arrays, maps, structs, enums with `match`, generics, closures,
  nullable references, dynamic (`any`) values, methods, interfaces, exceptions and results (`?`).
  The builder infers and checks types and reports errors in the language's own syntax.
- **Backends that agree**: native code through Cranelift, run at once from memory (a JIT) or linked
  into executables, and an interpreter. Same output, errors and exit codes, checked on every test
  program, on Linux, macOS and Windows; cross-compilation to Linux and Windows from any host
  ([docs/cross.md](docs/cross.md)).
- **A precise garbage collector**, shared by threads (or none, for a language that manages its own
  memory), and an **optimizer**, on by default, with no work in the frontend.
- **Systems features** when a language wants them: sized numbers, C-layout structs as values, raw
  pointers, and C functions called directly, structs passed by value.
- **Diagnostics as good as rustc's**: codes with explanations, related locations, "did you mean"
  fixes, warnings, and runtime errors with a stack trace of source functions.
- **Programs of several files and packages**: imports found through the grammar, modules with
  visibility, manifests, dependencies and lock files, with reproducible builds ([docs/packages.md](docs/packages.md)).
- **A standard library**: files, input, arguments, the environment, time, mathematics, strings,
  and threads with channels ([docs/std.md](docs/std.md)).
- **Tools**: a REPL, a language server (diagnostics with quick fixes, highlighting, outline,
  go-to-definition, hover, completion, references, rename), a debugger (breakpoints, stepping,
  locals), a VS Code extension, and DWARF for gdb and lldb.
- **A C API and Python bindings**, for hosts that are not written in Rust ([docs/c-api.md](docs/c-api.md)).

## Languages built on A0

| Language | What it shows | Size |
|---|---|---|
| [B0](b0) | The flagship: static types with inference, structs, enums, generics, interfaces, closures, exceptions, threads, `unsafe` pointers, C functions, packages, a prelude in B0. Built only on the public API. | ~1350 lines |
| [Kaleidoscope](examples/kaleidoscope) | The language of LLVM's tutorial, and what A0's tutorial builds | ~580 lines |
| [Minnow](examples/minnow) | A typed language with modules, generics, interfaces and closures, and [`json`](examples/minnow/json), a tool written in it | ~1000 lines |
| [Lox](examples/lox) | *Crafting Interpreters*' language: passes all 241 tests of the official suite, and beats clox on five of the book's six benchmarks | ~1500 lines |
| [Tin](examples/tin) | A C-like systems language without a collector that calls cJSON as C would | ~850 lines |
| [Calc](examples/calc), [Lispy](examples/lispy) | Languages that are only a grammar | 54 and 41 lines |
| [Pebble](bindings/python/examples/pebble) | A typed language whose compiler is a Python file | one file |

Sizes include the grammar. B0 and the example languages with compilers each have a language
server, a debugger and a REPL, and are tested interpreted, in the JIT and natively:

```
cargo build --release
target/release/b0 run b0/tests/programs/prelude.b0
target/release/minnow run examples/minnow/tests/programs/closures.mn
target/release/lox examples/lox/tests/craftinginterpreters/test/benchmark/fib.lox
target/release/kaleidoscope build examples/kaleidoscope/tests/programs/mandelbrot.k -o mandel && ./mandel
```

## Performance

A0 compiles through Cranelift, so its languages run at roughly `clang -O0` speed, and A0's own
work, from source to object file, takes 4–20 ms. Lox compiled with A0 beats clox, the book's
bytecode VM, on five of its six benchmarks. Kaleidoscope runs `mandel` slightly faster than LLVM's
tutorial passes. A 100,000-line program builds in 1.3 s. The numbers, the machine and how to rerun
them are in [docs/benchmarks.md](docs/benchmarks.md) (`bench/run.py`).

## Documentation

| Document | What it covers |
|---|---|
| [The book](docs/book/README.md) | From getting started to a full language, memory, concurrency and hosting |
| [docs/tutorial.md](docs/tutorial.md) | Building Kaleidoscope, step by step |
| [docs/grammar.md](docs/grammar.md) | The grammar language, and languages without host code |
| [docs/hir.md](docs/hir.md) | The high-level IR: types, expressions, building it, lowering, optimization |
| [docs/ir.md](docs/ir.md) | The core IR and its text format |
| [docs/std.md](docs/std.md) | The standard library |
| [docs/diagnostics.md](docs/diagnostics.md) | Error messages: how they render, their codes, how a language makes them good |
| [docs/editors.md](docs/editors.md) | The language server in VS Code, Neovim and Helix |
| [docs/debugging.md](docs/debugging.md) | The debugger, and native debug information |
| [docs/repl.md](docs/repl.md) | The REPL |
| [docs/packages.md](docs/packages.md) | Packages: manifests, dependencies, lock files, reproducible builds |
| [docs/cross.md](docs/cross.md) | Cross-compilation |
| [docs/c-api.md](docs/c-api.md), [docs/python.md](docs/python.md) | Using A0 from C, and writing a language in Python |
| [docs/playground.md](docs/playground.md) | The browser playground: how it works, building it |
| [docs/benchmarks.md](docs/benchmarks.md) | Compile and run times against LLVM, C, Go and clox |
| [docs/compatibility.md](docs/compatibility.md) | What stays compatible until 2.0 |
| [docs/releases.md](docs/releases.md) | Release archives and packages |
| [docs/decisions.md](docs/decisions.md), [docs/milestones.md](docs/milestones.md) | Design decisions and why; what was built when |
| [CHANGELOG.md](CHANGELOG.md) | Releases |
| [AGENTS.md](AGENTS.md) | Working on A0: commands, layer rules, conventions |

<details>
<summary><b>The <code>a0</code> command</b></summary>

```
a0 new mylang                             # start a language project
a0 lang calc.a0g run prog.calc            # a grammar-only language (also build, hir, ir, lsp)
a0 parse lang.a0g input.txt               # try a grammar: print the tree or the errors
a0 explain E2001                          # what an error code means

a0 run prog.hir                           # HIR and core IR programs: run (JIT; A0_INTERP=1
a0 build prog.a0ir                        #   interprets), build, check, fmt
a0 build --target x86_64-windows prog.hir # an executable for another platform
a0 obj --target aarch64-linux prog.a0ir   # an object file for another target
a0 lower prog.hir                         # the core IR a HIR module lowers to
```

`A0_OPT=0` turns the HIR optimizer off; `A0_GC_STRESS=1` runs the collector on every allocation.
A language built on `a0_driver` has the same subcommands in its own command (`mylang run`,
`build`, `check`, `ir`, `hir`, `repl`, `lsp`, `dap`, `explain`; [docs/book/getting-started.md](docs/book/getting-started.md)).

</details>

<details>
<summary><b>Repository layout</b></summary>

| Path | What it is |
|---|---|
| `crates/a0-diag` | Source files, spans and diagnostics shared by every layer |
| `crates/a0-syntax` | Grammar language, lexer, parser, syntax trees, the syntax index for editors |
| `crates/a0-sema` | Scopes and static types for type systems the HIR does not cover |
| `crates/a0-hir` | High-level IR: types, builder with inference, checker, text format, optimizer, lowering |
| `crates/a0-ir` | The core IR: data model, builder, text format, verifier |
| `crates/a0-interp` | Interpreter for the core IR |
| `crates/a0-target` | Target descriptions |
| `crates/a0-native` | Native code through Cranelift, linked by the system linker |
| `crates/a0-wasm` | WebAssembly from the core IR, for the playground |
| `crates/a0-runtime` | The C runtime (`include/a0rt.h`, `c/`): output, strings, collections, `any`, the collector |
| `crates/a0-build` | Build-script helper for languages that ship C code |
| `crates/a0-driver` | The driver every language uses (`run`, `build`, `lsp`, `dap`, `repl`, ...) |
| `crates/a0-capi` | The C API: static and shared libraries, and `include/a0.h` |
| `crates/a0` | The `a0` command (the crate `a0-cli`) |
| `b0` | B0, the flagship language |
| `examples/*` | Kaleidoscope, Lox, Minnow, Tin, and the grammar-only Calc and Lispy |
| `bindings/python` | The Python package `a0`, and Pebble |
| `crates/a0-playground`, `playground` | The browser playground: its engine and its page |
| `editors/vscode` | The VS Code extension |
| `tests/programs`, `tests/hir` | Core IR and HIR programs with expected output, run by every backend |
| `bench` | The benchmark |

</details>

## Building and testing

```
cargo build
cargo test
```

`cargo test` runs everything: every IR, HIR and example-language program and the Lox suite
interpreted, in the JIT and natively, under GC stress, optimized and not; random programs, which
must agree in all those modes; fuzzing of every input A0 reads; diagnostic snapshots; the language
servers, debuggers and REPLs; the C API and the Python bindings; and the tutorial's listings. CI
runs it on x86-64 and aarch64 Linux, macOS and Windows. [AGENTS.md](AGENTS.md) has the rest.

## Status

**1.0.0**, stable. A0 follows Semantic Versioning: the command line, the text formats, the C API,
the Python package and the Rust API for languages stay compatible until 2.0
([docs/compatibility.md](docs/compatibility.md)). A language project depends on A0 from git,
pinned to a release (`a0-hir = { git = "https://github.com/Dan-k391/a0", tag = "v1.0.0" }`), as
`a0 new` sets up. A0 is MIT-licensed.
