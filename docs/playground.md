# The playground

The playground at **https://dan-k391.github.io/a0/** runs A0 in your browser, with nothing to
install. It has three panes:

- **Grammar**: a grammar that defines a small language.
- **Program**: a program in that language.
- **Output**: what the program prints, or its errors.

Pick an example from the **Example** list and press **Run** (or Ctrl+Enter). For instance, the
example "Calc: functions" is this program:

```
# Functions, called before or after they are defined, recursion and the standard mathematics.
print hypot(3, 4)
print fib(20)

fn hypot(a, b) {
  return sqrt(a * a + b * b)
}

fn fib(n) {
  if n < 2 { return n }
  return fib(n - 1) + fib(n - 2)
}

fn greet(who) {
  print "hello,", who
}

greet("calc")
print floor(pi() * 100) / 100, max(2, 3), round(2.5)
```

Run prints:

```
5
6765
hello, calc
3.14 3 3
```

## What you can do there

**Change the program**, and run it again. Errors are shown as the `a0` command shows them, pointing
at the program (or at the grammar, when the grammar has the error). For example, a call to a
function that does not exist:

```
error[E1001]: unknown function `nope`
 --> program:2:7
  |
2 | print nope(1)
  |       ^^^^ not found in this scope
```

**Change the grammar**, and the language changes. Each rule of the grammar says what its part of a
program means with an `@hir` form, for example "this is a call" or "this is a loop"
([languages without host code](grammar.md#languages-without-host-code-hir) lists the forms). This
is the same kind of grammar that `a0 lang` runs. The examples are two languages, Calc and Lispy,
which use the same forms with different syntax: Lispy writes everything in parentheses.

**See what A0 made of the program** with the other buttons:

| Button | Shows |
|---|---|
| **Tree** | the program's syntax tree, as the grammar parsed it |
| **HIR** | the program in A0's [high-level IR](hir.md), the typed form the grammar's forms build |
| **IR** | the program in A0's [core IR](ir.md), the low-level form the HIR becomes |

**Choose how it runs.** Run compiles the program to WebAssembly, which the browser runs at close to
native speed. Tick **Interpret** to run it in A0's interpreter instead. The status after a run
gives the exit status, the time it took, and "compiled" when it was compiled.

**Stop a program** that does not end: while it runs, the Run button says **Stop**.

**Share your work.** **Copy link** copies a link that holds the grammar and the program. The page
also remembers what you typed, in your browser, for the next visit.

## Limits

- Programs cannot read files, and standard input is empty. `std` functions that need them fail as
  they would on a system without them.
- Programs cannot start threads.
- Calls nest at most 1,000 deep in the interpreter, which runs on the browser's own stack (about
  1 MB), and 10,000 deep when compiled. Deeper recursion stops the program with
  `panic: stack overflow` and exit status 101.
- Compiled programs cannot pass C structs by value. C functions they declare must use the sizes C
  types have on 32-bit WebAssembly, where `size_t` and `long` are 32 bits.
- Output is shown up to about a million characters (2<sup>20</sup>).
- `clock()` gives the time since the program started, because WASI has no processor clock.

## Building it

This part is for working on the playground itself. The page is published from the `main` branch
by `.github/workflows/pages.yml`, and CI builds and tests it on every change.

You need:

- the Rust target for WebAssembly: `rustup target add wasm32-wasip1`;
- [wasi-sdk](https://github.com/WebAssembly/wasi-sdk), clang with a C library for WebAssembly,
  which compiles A0's C runtime (CI uses release 34);
- Node, for the tests.

The build works on Linux and macOS. On Windows, build it in WSL.

```
WASI_SDK=/path/to/wasi-sdk sh playground/build.sh
node playground/test.mjs target/playground
python3 -m http.server -d target/playground
```

The first command builds the playground into `target/playground` (or
`$CARGO_TARGET_DIR/playground`) and ends with `built target/playground`. The second runs every
example the way the page does, interpreted and compiled, and every program of the IR and HIR test
corpus compiled to WebAssembly, against its expected output, stack traces included. It prints a
line for each, such as `ok   Calc: functions (compiled)`. The third serves the page; open
http://localhost:8000.

## How it works

- **The engine** is `crates/a0-playground`, a small program built for WebAssembly
  (wasm32-wasip1). It has A0's grammar layer, the HIR builder and lowering, the interpreter, the
  WebAssembly backend and the C runtime, but no native backend. It takes the grammar and the
  program as arguments, since a page has no files, and a mode: `run`, `tree`, `hir`, `ir` or
  `wasm`. Built for your own machine, it is tested like any other crate.
- **The page** (`playground/`) is static files: `index.html`; `app.js` for the editors; `worker.js`,
  which runs the engine in a Web Worker so that a program that never ends can be stopped; and
  `wasi.js`, the few WASI calls the engine makes (arguments, standard output and error, clocks,
  randomness), in about 120 lines. Each run starts a fresh instance of the engine, so the runtime's
  state starts anew.
- **Compiled programs.** `a0-playground wasm` compiles the program to a WebAssembly module
  (`crates/a0-wasm`) and writes it on standard output. The module is not a whole program: it
  imports A0's runtime from an instance of the engine. The engine exports its runtime functions
  (built with default visibility, and the C library's functions by name), its memory and its
  function table (`playground/build.sh`). `compiled.js` makes a fresh engine instance, allocates
  the memory the module asks for and grows the table (the module's custom section `a0.layout`
  says how much), instantiates the module with both, and calls `a0_wasm_main`.
- **The runtime on 32 bits.** Generated code reads a few runtime fields as 64-bit words: an
  object's type in the collector's header, a type's family and interface table, an array's
  elements. On wasm32 a pointer is 4 bytes, so `a0rt.h` declares those fields with `A0_WORD_PTR`:
  a pointer in the low half of a word whose high half stays zero. The layout is the same on every
  target, and static checks in `gc.c` keep it so.
- **Calls into C.** The interpreter calls C functions with every argument as a 64-bit word,
  through a function pointer of one generic type. Natively the C compiler casts the pointer. In
  WebAssembly a call through a pointer of the wrong type traps, so the symbol table holds wrappers
  that take the words and convert them to the real parameter types. `crates/a0-runtime/build.rs`
  generates them from the prototypes in `a0rt.h`.
