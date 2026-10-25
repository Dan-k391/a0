# A0

[![CI](https://github.com/Dan-k391/a0/actions/workflows/ci.yml/badge.svg)](https://github.com/Dan-k391/a0/actions/workflows/ci.yml)

A0 is a framework for building programming languages. You write a grammar and a compiler from its
syntax trees to A0's high-level IR; your language gets an interpreter, native executables, garbage
collection, an optimizer, error messages in its own terms, and editor support.

```
grammar ──a0-syntax──▶ syntax tree ──your compiler──▶ HIR ──a0-hir──▶ core IR ──┬─ a0-interp ──▶ run it
                                                                                └─ a0-native ──▶ executable
```

## What a language gets

- **A parser from a grammar file**, loaded at run time: tokens, rules, operator precedence tables,
  error messages (`~ "expected ')'"`) and error recovery, so one run reports every syntax error.
- **A high-level IR** with strings, arrays, maps, structs, enums with `match`, generics, closures,
  nullable references (narrowed with `if let`) and dynamic (`any`) values. The builder resolves
  names, infers and checks types and reports errors in the language's own syntax, so a typed
  language needs no checker.
- **Two backends that agree**: an interpreter, and native code through Cranelift linked by the
  system linker, on Linux (x86-64, aarch64), macOS (aarch64, x86-64) and Windows (x86-64). Same
  output, errors and exit codes, checked on every test program.
- **A precise garbage collector** and an **optimizer**, on by default, with no work in the frontend.
- **A command-line driver** (`run`, `build`, `ir`, `hir`) and **a language server** (`lsp`) for
  diagnostics, highlighting, outline, go-to-definition and hover, with a VS Code extension.
- **A C API**, for hosts that are not written in Rust: a C program can build HIR with the same
  builder, run it and compile it.

The example languages show the scale. [Kaleidoscope](examples/kaleidoscope), the language of
LLVM's tutorial, is about 580 lines including its grammar and is what the tutorial builds;
[Minnow](examples/minnow), a typed language with structs, enums, generics, closures and nullable
references, is about 760; [Lox](examples/lox), from *Crafting Interpreters*, passes all 241 tests of
the official suite, and its native programs run faster than they did with a hand-written C
runtime.

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
target/release/lox examples/lox/tests/craftinginterpreters/test/benchmark/fib.lox
target/release/kaleidoscope build examples/kaleidoscope/tests/programs/mandelbrot.k -o mandel
./mandel
```

Then:

- **[The tutorial](docs/tutorial.md)** builds Kaleidoscope (LLVM's tutorial language) step by step.
  Its listings are cut from the example's code and its commands are run by a test, so what you read
  is what runs.
- **[Minnow](examples/minnow)** is the model for a typed language, **[Lox](examples/lox)** for a
  dynamically typed one, on the high-level IR.
- **[docs/editors.md](docs/editors.md)** sets up the language server in VS Code, Neovim or Helix.

## Writing a language

1. Write a grammar (`docs/grammar.md`): tokens, rules, `@prec` operator tables, `~ "message"` error
   messages, `@recover` for error recovery, and `@def`/`@scope` to mark declarations and scopes for
   editors. Try it with `a0 parse lang.a0g input`.
2. Walk the `SyntaxTree` once and build HIR with `a0_hir::HirBuilder` (`docs/hir.md`). Build by name
   (`let_named`, `name`, `call_named`, `named_type`, ...) and the builder resolves names, infers
   and checks types and reports errors. Runtime support can be written in HIR itself and parsed with
   `a0_hir::parse`. A language that needs full control can emit the core IR (`docs/ir.md`) with
   `a0_ir::Builder` instead.
3. Give `a0_driver::Language` your compile function and grammar. It provides `run` (interpreter),
   `build` (native executable), `ir`, `hir` and `lsp`, renders diagnostics and maps errors to your
   exit codes.

A0 is not published on crates.io; depend on it from git (`a0-hir = { git =
"https://github.com/Dan-k391/a0", tag = "v0.1.0" }`) or work inside this repository.

## Documentation

| Document | What it covers |
|---|---|
| [docs/tutorial.md](docs/tutorial.md) | Building Kaleidoscope, step by step |
| [docs/grammar.md](docs/grammar.md) | The grammar language |
| [docs/hir.md](docs/hir.md) | The high-level IR: types, expressions, building it from a frontend, lowering, optimization |
| [docs/ir.md](docs/ir.md) | The core IR and its text format |
| [docs/editors.md](docs/editors.md) | The language server and editor setup |
| [docs/c-api.md](docs/c-api.md) | Using A0 from C |
| [docs/decisions.md](docs/decisions.md) | Design decisions and why |
| [docs/milestones.md](docs/milestones.md) | What was built when, with measurements and open questions |
| [CHANGELOG.md](CHANGELOG.md) | Releases |
| [AGENTS.md](AGENTS.md) | How to work on A0: commands, layer rules, conventions |

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
| `crates/a0-capi` | The C API: a static library and `include/a0.h` |
| `crates/a0` | The `a0` command-line tool |
| `examples/kaleidoscope`, `examples/lox`, `examples/minnow` | The example languages |
| `editors/vscode` | A VS Code extension that starts each language's server |
| `tests/programs`, `tests/hir` | Core IR and HIR programs with expected output, run by both backends |

## Building and testing

```
cargo build
cargo test
```

`cargo test` runs everything: every IR, HIR, Kaleidoscope and Minnow program and the Lox suite
through both backends and under GC stress, optimized and not; the compile-error tests; the language
servers; the C API from a C program; and the tutorial's listings and commands. CI runs the same on
x86-64 and aarch64 Linux, aarch64 macOS and x86-64 Windows.

## Status

Version 0.1.0 (`CHANGELOG.md`). Not yet: cross-compiled executables (object files for other
targets work), wasm, bindings other than C, aarch64 Windows. A0 is MIT-licensed.
