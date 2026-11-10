# The A0 book

A0 is a framework for building programming languages. You describe a language's syntax in a
grammar, and A0 gives the language a compiler, native executables, garbage collection, error
messages, a language server, a debugger and a REPL.

This book starts with making your first language and ends with the inside of A0. The code in it is
real: every listing in the tutorial is cut from code that is built and tested, and a test runs
every command the tutorial shows.

## Where to start

- **New to A0?** Read the [introduction](introduction.md) for what A0 is and how its parts fit
  together. Then follow [Getting started](getting-started.md), which makes a language with one
  command and runs it.
- **Want to learn by building?** [The tutorial](../tutorial.md) builds a complete small language,
  Kaleidoscope, one piece at a time.
- **Looking something up?** Part II is the reference for what a language is made of, and Part V
  is the reference for the rest.

## Contents

**Part I: Starting**

1. [Introduction](introduction.md): what A0 is, how its parts fit together, and the languages
   built on it
2. [Getting started](getting-started.md): install A0, make a language with `a0 new`, and run it
   every way
3. [The tutorial](../tutorial.md): Kaleidoscope, the language of LLVM's tutorial, step by step

**Part II: What a language is made of**

4. [Grammars](../grammar.md): tokens, rules, operator precedence, error recovery, names and
   scopes for editors, imports, and languages made of a grammar alone
5. [The high-level IR](../hir.md): A0's typed form of a program: its types and expressions, the
   builder, modules and the optimizer
6. [Diagnostics](../diagnostics.md): error messages as good as rustc's, in the language's own words
7. [The standard library](../std.md): files, input and output, strings, mathematics, threads
8. [Packages](../packages.md): manifests, dependencies, lock files

**Part III: Tools every language gets**

9. [Editors](../editors.md): the language server, in VS Code, Neovim and Helix
10. [Debugging](../debugging.md): the debugger, stack traces, native debug information
11. [The REPL](../repl.md): the interactive prompt

**Part IV: Going further**

12. [Building a full language: B0](b0.md)
13. [Memory: the collector, manual memory and C](memory.md)
14. [Concurrency: threads and channels](concurrency.md)
15. [Dynamic languages](dynamic.md)
16. [Performance](../benchmarks.md): what the optimizer and the backends do, measured
17. [Other platforms](../cross.md): cross-compilation, and [the browser](../playground.md)
18. [The core IR](../ir.md) and [the runtime](../runtime.md), the layers under the high-level IR
19. [A0 from C](../c-api.md) and [from Python](../python.md)

**Part V: Reference**

20. [The language guides](languages.md): what each example language is for, and what to read in it
21. [Compatibility](../compatibility.md): what 1.x keeps working, and how things are retired
22. [Releases](../releases.md): what a release contains, and how it is made
23. The API reference, built from the code:
    `cargo doc --no-deps -p a0-driver -p a0-hir -p a0-syntax -p a0-diag -p a0-ir -p a0-sema -p a0-build --open`
24. [Decisions](../decisions.md): why A0 is the way it is, each decision with its alternatives
25. [Milestones](../milestones.md) and [the changelog](../../CHANGELOG.md): what was built when

## Reading it

The book is plain Markdown, so it reads on GitHub as it is. To read it as a website instead,
install [mdBook](https://rust-lang.github.io/mdBook/) and run `mdbook build docs`;
[`SUMMARY.md`](../SUMMARY.md) is its table of contents.
