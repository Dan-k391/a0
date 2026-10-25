# Changelog

Notable changes to A0. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and versions follow [Semantic Versioning](https://semver.org/); before 1.0, a minor version may change
APIs. `docs/milestones.md` has the details of each step, with measurements.

## [Unreleased]

### Added

- **`a0 new <name>`** creates a language project: grammar, compiler, driver, tests of its programs
  (interpreted and native) and of its language server, an example and VS Code settings; `cargo
  test` passes in it from the start.
- **Languages without host code**: grammar rules give their nodes HIR forms (`@hir(let)`, or a
  rule named `function`, `block`, `binary`, ...), and `a0 lang <grammar.a0g> run|build|hir|ir|lsp`
  runs such a language with both backends and a language server. Calc (`examples/calc`) is one.
- **Python bindings** (`bindings/python`, `docs/python.md`): the package `a0` over the C API with
  ctypes (grammars, trees, both builders, running and native executables, and a driver), and
  Pebble, a language written in Python. The C API is also a shared library, and gained
  `a0_diagnostics_render` and `a0_hir_text`.
- The VS Code extension expands `${workspaceFolder}` in commands and their `args` (which replace
  `lsp`), and colors keywords, strings, numbers and comments of every language.

## [0.2.0] - 2026-10-25

Real programs: languages built on A0 can write programs of several files that work with files,
handle errors and use abstractions.

### Added

- **`json`**, a JSON tool in Minnow (`examples/minnow/json`): reading, pretty and compact writing,
  path queries and statistics over the standard library, in five modules. What writing it revealed
  is fixed below.
- `A0_GC_STATS=1` prints the collector's work at exit. Minnow has maps (`map<K, V>`, `new map {}`).
- **A standard library** for every language (`docs/std.md`): `std.io` (standard input and
  output), `std.fs` (files and directories), `std.env` (arguments, environment variables),
  `std.time`, `std.math` (the C library's functions, random numbers) and `std.str` (searching,
  splitting, joining, trimming, parsing numbers, bytes and UTF-8), returning `std.Result` for what
  can fail. It is HIR over runtime functions, added to a program only when imported.
- **C functions that take strings and arrays**: `extern fn getenv(string) -> string?;` just works,
  interpreted and native; the interpreter finds C library functions without registering them.
  `extern "a0" fn` declares runtime functions that take and return A0's own values.
- **Methods, interfaces and bounded generics**: methods on structs, enums and value types;
  interfaces whose values dispatch at run time through tables on the type descriptors; type
  parameters bounded by interfaces, resolved statically. Minnow has `impl`, `interface` and
  `T: Bound`.
- **Exceptions and results**, interpreted and native alike: `throw` and `try`/`catch` in the HIR
  (an uncaught exception reports its value and where it was thrown), and `propagate`, a language's
  `?`, for result values; Minnow has `?`. Programs that never throw pay nothing.
- **Lox exceptions and natives** (A0's extensions to the book's Lox): `throw` and
  `try`/`catch`, and `readLine`, `readFile`, `writeFile`, `env`, `number`, `sqrt`, `floor` over
  the standard library, whose failures are exceptions; the official suite still passes.
- `add_text` adds HIR text to a builder.
- **Labeled `break` and `continue`** in the core IR (`break 1` leaves two loops).
- Lowering compiles only the functions `main` can reach.
- **Modules**: programs of several files, with imports found through the grammar (`@import`),
  visibility (`set_public`, Minnow's `pub`), qualified names in the IR and stack traces, errors for
  missing modules, private items and import cycles, and the language server analyzing a program's
  files together (diagnostics per file, go-to-definition across files).
- **Runtime errors** report what failed with its values (`index out of bounds: the length is 3 but
  the index is 3`), then the stack of source-level functions, inlined ones included, identically in
  the interpreter and native code, optimized or not, at no cost to programs that do not fail.
- **Diagnostics rendered like rustc's**: related labels in one snippet, multi-line spans, fixes as
  corrected code, colors on terminals (`NO_COLOR`, `CLICOLOR_FORCE`).
- **Error codes** with explanations (`lang explain E2001`, `a0 explain`); a language can hide them
  or map them to its own (`Codes`).
- **Related locations** in the builder's errors: where an expected type comes from, the earlier
  declaration, the called function, the variants a `match` misses.
- **"Did you mean" suggestions** for unknown names, functions, types, fields and variants, as fixes
  the language server offers as quick fixes; a language reports its own errors with codes, labels,
  notes and fixes (`report_code` and the `report_*` functions, also in C).
- **Warnings** a language turns on: unused variables and unreachable code (`enable_warning`).
- `set_term` lets a language name the concepts messages use; `docs/diagnostics.md`.
- Snapshot tests of every example language's rendered output (`tests/diagnostics`,
  `a0_driver::snapshots`), and a random HIR program generator whose programs must agree in all six
  test modes.
- An ignored test that reports the Lox suite's compile time with and without the optimizer.

### Changed

- Inlining is bounded by the size of a callee after its own inlining and by how much a function may
  grow, so chains of helpers compile quickly.
- Runtime errors print `panic: ...` and a stack trace instead of `a0: panic: ...` and
  `a0: trap: ...`. Runtime functions that can fail take a location and a frame pointer
  (`REPORTS_LOCATION`).
- Every diagnostic has a code (`E1001`, ...) instead of a layer name (`hir.type`); a language's own
  errors from `report` have none.

### Fixed

- The optimizer never inlined a function whose body ends in `return x;` (every function of a
  language with explicit returns, Minnow's included), taking it for one that never returns.
- Faster code: lengths, `byte_at` and array elements are inline loads after a bounds check; float
  formatting finds the shortest digits with a fast path for integers and a bisection (6x faster on
  numbers); the collector sweeps memory chunk by chunk instead of following a list of every object
  (10x faster sweeping, half the collector's time on the JSON tool's large inputs).
- In the HIR text format, a `match` inside an `if let` whose type depended on the binding failed with
  "match needs an enum".

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

[Unreleased]: https://github.com/Dan-k391/a0/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/Dan-k391/a0/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/Dan-k391/a0/releases/tag/v0.1.0
