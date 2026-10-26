# The A0 book

A0 is a framework for building programming languages. This book goes from making a first language
to the inside of the framework. Every listing in the tutorial is cut from code that is built and
tested, and every command in it is run by a test.

**Part I: Starting**

1. [Introduction](introduction.md): what A0 is, and the languages built on it
2. [Getting started](getting-started.md): build A0, make a language with `a0 new`, run it every way
3. [The tutorial](../tutorial.md): Kaleidoscope, from LLVM's tutorial, step by step

**Part II: What a language is made of**

4. [Grammars](../grammar.md): tokens, rules, precedence, recovery, `@def`, `@scope`, `@import`, `@hir`
5. [The high-level IR](../hir.md): types, expressions, the builder, modules, the optimizer
6. [Diagnostics](../diagnostics.md): errors as good as rustc's, in the language's own words
7. [The standard library](../std.md): files, input and output, strings, mathematics, threads
8. [Packages](../packages.md): manifests, dependencies, lock files

**Part III: Tools every language gets**

9. [Editors](../editors.md): the language server, VS Code, Neovim, Helix
10. [Debugging](../debugging.md): the debugger, stack traces, native debug information
11. [The REPL](../repl.md)

**Part IV: Going further**

12. [Building a full language: B0](b0.md)
13. [Memory: the collector, manual memory and C](memory.md)
14. [Concurrency: threads and channels](concurrency.md)
15. [Dynamic languages](dynamic.md)
16. [Performance](../benchmarks.md): what the optimizer and the backends do, measured
17. [Other platforms](../cross.md): cross-compilation, and [the browser](../playground.md)
18. [The core IR](../ir.md) and [the runtime](../runtime.md), under the HIR
19. [A0 from C](../c-api.md) and [from Python](../python.md)

**Part V: Reference**

20. [The language guides](languages.md)
21. [Compatibility](../compatibility.md): what 1.x keeps working, and how things are retired
22. [Releases](../releases.md): what a release contains, and how it is made
23. The API reference: `cargo doc --no-deps -p a0-driver -p a0-hir -p a0-syntax -p a0-diag -p a0-ir -p a0-sema -p a0-build --open`
24. [Decisions](../decisions.md): why A0 is the way it is, D1 to the latest
25. [Milestones](../milestones.md) and [the changelog](../../CHANGELOG.md)

The book is plain Markdown, readable on GitHub; `docs/SUMMARY.md` lays it out for mdBook too
(`mdbook build docs`).
