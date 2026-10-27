# A0

[![CI](https://github.com/Dan-k391/a0/actions/workflows/ci.yml/badge.svg)](https://github.com/Dan-k391/a0/actions/workflows/ci.yml)

A0 is a framework for building programming languages. You write a grammar and a compiler from its
syntax trees to A0's high-level IR; your language runs at once at native speed (a JIT), compiles to
native executables, has an interpreter for debugging, garbage
collection, an optimizer, error messages in its own terms, and editor support.

```
grammar ──a0-syntax──▶ syntax tree ──your compiler──▶ HIR ──a0-hir──▶ core IR ──┬─ a0-interp ──▶ run it
                                                                                └─ a0-native ──▶ executable
```

## What a language gets

- **A parser from a grammar file**, loaded at run time: tokens, rules, operator precedence tables,
  error messages (`~ "expected ')'"`) and error recovery, so one run reports every syntax error.
- **A high-level IR** with strings, arrays, maps, structs, enums with `match`, generics, closures,
  nullable references (narrowed with `if let`) and dynamic (`any`) values, methods and interfaces
  with dynamic dispatch and generics bounded by them, and errors both ways: exceptions (`throw`,
  `try`/`catch`) and results (`?`). The builder resolves names, infers and checks types and
  reports errors in the language's own syntax, so a typed language needs no checker.
- **Programs of several files**: imports found through the grammar, modules with their own names
  and visibility, supported by the driver, the language server and diagnostics.
- **A standard library** every language can offer: files, standard input, arguments, the
  environment, time, mathematics, strings, and threads with channels. Calling C functions that take strings and arrays
  takes only their declaration.
- **Backends that agree**: native code through Cranelift, run at once from memory (a JIT) or
  linked into executables by the system linker, and an interpreter, the reference, on Linux
  (x86-64, aarch64), macOS (aarch64, x86-64) and Windows (x86-64). Same output, errors and exit
  codes, checked on every test program.
- **A precise garbage collector**, which threads share (or none, for a language that manages its
  memory itself), and an **optimizer**, on by default, with no work in the frontend.
- **Systems features** when a language wants them: sized numbers, structs as values laid out as C
  lays them out, raw pointers, and C functions that take and return structs by value.
- **Diagnostics as good as rustc's**, from the builder: codes with explanations, related locations
  (where an expected type comes from, the earlier declaration), "did you mean" fixes, warnings, in
  the language's words; and runtime errors with the values involved and a stack trace of source
  functions, inlined ones included.
- **A command-line driver** (`run`, `build`, `ir`, `hir`, `explain`) with **a REPL** (`repl`),
  **a language server** (`lsp`) for diagnostics with quick fixes, highlighting, outline,
  go-to-definition, hover, completion, find-references and rename across a program's files, and
  **a debugger** (`dap`: breakpoints, stepping, the stack and its locals), with a VS Code
  extension; `build -g` describes executables for gdb and lldb.
- **A C API**, for hosts that are not written in Rust: a C program can build HIR with the same
  builder, run it and compile it. **Python bindings** over it, for languages written in Python.

[B0](b0) is A0's flagship language: static types with inference, structs and inline structs,
enums, generics, interfaces, closures, maps, exceptions, dynamic objects, threads, manual memory
with raw pointers in `unsafe`, C functions, modules and packages, with a prelude written in B0
itself. Its compiler is about 1350 lines including its grammar and uses only A0's public API
([docs/compatibility.md](docs/compatibility.md)), as anyone's language would.

The example languages show the scale. [Kaleidoscope](examples/kaleidoscope), the language of
LLVM's tutorial, is about 580 lines including its grammar and is what the tutorial builds;
[Minnow](examples/minnow), a typed language with modules, structs, enums, generics, interfaces,
closures and nullable references, is about 1000, and [`json`](examples/minnow/json), a JSON tool
written in it, about 480; [Lox](examples/lox), from *Crafting Interpreters*, passes all 241 tests
of the official suite (and has exceptions as an extension), and its native programs run faster
than clox, the book's own bytecode virtual machine, on all of the book's benchmarks
([docs/benchmarks.md](docs/benchmarks.md)); and [Tin](examples/tin), a C-like systems language
(structs as values, raw pointers, memory it frees itself, C functions called directly, structs
passed to them by value), is about 850 lines including its grammar and calls cJSON, a C
library, as C would.

```
// examples/minnow/tests/programs/closures.mn
fn counter() -> fn() -> int {
  let n = 0;
  return fn() -> int {
    n = n + 1;
    return n;
  };
}

fn main() {
  let c = counter();
  c();
  c();
  print(c());  // 3
}
```

## Quick start

You need Rust and a C compiler (MSVC on Windows, `cc` on Linux and macOS).

```
git clone https://github.com/Dan-k391/a0 && cd a0
cargo build --release
target/release/minnow run examples/minnow/tests/programs/closures.mn
minnow build                              # in a package: minnow.toml (docs/packages.md)
target/release/lox examples/lox/tests/craftinginterpreters/test/benchmark/fib.lox
target/release/kaleidoscope build examples/kaleidoscope/tests/programs/mandelbrot.k -o mandel
./mandel
```

Then:

- **[The tutorial](docs/tutorial.md)** builds Kaleidoscope (LLVM's tutorial language) step by step.
  Its listings are cut from the example's code and its commands are run by a test, so what you read
  is what runs.
- **[B0](b0)** shows everything at once, and is the model to start a full language from.
- **[Minnow](examples/minnow)** is the model for a typed language, **[Lox](examples/lox)** for a
  dynamically typed one, **[Tin](examples/tin)** for a systems language without a collector, on
  the high-level IR.
- **[docs/editors.md](docs/editors.md)** sets up the language server in VS Code, Neovim or Helix.

## Writing a language

`a0 new mylang` creates a project to start from: a grammar and a compiler for a small typed
language, its command (`run`, `build`, `lsp`, ...), tests that run its programs both ways and
check its language server, and VS Code settings. `cargo test` in it passes from the start.

```
cargo install --path crates/a0
a0 new mylang
cd mylang && cargo run -- run examples/hello.mylang
```

**Try it in the browser: [the playground](https://dan-k391.github.io/a0/)** runs A0 itself,
compiled to WebAssembly: edit a grammar and a program and run it (`docs/playground.md`).

A small language needs no compiler at all: its grammar can say what each rule means
(`docs/grammar.md`, "Languages without host code"), and `a0 lang` runs it. [Calc](examples/calc)
is a calculator language of one 54-line grammar, with functions, loops, the standard mathematics,
native executables and a language server:

```
a0 lang examples/calc/calc.a0g run examples/calc/programs/functions.calc
```

A language can also be written in Python, through the C API (`docs/python.md`), and gets the
same driver: the JIT, native executables, the language server, the debugger and the REPL.
[Pebble](bindings/python/examples/pebble) is a typed language with arrays whose compiler is a
Python file:

```
cargo build -p a0-capi
python bindings/python/examples/pebble/pebble.py run bindings/python/examples/pebble/programs/arrays.pb
```

By hand:

1. Write a grammar (`docs/grammar.md`): tokens, rules, `@prec` operator tables, `~ "message"` error
   messages, `@recover` for error recovery, and `@def`/`@scope` to mark declarations and scopes for
   editors. Try it with `a0 parse lang.a0g input`.
2. Walk the `SyntaxTree` once and build HIR with `a0_hir::HirBuilder` (`docs/hir.md`). Build by name
   (`let_named`, `name`, `call_named`, `named_type`, ...) and the builder resolves names, infers
   and checks types and reports errors. Runtime support can be written in HIR itself and parsed with
   `a0_hir::parse`. A language that needs full control can emit the core IR (`docs/ir.md`) with
   `a0_ir::Builder` instead.
3. Give `a0_driver::Language` your compile function and grammar. It provides `run` (native speed, through the JIT),
   `build` (native executable, `-g` for gdb and lldb), `ir`, `hir`, `lsp`, `dap` (debugger) and
   `repl`, renders diagnostics and maps errors to your exit codes.

A0 is not published on crates.io; depend on it from git (`a0-hir = { git =
"https://github.com/Dan-k391/a0", tag = "v0.5.0" }`) or work inside this repository.

## Documentation

| Document | What it covers |
|---|---|
| [docs/tutorial.md](docs/tutorial.md) | Building Kaleidoscope, step by step |
| [docs/grammar.md](docs/grammar.md) | The grammar language |
| [docs/hir.md](docs/hir.md) | The high-level IR: types, expressions, building it from a frontend, lowering, optimization |
| [docs/ir.md](docs/ir.md) | The core IR and its text format |
| [docs/diagnostics.md](docs/diagnostics.md) | Error messages: how they render, their codes, how a language makes them good |
| [docs/editors.md](docs/editors.md) | The language server and editor setup |
| [docs/c-api.md](docs/c-api.md) | Using A0 from C |
| [docs/python.md](docs/python.md) | Writing a language in Python |
| [docs/packages.md](docs/packages.md) | Packages: manifests, dependencies, lock files, reproducible builds |
| [docs/cross.md](docs/cross.md) | Cross-compilation: building executables for other platforms |
| [docs/playground.md](docs/playground.md) | The browser playground: building it, how it works |
| [docs/debugging.md](docs/debugging.md) | The debugger every language gets, and native debug information |
| [docs/repl.md](docs/repl.md) | The REPL every language gets |
| [docs/benchmarks.md](docs/benchmarks.md) | Where A0 stands against LLVM, C, Go and clox: compile and run times |
| [docs/decisions.md](docs/decisions.md) | Design decisions and why |
| [docs/milestones.md](docs/milestones.md) | What was built when, with measurements and open questions |
| [CHANGELOG.md](CHANGELOG.md) | Releases |
| [AGENTS.md](AGENTS.md) | How to work on A0: commands, layer rules, conventions |

## Using the tools

```
a0 check prog.a0ir                        # parse and verify IR
a0 fmt prog.a0ir                          # print the canonical form
a0 run prog.a0ir                          # run @main (JIT; A0_INTERP=1 interprets)
a0 build prog.a0ir                        # native executable for the host
a0 obj --target aarch64-linux prog.a0ir   # object file for another target
a0 build --target x86_64-windows prog.hir # executable for another platform (docs/cross.md)
a0 parse lang.a0g input.txt               # try a grammar: print the tree or the errors
a0 run prog.hir                           # HIR programs: run, build, fmt, check work the same
a0 lower prog.hir                         # print the core IR a HIR module lowers to

kaleidoscope run examples/kaleidoscope/tests/programs/mandelbrot.k
lox script.lox                            # like jlox; `lox build script.lox` for an executable
minnow run examples/minnow/tests/programs/closures.mn
b0 run b0/tests/programs/prelude.b0
lox lsp                                   # the language server, for editors
```

`A0_OPT=0` turns the HIR optimizer off; `A0_GC_STRESS=1` makes the collector run on every
allocation.

## Repository layout

| Path | What it is |
|---|---|
| `crates/a0-diag` | Source files, spans and diagnostics shared by every layer |
| `crates/a0-syntax` | Grammar layer: grammar language, lexer, parser, syntax trees, the syntax index for editors |
| `crates/a0-sema` | Scopes and static types for languages whose type systems the HIR does not cover |
| `crates/a0-hir` | High-level IR: types, builder with inference, checker, text format, optimizer, lowering |
| `crates/a0-ir` | The core IR: data model, builder, text format, verifier |
| `crates/a0-interp` | Interpreter for the core IR |
| `crates/a0-target` | Target descriptions |
| `crates/a0-native` | Native code through Cranelift, linked by the system linker |
| `crates/a0-runtime` | The C runtime (`include/a0rt.h`, `c/`): output, strings, collections, `any`, the collector |
| `crates/a0-build` | Build-script helper for languages that ship C code (Kaleidoscope) |
| `crates/a0-driver` | The driver every example language uses (run, build, ir, hir, lsp) |
| `crates/a0-capi` | The C API: a static and a shared library, and `include/a0.h` |
| `bindings/python` | The Python package `a0` over the C API, and Pebble, a language in Python |
| `crates/a0` | The `a0` command-line tool |
| `b0` | B0, the flagship language, on the public API only |
| `examples/kaleidoscope`, `examples/lox`, `examples/minnow`, `examples/tin` | The example languages |
| `examples/calc`, `examples/lispy` | Languages defined by their grammars alone, run by `a0 lang` |
| `crates/a0-playground`, `playground` | The browser playground: its engine (built for WebAssembly) and its page |
| `editors/vscode` | A VS Code extension that starts each language's server |
| `crates/a0-wasm` | WebAssembly from the core IR, for the playground (programs link against its runtime) |
| `tests/programs`, `tests/hir` | Core IR and HIR programs with expected output, run by every backend |

## Building and testing

```
cargo build
cargo test
```

`cargo test` runs everything: every IR, HIR, Kaleidoscope, Minnow and Tin program and the Lox suite
through both backends and under GC stress, optimized and not; generated random programs, which must
agree in all those modes; snapshots of each language's rendered diagnostics and runtime errors; the
language servers; the C API from a C program; and the tutorial's listings and commands. CI runs the same on
x86-64 and aarch64 Linux, macOS and Windows, and runs executables cross-compiled with the target
packs.

## Status

Version 1.0.1 (`CHANGELOG.md`), stable: from 1.0 on, A0 follows Semantic Versioning and keeps
its command line, text formats, C API, Python package and the Rust API for languages compatible
until 2.0 ([docs/compatibility.md](docs/compatibility.md)). Builds for x86-64 and aarch64 Linux,
macOS and Windows, and for WebAssembly in the playground; cross-compiles to Linux and Windows from
any host; programs are packages with dependencies; every input A0 reads is fuzzed. Releases are
described in [docs/releases.md](docs/releases.md). A0 is MIT-licensed.
