# The language guides

A0 comes with eight languages. Each one is an example of a kind of language you might build, and
shows how to build that kind on A0. Pick the one closest to yours:

| To build | Read | Size |
|---|---|---|
| a complete statically typed language | [B0](#b0-the-full-language) | ~1350 lines |
| a typed language, smaller | [Minnow](#minnow-a-typed-language) | ~1000 lines |
| a dynamically typed language | [Lox](#lox-a-dynamic-language) | ~1500 lines |
| your first language, step by step | [Kaleidoscope](#kaleidoscope-the-tutorials-language) | ~580 lines |
| a systems language, without a garbage collector | [Tin](#tin-a-systems-language) | ~850 lines |
| a language from a grammar alone, with no code | [Calc and Lispy](#calc-and-lispy-languages-without-host-code) | 54 and 41 lines |
| a language whose compiler is in Python | [Pebble](#pebble-a-language-in-python) | one file |

Sizes count the grammar and the compiler. Each language has a page that describes the language,
its files and its commands. Each has tests that run its programs interpreted, in the JIT and as
native executables, and check that all three give the same output.

## B0, the full language

**For:** seeing everything A0 offers in one statically typed language, built the way your own
would be. B0 uses only A0's public API, the part [Compatibility](../compatibility.md) promises
to keep working.

- [`b0/README.md`](../../b0/README.md) describes the language.
- [Building a full language](b0.md) explains how its compiler works.

What to read in it:

| File | What it shows |
|---|---|
| `b0/src/lib.rs` | a compiler that builds everything by name, and lets the HIR builder resolve the names and check the types |
| `b0/b0.a0g` | the grammar of a whole language |
| `b0/src/prelude.b0` | library code written in the language itself |
| `b0/examples/wordfreq` | a program of several modules |

## Minnow, a typed language

**For:** a statically typed language that is smaller than B0, and easier to read in one sitting.
B0 grew from it.

[`examples/minnow/README.md`](../../examples/minnow/README.md) describes the language. It has
modules, generics with bounds, interfaces, closures, nullable references with `if let` and `?`,
and the standard library. Its compiler is one walk over the syntax tree, about 900 lines.
`examples/minnow/json` is a JSON tool written in Minnow, a real program of several modules.

## Lox, a dynamic language

**For:** a dynamically typed language, where every value's type is known only when the program
runs.

- [`examples/lox/README.md`](../../examples/lox/README.md) describes it.
- [Dynamic languages](dynamic.md) explains how a dynamic language runs on A0's typed HIR.

Lox is the language of the book *Crafting Interpreters*. This one passes all 241 tests of the
book's official suite, in every mode, with the exact error messages and exit codes of jlox, the
book's first implementation.

What to read in it:

| File | What it shows |
|---|---|
| `examples/lox/src/analyze.rs` | a language's own checks, run before any HIR is built: jlox's resolver, which finds where each variable lives |
| `examples/lox/src/emit.rs` | HIR built from handles to variables and functions, instead of by name |
| `examples/lox/prelude.hir` | the language's run-time behavior (values, calls, classes, errors) written in HIR text |

## Kaleidoscope, the tutorial's language

**For:** learning A0 one piece at a time. Kaleidoscope is the language of LLVM's tutorial.

[The tutorial](../tutorial.md) builds it step by step.
[`examples/kaleidoscope/README.md`](../../examples/kaleidoscope/README.md) lists how it differs
from LLVM's version.

Read it also for two things the other languages do not show:

- **Operators the program defines itself.** Their precedence is known only after parsing, so the
  grammar parses them as a flat chain, and the compiler orders them with `resolve_precedence`.
- **A C runtime of its own.** `runtime/kaleidoscope.c` is compiled with `a0-build` and given to
  the driver with `Language::runtime`.

## Tin, a systems language

**For:** a language with manual memory and direct access to C.

- [`examples/tin/README.md`](../../examples/tin/README.md) describes it.
- [Memory](memory.md) explains manual memory and calling C.

Tin is C without its preprocessor. It has structs as values, raw pointers, `alloc` and `free`,
and no garbage collector. It calls C functions directly and passes structs to them by value. It
ships the C library cJSON and calls it as C would.

## Calc and Lispy, languages without host code

**For:** a language made of a grammar alone, with no Rust or other code.

- [`examples/calc/README.md`](../../examples/calc/README.md)
- [`examples/lispy/README.md`](../../examples/lispy/README.md)

Each grammar rule names the HIR form it means, with `@hir` ([Grammars](../grammar.md)), and
`a0 lang` runs the language. Calc and Lispy use the same forms with different syntax, so they are
two syntaxes for one language. They are the examples in
[the browser playground](../playground.md).

## Pebble, a language in Python

**For:** writing a language in Python instead of Rust.

- [A0 from Python](../python.md) explains it.
- The code is in `bindings/python/examples/pebble`.

Pebble is a typed language whose compiler is about 230 lines of Python over A0's C API. It uses the
same driver as the other languages, so it has native executables, the language server and the
debugger too.
