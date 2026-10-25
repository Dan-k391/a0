# Changelog

Notable changes to A0. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and versions follow [Semantic Versioning](https://semver.org/); before 1.0, a minor version may change
APIs. `docs/milestones.md` has the details of each step, with measurements.

## [Unreleased]

## [0.1.0] - 2026-10-24

The first release: a framework for building languages, from a grammar to native executables, with
three example languages, editor support and a C API.

### Added

- **Grammar layer** (`a0-syntax`): a grammar language loaded at run time, a longest-match lexer, a
  PEG parser with `@prec` operator tables, `~ "message"` errors and `@recover` error recovery, and
  uniform syntax trees with labeled children. `@def` and `@scope` annotations declare names, and
  `SyntaxIndex` classifies tokens and resolves names from them.
- **Core IR** (`a0-ir`): a typed, structured IR with mutable variables, a builder that records
  misuse as diagnostics, a text format that round-trips exactly, and a verifier.
- **Backends**: an interpreter (`a0-interp`) and native code through Cranelift (`a0-native`),
  linked by the system linker, that agree on output, errors and exit codes. Executables for Linux
  (x86-64, aarch64), macOS (aarch64, x86-64) and Windows (x86-64); object files for each of them
  from any host.
- **Runtime** (`a0-runtime`, `a0rt`): output, traps, strings, arrays, insertion-ordered maps,
  NaN-boxed `any` values with inline 48-bit integers, and a precise mark-sweep garbage collector
  with a shadow stack and size-class free lists. `A0_GC_STRESS=1` collects on every allocation.
- **High-level IR** (`a0-hir`): strings, arrays, maps, structs, enums with `match`, generic
  functions, structs and enums (monomorphized by A0), closures, nullable references (`T?`, `e!`,
  narrowed with `if let`) and `any`. The builder infers types, resolves names and reports errors
  once, in the language's own type syntax; a text format; lowering to the core IR with precise GC
  rooting, in which code without references makes no runtime calls.
- **Optimizer**: inlining, decisions on `any` tests and casts, and simplification on the HIR, on by
  default for every language; `A0_OPT=0` turns it off.
- **Driver** (`a0-driver`): `run`, `build`, `ir`, `hir` and `lsp` for any language, with
  rustc-style diagnostics or a language's own error format.
- **Language server**: diagnostics, semantic highlighting, outline, go-to-definition and hover for
  every language, from its grammar and compiler, and a VS Code extension (`editors/vscode`).
- **C API** (`a0-capi`, `include/a0.h`): parse with grammars, build HIR (by handle or by name, with
  the builder's name resolution and type checking) and core IR, print, parse, run and compile
  modules from C. A C program in the tests compiles a small language through it on every platform.
- **Example languages**: Kaleidoscope (LLVM's tutorial language, about 580 lines), Lox (all 241 tests
  of the *Crafting Interpreters* suite, interpreted, native and under GC stress; natively faster than
  with its earlier hand-written C runtime on every benchmark of the book) and Minnow (a typed
  language with structs, enums, generics, closures and nullable references in about 760 lines), all
  three on the HIR.
- **Documentation**: a tutorial that builds Kaleidoscope step by step, whose listings are cut from
  the example's code and whose commands are run by a test; guides to the grammar language, the core
  IR, the HIR, editors and the C API; design decisions and milestone summaries.
- **CI** on x86-64 and aarch64 Linux, aarch64 macOS and x86-64 Windows, with rustfmt and clippy.

[Unreleased]: https://github.com/Dan-k391/a0/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/Dan-k391/a0/releases/tag/v0.1.0
