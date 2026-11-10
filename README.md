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

You can try A0 in your browser with nothing to install, or on your own machine in a few commands.
Both ways run Calc, a small calculator language. All of Calc is one grammar file,
[`calc.a0g`](examples/calc/calc.a0g).

### In your browser

Open **[the playground](https://dan-k391.github.io/a0/)**. It is A0 itself, compiled to
WebAssembly and running in the page. Pick an example such as **Calc: loops**. The grammar is on
one side, a program on the other, and **Run** shows what the program prints. Change the grammar
and the language changes with it.

The **Tree**, **HIR** and **IR** buttons show the forms A0 turns the program into on its way to
running it. [The introduction](docs/book/introduction.md) explains each one.

### On your machine

You need two things:

- **Rust**, from [rustup.rs](https://rustup.rs).
- **A C compiler**, because A0's runtime is written in C. On Linux and macOS, A0 uses `cc`. On
  Windows, it uses Microsoft's compiler (MSVC), which it finds on its own.

Install the `a0` command:

```
git clone https://github.com/Dan-k391/a0
cd a0
cargo install --path crates/a0
```

This compiles A0, which takes a few minutes, and puts `a0` on your `PATH`.

Here is a Calc program, `examples/calc/programs/loops.calc`:

```
# Loops: compound interest, a table of squares and a countdown.
let total = 1000
repeat 10 {
  total = total * 1.05
}
print "after ten years:", round(total)

let i = 1
while i <= 4 {
  print i, i * i
  i = i + 1
}

let n = 3
repeat n {
  print n
  n = n - 1
}
if n == 0 { print "liftoff" } else if n < 0 { print "too far" } else { print "not yet" }
```

Run it. `a0 lang` takes a grammar, a command and a program:

```
a0 lang examples/calc/calc.a0g run examples/calc/programs/loops.calc
```

It prints:

```
after ten years: 1629
1 1
2 4
3 9
4 16
3
2
1
liftoff
```

`run` compiles the program to machine code in memory and runs it straight away. Compiling at the
moment of running like this is called JIT compilation.

To make an executable that runs without A0, use `build` instead:

```
a0 lang examples/calc/calc.a0g build examples/calc/programs/loops.calc -o loops
./loops
```

The executable prints the same lines. On Windows it is called `loops.exe`.

Now make a mistake. Put this in a file `bad.calc`:

```
let x = 1
print x + "a"
```

Check it without running it:

```
$ a0 lang examples/calc/calc.a0g check bad.calc
error[E2001]: mismatched types
 --> bad.calc:2:11
  |
2 | print x + "a"
  |       -   ^^^ expected `float`, found `string`
  |       |
  |       expected because this is `float`
```

`a0 explain E2001` says more about this kind of error.

Calc has no code besides its grammar. The parser, the type checking, the error message, the machine
code and the executable all came from A0.

**Without compiling A0:** the [release page](https://github.com/Dan-k391/a0/releases/latest) has
archives for Linux, macOS and Windows, on x86-64 and aarch64. Each has `a0`, the example
languages, a linker, and the target packs used to build executables for the other platforms. The
only other thing you need is a C compiler's linker, to build executables for your own machine
([releases](docs/releases.md)).

## How A0 runs a language

A program goes through the same steps in every language built on A0:

```
program ──grammar──▶ syntax tree ──@hir forms or your compiler──▶ HIR ──▶ core IR ──┬─ machine code: run now (JIT), or an executable
                                                                                  ├─ interpreter: the reference, and the debugger
                                                                                  └─ WebAssembly: the playground
```

1. A **grammar** describes what programs in the language look like. A0 reads it and parses each
   program into a **syntax tree**.
2. The tree is turned into A0's **high-level IR (HIR)**: a typed form of the program, with
   strings, arrays, structs, closures and garbage collection built in. A0 checks its types and
   optimizes it.
3. A0 lowers the HIR to its **core IR**, a small low-level form close to machine code.
4. A **backend** runs the core IR. Cranelift makes machine code, in memory or in an executable.
   An interpreter runs it directly, and gives the same output. WebAssembly runs it in a browser.

[The introduction](docs/book/introduction.md) follows one small program through all four steps.

Step 2 is the only one a language provides, and it can do it in two ways.

### A language in one file

Calc's grammar says what each rule means with an `@hir` form, and `a0 lang` does the rest. Here is
part of [`calc.a0g`](examples/calc/calc.a0g):

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

The first line says that a `while` statement is the word `while`, a condition and a block, and
that it means a `while` loop. `~ "expected a condition"` is the error to report when the condition
is missing. The `@prec` table lists the operators, loosest first.

The same grammar gives every command a language has:

```
a0 lang examples/calc/calc.a0g run   prog.calc    # compile into memory and run
a0 lang examples/calc/calc.a0g build prog.calc    # a native executable
a0 lang examples/calc/calc.a0g check prog.calc    # report errors without running
a0 lang examples/calc/calc.a0g hir   prog.calc    # print the HIR the program became
a0 lang examples/calc/calc.a0g repl               # an interactive prompt
a0 lang examples/calc/calc.a0g lsp                # a language server for your editor
```

[Lispy](examples/lispy) uses the same forms with a Lisp syntax. [Grammars](docs/grammar.md)
explains the notation and the `@hir` forms.

### A language with a compiler

Some languages need more than the `@hir` forms: types of their own, modules, or a runtime written
in C. For those, you write a compiler in Rust: a function that walks the syntax tree and builds
HIR. `a0 new` makes a project with a grammar, a compiler for a small typed language, its command,
tests that pass from the start, and settings for VS Code:

```
a0 new mylang
cd mylang
cargo run -- run examples/hello.mylang
cargo test
```

[Getting started](docs/book/getting-started.md) goes through this project and what it prints.
The compiler has three parts:

1. **A grammar** ([Grammars](docs/grammar.md)): tokens, rules, operator tables, error messages,
   error recovery, and annotations that tell editors where names are declared. Try a grammar with
   `a0 parse lang.a0g input`, which prints the syntax tree or the errors.
2. **A walk over the syntax tree that builds HIR** with `a0_hir::HirBuilder`
   ([the HIR](docs/hir.md)). The compiler builds by name (`let_named`, `call_named`,
   `named_type`, ...). The builder resolves the names, infers and checks the types, and reports
   the errors, so a typed language needs no checker of its own.
3. **A few lines that hand the compile function to `a0_driver::Language`.** That gives the
   language's command `run`, `build` (`-g` for gdb and lldb), `check`, `ir`, `hir`, `lsp`, `dap`,
   `repl` and `explain`.

[The tutorial](docs/tutorial.md) builds a language called Kaleidoscope this way, step by step. Its
listings are cut from the real code, and a test runs its commands. A language can also be written
in Python, over A0's C API, with the same commands ([A0 from Python](docs/python.md)).

## What every language gets

- **A parser from a grammar**, loaded when the language starts, with error messages and recovery,
  so one run reports every syntax error.
- **A high-level IR** with strings, arrays, maps, structs, enums with `match`, generics, closures,
  nullable references, dynamic (`any`) values, methods, interfaces, exceptions, and results with
  `?`. The builder infers and checks types and reports errors in the language's own syntax.
- **Backends that agree.** Native code comes from Cranelift: run at once from memory (the JIT) or
  linked into executables. An interpreter runs the same programs. All of them give the same
  output, errors and exit codes, which the tests check on every test program on Linux, macOS and
  Windows.
- **Cross-compilation**: executables for Linux and Windows from any host
  ([other platforms](docs/cross.md)).
- **A precise garbage collector**, shared by threads. A language can instead manage its own
  memory, with no collector at all.
- **An optimizer**, on by default, with no work in the language's compiler.
- **Systems features**, when a language wants them: sized numbers, C-layout structs as values, raw
  pointers, and C functions called directly, with structs passed by value.
- **Diagnostics as good as rustc's**: codes with explanations, related locations, "did you mean"
  fixes, warnings, and runtime errors with a stack trace of the program's functions.
- **Programs of several files, and packages**: imports found through the grammar, modules with
  visibility, manifests, dependencies and lock files, and reproducible builds
  ([packages](docs/packages.md)).
- **A standard library**: files, input, arguments, the environment, time, mathematics, strings,
  and threads with channels ([the standard library](docs/std.md)).
- **Tools**: a REPL, a language server, a debugger, a VS Code extension, and debug information for
  gdb and lldb. The language server gives diagnostics with quick fixes, highlighting, an outline,
  go to definition, hovers, completion, references and rename. The debugger has breakpoints,
  stepping and local variables.
- **A C API and Python bindings**, for programs that are not written in Rust
  ([A0 from C](docs/c-api.md)).

## Languages built on A0

| Language | What it shows | Size |
|---|---|---|
| [B0](b0) | The flagship: static types with inference, structs, enums, generics, interfaces, closures, exceptions, threads, `unsafe` pointers, C functions, packages, and a prelude written in B0. Built only on A0's public API. | ~1350 lines |
| [Kaleidoscope](examples/kaleidoscope) | The language of LLVM's tutorial, and what A0's tutorial builds | ~580 lines |
| [Minnow](examples/minnow) | A typed language with modules, generics, interfaces and closures, and [`json`](examples/minnow/json), a tool written in it | ~1000 lines |
| [Lox](examples/lox) | The language of the book *Crafting Interpreters*. It passes all 241 tests of the official suite, and beats clox, the book's fast implementation, on five of the book's six benchmarks | ~1500 lines |
| [Tin](examples/tin) | A C-like systems language without a garbage collector, which calls the C library cJSON as C would | ~850 lines |
| [Calc](examples/calc), [Lispy](examples/lispy) | Languages that are only a grammar | 54 and 41 lines |
| [Pebble](bindings/python/examples/pebble) | A typed language whose compiler is a Python file | one file |

Sizes include the grammar. B0 and the example languages with compilers each have a language
server, a debugger and a REPL, and are tested interpreted, in the JIT and natively.
[The language guides](docs/book/languages.md) say what each one is for and what to read in it.
To try them:

```
cargo build --release
target/release/b0 run b0/tests/programs/prelude.b0
target/release/minnow run examples/minnow/tests/programs/closures.mn
target/release/lox examples/lox/tests/craftinginterpreters/test/benchmark/fib.lox
target/release/kaleidoscope build examples/kaleidoscope/tests/programs/mandelbrot.k -o mandel && ./mandel
```

## Performance

A0 compiles through Cranelift, so its languages run at roughly the speed of C compiled with
`clang -O0`. A0's own work, from source to object file, takes 4–20 ms. Lox compiled with A0 beats
clox, the book's bytecode VM, on five of its six benchmarks. Kaleidoscope runs `mandel` slightly
faster than LLVM's tutorial does. A 100,000-line program builds in 1.3 s.
[Benchmarks](docs/benchmarks.md) has the numbers, the machine, and how to rerun them
(`bench/run.py`).

## Documentation

Start with [the book](docs/book/README.md). It goes from a first language to the inside of A0, and
links every page below.

**Learning A0**

| Page | What it covers |
|---|---|
| [Getting started](docs/book/getting-started.md) | Install A0, make a language with `a0 new`, and run it every way |
| [The tutorial](docs/tutorial.md) | Building Kaleidoscope, step by step |
| [The language guides](docs/book/languages.md) | The languages in this repository, and what each one teaches |

**What a language is made of**

| Page | What it covers |
|---|---|
| [Grammars](docs/grammar.md) | The grammar notation, and languages without host code |
| [The high-level IR](docs/hir.md) | The HIR: types, expressions, building it, lowering, optimization |
| [The core IR](docs/ir.md) | The low-level IR under the HIR, and its text format |
| [The standard library](docs/std.md) | The functions every language can use |
| [Diagnostics](docs/diagnostics.md) | Error messages: how they render, their codes, how a language makes them good |
| [Packages](docs/packages.md) | Manifests, dependencies, lock files, reproducible builds |

**Tools**

| Page | What it covers |
|---|---|
| [Editors](docs/editors.md) | The language server in VS Code, Neovim and Helix |
| [Debugging](docs/debugging.md) | The debugger, and native debug information |
| [The REPL](docs/repl.md) | The interactive prompt |
| [Other platforms](docs/cross.md) | Cross-compilation |
| [The playground](docs/playground.md) | The browser playground: how it works, and how to build it |
| [A0 from C](docs/c-api.md), [A0 from Python](docs/python.md) | Using A0 from C, and writing a language in Python |

**The project**

| Page | What it covers |
|---|---|
| [Benchmarks](docs/benchmarks.md) | Compile and run times against LLVM, C, Go and clox |
| [Compatibility](docs/compatibility.md) | What stays compatible until 2.0 |
| [Releases](docs/releases.md) | Release archives and packages |
| [Decisions](docs/decisions.md), [Milestones](docs/milestones.md) | Design decisions and why; what was built when |
| [CHANGELOG.md](CHANGELOG.md) | Releases |
| [AGENTS.md](AGENTS.md) | Working on A0: commands, layer rules, conventions |

<details>
<summary><b>The <code>a0</code> command</b></summary>

```
a0 new mylang                             # start a language project
a0 lang calc.a0g run prog.calc            # a grammar-only language (also build, check, hir, ir, repl, lsp)
a0 parse lang.a0g input.txt               # try a grammar: print the tree or the errors
a0 explain E2001                          # what an error code means
a0 --version                              # the installed A0 version

a0 run prog.hir                           # HIR and core IR programs: run (JIT; A0_INTERP=1
a0 build prog.a0ir                        #   interprets), build, check, fmt
a0 build --target x86_64-windows prog.hir # an executable for another platform
a0 obj --target aarch64-linux prog.a0ir   # an object file for another target
a0 lower prog.hir                         # the core IR a HIR module lowers to
```

`A0_OPT=0` turns the HIR optimizer off. `A0_GC_STRESS=1` runs the collector on every allocation.
A language built on `a0_driver` has the same commands in its own command: `mylang run`, `build`,
`check`, `ir`, `hir`, `repl`, `lsp`, `dap` and `explain`
([Getting started](docs/book/getting-started.md)).

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

`cargo test` runs everything:

- every IR, HIR and example-language program, and the Lox suite: interpreted, in the JIT and
  natively, under GC stress, with the optimizer on and off;
- random programs, which must agree in all those modes;
- fuzzing of every input A0 reads;
- the diagnostic snapshots;
- the language servers, debuggers and REPLs;
- the C API and the Python bindings;
- the tutorial's listings.

CI runs it on x86-64 and aarch64 Linux, macOS and Windows. [AGENTS.md](AGENTS.md) has the rest.

## Status

**1.0.0**, stable. A0 follows Semantic Versioning: the command line, the text formats, the C API,
the Python package and the Rust API for languages stay compatible until 2.0
([compatibility](docs/compatibility.md)). A language project depends on A0 from git, pinned to a
release (`a0-hir = { git = "https://github.com/Dan-k391/a0", tag = "v1.0.0" }`), as `a0 new` sets
up. A0 is MIT-licensed.
