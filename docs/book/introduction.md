# Introduction

A0 is a framework for building programming languages. A language on A0 is a grammar and a
compiler from the grammar's syntax trees to A0's high-level IR (HIR); everything after that is
A0's:

```
source ──grammar (a0-syntax)──▶ syntax tree ──your compiler──▶ HIR ──a0-hir──▶ core IR ─┬─ interpreter
                                                                                       ├─ JIT (lang run)
                                                                                       ├─ native executables, any target
                                                                                       └─ WebAssembly
```

- **The grammar** (`docs/grammar.md`) is a file of tokens and rules. A0 makes a lexer and a parser
  from it, with error recovery and good messages, and syntax trees with named fields. Annotations
  on its rules (`@def`, `@scope`, `@import`) tell the language server what declares names and
  where scopes are, and the driver how files import each other.
- **The HIR** (`docs/hir.md`) is a typed tree: numbers, strings, arrays, maps, structs (by
  reference or inline), enums, closures, generics, interfaces, nullable references, `any`,
  exceptions, raw pointers and C functions. A language builds it by name: the builder resolves
  names in scopes, infers and checks types, and reports errors in the language's own words. A
  compiler is often one walk over the syntax tree and no checker of its own.
- **Everything else** comes with it: the collector (or memory the program frees itself), threads,
  an optimizer, an interpreter that is the reference, a JIT for `run`, native executables through
  Cranelift for every supported platform, debug information, a language server, a debugger, a
  REPL, packages with dependencies, and a standard library.

A language gets all of this through the driver (`a0-driver`): `Language::new("mylang", compile)`
is a command with `run`, `build`, `check`, `hir`, `ir`, `lsp`, `dap`, `repl` and `explain`.

## The languages in this repository

| Language | What it shows | Guide |
|---|---|---|
| B0 | everything at once: the flagship, on the public API only | [`b0/README.md`](../../b0/README.md), [Building a full language](b0.md) |
| Minnow | a typed language with modules, generics and interfaces | [`examples/minnow/README.md`](../../examples/minnow/README.md) |
| Lox | a dynamic language, from *Crafting Interpreters* | [`examples/lox/README.md`](../../examples/lox/README.md), [Dynamic languages](dynamic.md) |
| Kaleidoscope | LLVM's tutorial language | [the tutorial](../tutorial.md) |
| Tin | a systems language without a collector | [`examples/tin/README.md`](../../examples/tin/README.md), [Memory](memory.md) |
| Calc, Lispy | languages defined by their grammar alone | [`examples/calc/README.md`](../../examples/calc/README.md) |
| Pebble | a language written in Python | [`docs/python.md`](../python.md) |

[The language guides](languages.md) say what to read in each.

## How to read this book

[Getting started](getting-started.md) makes a language with `a0 new` and runs it. [The
tutorial](../tutorial.md) then builds Kaleidoscope step by step, every listing cut from the real
code. Part II is the reference for what a language is made of; Part III is about tools, which a
language gets without code of its own; Part IV goes further: a full language, memory and systems
programming, threads, dynamic languages, performance, other platforms, and hosting A0 from C and
Python. Part V is the reference: the compatibility promise, the API, the decisions behind A0.
