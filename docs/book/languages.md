# The language guides

Every language in this repository is a guide to building one kind of language on A0. Each has a
README with the language, its files and its commands, and tests that run its programs in every
mode. What to read in each:

## B0, the full language

[`b0/README.md`](../../b0/README.md) is the language; [Building a full language](b0.md) is how its
compiler works. Read `b0/src/lib.rs` for a compiler that builds everything by name, `b0/b0.a0g`
for a grammar of a whole language, `b0/src/prelude.b0` for library code in the language itself,
and `b0/examples/wordfreq` for a program of several modules. B0 uses only the covered API.

## Minnow, a typed language

[`examples/minnow/README.md`](../../examples/minnow/README.md). The language B0 grew from: modules,
generics with bounds, interfaces, closures, nullable references with `if let` and `?`, and the
standard library, in one walk of about 900 lines. `examples/minnow/json` is a JSON tool written in
it, a real program of several modules.

## Lox, a dynamic language

[`examples/lox/README.md`](../../examples/lox/README.md) and [Dynamic languages](dynamic.md). Lox
from *Crafting Interpreters*, passing all 241 tests of the official suite in every mode, with
jlox's exact messages and exit codes. Read `src/analyze.rs` for a language's own checks (jlox's
resolver) before HIR is built, `src/emit.rs` for HIR built by handle, and `prelude.hir` for
semantics written in HIR.

## Kaleidoscope, the tutorial's language

[The tutorial](../tutorial.md) builds it step by step from LLVM's tutorial;
[`examples/kaleidoscope/README.md`](../../examples/kaleidoscope/README.md) lists how it differs.
Read it for user-defined operators parsed with `resolve_precedence`, and for a language with a C
runtime of its own (`runtime/kaleidoscope.c`, `a0-build`, `Language::runtime`).

## Tin, a systems language

[`examples/tin/README.md`](../../examples/tin/README.md) and [Memory](memory.md). C without its
preprocessor: structs as values, raw pointers, `alloc` and `free`, no collector, C functions called
directly and structs passed to them by value. It ships cJSON and calls it as C would.

## Calc and Lispy, languages without host code

[`examples/calc/README.md`](../../examples/calc/README.md) and
[`examples/lispy/README.md`](../../examples/lispy/README.md). A grammar whose rules name HIR forms
(`@hir`, `docs/grammar.md`) is a language by itself, run by `a0 lang`, with no Rust at all. Two
grammars with the same forms are two syntaxes for one language. They run in the browser
playground (`docs/playground.md`).

## Pebble, a language in Python

[`docs/python.md`](../python.md) and `bindings/python/examples/pebble`. A typed language whose
compiler is about 230 lines of Python over the C API, with the same driver: native executables, the
language server and the debugger.
