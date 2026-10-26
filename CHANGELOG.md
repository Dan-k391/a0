# Changelog

Notable changes to A0. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and versions follow [Semantic Versioning](https://semver.org/); before 1.0, a minor version may change
APIs. `docs/milestones.md` has the details of each step, with measurements.

## [Unreleased]

### Added

- **B0, the flagship language** (`b0/`, D87): static types with inference, structs and inline
  structs, sized numbers, enums, generics, interfaces, closures, maps, exceptions, dynamic objects,
  threads, manual memory with raw pointers in `unsafe`, C functions, modules and packages, and a
  prelude written in B0. Built only on the covered API, which a test checks; tested like the
  example languages in every mode, with `examples/wordfreq`, a program of two modules. The VS Code
  extension knows `.b0` files. New code `E2027` (needs `unsafe`).

- **The A0 book** (`docs/book/README.md`, `docs/SUMMARY.md` for mdBook): from getting started
  and the tutorial through the language's parts and its tools to a full language (B0), memory,
  concurrency, dynamic languages, other platforms and hosting, with a guide to each language. A
  test keeps every relative link in the documentation valid.
- **An API reference**: every public item of the covered crates is documented
  (`#![warn(missing_docs)]`), and CI builds the reference with warnings as errors.
- **Releases** (`docs/releases.md`): `release/package.py` makes an archive per platform (the
  tools, `rust-lld`, the target packs, the C API, the Python package with its library, the editor
  extension, the docs) and the packages to publish (crates, a Python wheel, the `.vsix`);
  `release/check.py` checks an archive alone; `.github/workflows/release.yml` drafts a GitHub
  release on a tag, publishing nothing. The crates have the metadata crates.io needs, the
  examples `publish = false`, and the repository a `LICENSE`.
- **Performance from release to release**: `bench/run.py --record` keeps headline numbers in
  `bench/history.csv`; `bench/history.py` prints them by machine.
- **A compatibility promise** for 1.0 (`docs/compatibility.md`, D86): what is covered (the command
  line, the text formats, the C header, the Python package, the Rust API for languages, the part of
  `a0rt.h` in the new `docs/runtime.md`), the rules for each, and how things are deprecated.
- **Versioned text formats**: `module "m" version 1;` (HIR), `module "m" version 1` (core IR) and
  `grammar g version 1;`, optional and 1 when absent; HIR and IR are printed with it. A later
  version is refused with the new code `E0004`. `a0_hir::FORMAT_VERSION`, `a0_ir::FORMAT_VERSION`,
  `a0_syntax::FORMAT_VERSION`.
- `a0_driver::backend` re-exports what a language with a C runtime needs (`Library`,
  `HostSymbols`, `A0Symbol`); `Library::new`.
- `A0_VERSION_MAJOR`, `A0_VERSION_MINOR`, `A0_VERSION_PATCH` and `A0_DEPRECATED` in `a0.h`;
  `a0.__version__` in Python, which checks the library's version when imported.
- `rust-version` (1.97, what Cranelift needs) in every crate.

### Changed

- `Language` is built with `Language::new(name, compile)` and methods (`grammar`, `modules`,
  `repl_main`, `codes`, `exit_codes`, `run_by_default`, `runtime`) instead of a struct literal;
  `Modules::new`, `ExitCodes::new`, `LowerOptions::optimized`, `Label::new` and `Edit::new` replace
  theirs. Public enums and structs that may grow are `#[non_exhaustive]`; the core IR's opcodes
  and types, and a few other enums A0 matches exhaustively, say in their documentation that they
  may grow.
- Kaleidoscope and Tin depend only on covered crates (and `a0-runtime` for its header).
- `bench/large/gen.py` makes a program whose run time is linear in its modules.

## [0.9.0] - 2026-10-26

Large programs and ecosystems: packages with dependencies, fast builds of large programs, a fast
language server on them, closures inlined, and every input fuzzed. Milestone M27 in
`docs/milestones.md`.

### Added

- **Packages** (D81, `docs/packages.md`): a manifest named after the language (`minnow.toml`)
  with the program's main file and its dependencies, from directories or git repositories (fetched
  with `git` into a shared cache, pinned in a lock file); `run`, `build`, `ir` and `hir` without a
  file take the package, `update` refreshes the lock file and `--locked` refuses to change it.
  The language server and the debugger resolve imports through packages. Builds are reproducible,
  byte for byte. New diagnostic codes `E5001`–`E5006`.
- **Fast builds of large programs** (D82): a 100,000-line program builds in about half the time
  (the lexer runs a DFA, the HIR optimizer is linear, the location table is interned), and large
  programs compile into several objects at once on the machine's processors. `A0_TIME=1` prints
  how long each stage takes; `A0_CODEGEN_UNITS=n` sets the number of objects.
- **Fuzzing** (D85): IR and HIR text, grammars with their programs, the C API and manifests are
  fuzzed in the test suite, from the corpus (`A0_FUZZ_ITERATIONS`, `A0_FUZZ_SEED` to run longer).
- **Inlining of known closures** (D84): a call of a closure through a local that always holds it,
  such as the parameter of an inlined `fold(xs, 0, fn(a, b) { a + b })`, is inlined; such code
  runs about 5.8 times faster (`docs/benchmarks.md`).
- `bench/large/` generates a large package and measures building it and the language server on it;
  the benchmark gains a `closures` program.

### Changed

- A module's imports resolve from its package's source directory, so a file analyzed on its own
  (in the editor) resolves them as the build does. MSVC and lld-link link with `/Brepro`.
- The language server keeps each file's syntax tree and index between analyses and reuses those of
  files that did not change.
- Drivers and `a0` compile on a thread with a 1 GB stack, and the parser stops at 10,000 levels of
  nesting with `E0003` instead of overflowing the stack (D83).
- The HIR builder makes types with the right number of type arguments even when given the wrong
  number (it still reports it), and the checker rejects a closure made outside the body it was
  declared in.

### Fixed

- Rendering a diagnostic whose span starts or ends inside a multi-byte character panicked.
- `a0_hir_set_struct_inline` and the IR builder's `func_addr` and `global_addr` did not check
  their handles; a type parameter used outside its item, or a closure whose body makes the closure,
  crashed the compiler instead of being reported. A span whose end is before its start (from the
  C API) overflowed in debug builds.

## [0.8.0] - 2026-10-26

Everywhere: executables for other platforms from any host, compiled programs in the browser,
native debug information on Windows and for variables, and CI on every platform A0 supports.
Milestone M26 in `docs/milestones.md`.

### Added

- **Cross-compilation** (D79, `docs/cross.md`): `build --target` (in `a0` and every language
  driver) links executables for x86-64 and aarch64 Linux and x86-64 Windows with lld against
  *target packs*, which `targets/build.py` builds from shippable sources (musl, mingw-w64's import
  libraries for the Universal C Runtime, compiler-rt's builtins, A0's runtime and the example
  languages' C runtimes). Linux executables are static. macOS has no pack (Apple's SDK may not be
  redistributed).
- **A WebAssembly backend** (D78): `crates/a0-wasm` compiles the core IR to WebAssembly modules
  that link against the runtime built for wasm32; the playground runs programs compiled by
  default.
- **Native debug information on Windows** (D80): CodeView in objects and a PDB with a natvis file
  beside the executable, for Visual Studio, WinDbg and cdb.
- **Variables in native debug information** (D80): with `-g`, gdb, lldb and Windows' debuggers show
  every source variable with its type: numbers, booleans, strings, structs, arrays, captured
  variables.
- **aarch64-windows** (D77), through a patched copy of `cranelift-object` in `vendor/`.
- CI on Intel macOS and Windows ARM64, and a job that builds the target packs and runs the corpus
  through them (aarch64 under QEMU, Windows on Windows).

### Changed

- `a0_native::link_executable` uses lld and a target pack for targets other than the host
  (`A0_LINK=pack` for the host too); `find_pack`, `pack_name` and `LinkOptions::natvis` are new.
- `-g` keeps every source variable in a stack slot as well, which each assignment stores to.
- The playground's engine exports the runtime's functions, memory and table to compiled modules.

## [0.7.0] - 2026-10-26

Concurrency: threads that run at the same time and share memory, channels between them, and a
collector, runtime, debugger and stack traces that understand them. Milestone M25 in
`docs/milestones.md`.

### Added

- **Threads and channels** (D75): `std.thread` with `spawn`, `join`, `parallelism`, `channel`,
  `send`, `receive` and `close`, over the runtime's `a0_thread_*` and `a0_chan_*`. Threads are the
  operating system's; they run in the interpreter, the JIT and native code alike. A panic on any
  thread ends the program; a single-threaded program that waits on a channel fails with
  "deadlock".
- **A collector for threads** (D76): a shadow stack per thread and collections that stop the world
  at safe points (allocations, polls at loop heads and function entries, waits in the runtime).
  Programs without threads take no locks and poll nowhere.
- **Stack traces and the debugger across threads**: a spawned thread's trace ends with where it was
  spawned; `lang dap` lists threads, stops them all when one stops, and shows every paused thread's
  stack and locals.
- A cast from `any` to a type parameter, for generic code that keeps values in `any`.
- Minnow's `parallel.mn` and `tests/parallel.rs`, which require real speedup in every mode, and a
  threads table in the benchmark.

### Changed

- The runtime's tables (interned names, shapes, buffers, random numbers) are locked, and its error
  text, exception in flight and thrown traces are per thread. Threaded programs keep their exception
  in per-thread slots and do not use per-site field caches.
- `a0_runtime::set_thread_starter`, `thread_started`, `thread_id`, `panic`; `Interpreter::debug_thread`
  and two new `DebugEvent`s; the interpreter shares one compiled program between threads.
- Native executables link with `-lpthread` on Linux.

## [0.6.0] - 2026-10-26

Systems languages: values in place, raw pointers, memory without a collector, and C structs across
the C boundary, proven by Tin, a C-like example language that calls a real C library. Milestone
M24 in `docs/milestones.md`.

### Added

- **Sized numbers** (D69): `i8` .. `u64` and `f32` in the HIR, with wrapping arithmetic, unsigned
  division, comparison and conversion, bitwise operators (`&`, `|`, `^`, `<<`, `>>`, `~`), and
  arrays that keep each element at its size.
- **Inline structs** (D70): `inline struct` values live where they are used (locals, fields,
  array elements, globals), laid out as C lays them out and copied when assigned or passed. The
  core IR gains `stack_slot`.
- **Raw pointers** (D71): `ptr<T>`, `addr`, `offset`, `data`, `size_of::<T>()` and conversions;
  unchecked where a language asks, while arrays and references stay checked.
- **Memory without a collector** (D72): a HIR module that says `memory manual;` roots nothing,
  never collects, and frees with `free`; garbage-collected programs are unchanged.
- **C structs by value** (D73): externs take and return structs as each platform's C ABI says
  (System V x86-64, Windows x64, AArch64 and Apple's), from one implementation in the native backend
  (`byval`/`sret` in the IR); the interpreter calls such functions (and those of `f32` or more than
  eight arguments) through thunks compiled in memory, so every mode shares it. HIR externs take and
  return inline structs and `f32`. `a0_builder_set_struct_param` and `a0_builder_stack_slot` in
  the C API and Python.
- **Tin** (`examples/tin`, D74): a C-like systems language: structs as values, `*T`, `&x`, `p->f`,
  `alloc`/`free`, checked arrays beside unchecked pointers, and C functions called directly. Its
  programs run in all eight modes; one parses and builds JSON with cJSON (vendored, MIT), one calls
  C functions of structs in every ABI shape. Language server, debugger and REPL included.
- `main` may return a sized integer (C's `int main`).
- The benchmark measures Tin against C.

### Changed

- `HostSymbols` can hold thunks (`add_thunk`, `thunk`, `keep_alive`); `a0_native::add_thunks`; the
  drivers and the C API add thunks before interpreting.
- `set_type_syntax` fills in the pointee of pointer types.

## [0.5.0] - 2026-10-25

Fast to run: programs start at once and run at native speed in every mode, and the gaps the
benchmark found are closed. Milestone M23 in `docs/milestones.md`.

### Added

- **A JIT** (D63): `lang run`, `a0 run`, the REPL and Python's driver compile the program into
  memory with Cranelift and run it at native speed, with no linking; the code, the traces and the
  errors are an executable's. `A0_INTERP=1` interprets; the interpreter stays the reference and the
  debugger's engine. `a0_native::run_main`, `a0_jit_run` in the C API, `Module.run_jit` in Python.
- **Dynamic objects** (D66): the HIR type `object`, with `has_field`, `field`, `set_field` and
  `field_names`, for dynamically typed languages. Names are interned, objects built alike share a
  shape, and every place that names a field with a literal keeps a cache, so a field is a
  comparison and a load away. Lox's instances and classes use them, and method calls no longer
  allocate a bound method.
- **Arrays at a size and compact booleans** (D65): `filled(n, v)`; `array<bool>` keeps a byte per
  element (C sees a `bool *`).
- **The language server and the debugger for languages in Python** (D67): the driver is bound in
  the C API (`a0_language_new`, `a0_language_main`) with a compile callback, and Python's
  `a0.driver.main` runs it, so Python languages have every command of Rust ones.
- **Source spans for the standard library** (D68): its modules are built-in sources (`<std>/fs.hir`)
  that every `SourceMap` knows; diagnostics point into it, traces show its lines, and
  go-to-definition and the debugger open a copy of it.
- **The benchmark** (`docs/benchmarks.md`, `bench/run.py`): Kaleidoscope against LLVM's tutorial
  and LLVM's standard -O2 pipeline, Minnow against C (clang -O0 and -O2) and Go, Lox against clox;
  compile and run times natively, in the JIT and interpreted, with the optimizer on and off.

### Changed

- The collector spends far less time on allocation-heavy programs (D64): object headers are 16
  bytes instead of 32 (an object's type is 8 bytes before it, which generated code reads), marking
  prefetches, and the heap grows to three times what survives a collection.
- The differential tests run eight ways: interpreted, in the JIT, in the JIT under GC stress, and
  native, each optimized and not.
- `HostSymbols` moved to `a0-runtime` (re-exported by `a0-interp`); `a0_ir::referenced_externs`;
  `Language::run_status`; the C API depends on `a0-driver`.
- The optimizer propagates string literals; lowering describes every struct and enum for the
  debugger, so `any` values holding them show their fields.

## [0.4.0] - 2026-10-25

Tooling for language users: every language gets a debugger, native debug information, a REPL, and
a language server that completes, finds references and renames, with no code of its own.
Milestone M21 in `docs/milestones.md`.

### Added

- **A debugger for every language** (`lang dap`, `docs/debugging.md`): the Debug Adapter Protocol
  over the interpreter, with breakpoints, pause, stepping, the stack, locals shown by their source
  types (structs, arrays, maps, enums, `any`), hovers, and a stop where a program fails. The VS
  Code extension adds a debugger of type `a0`; projects made with `a0 new` come with a launch
  configuration and a test of it.
- **Native debug information** (`lang build -g`, `a0 build|obj -g`): DWARF line tables and functions
  for gdb and lldb on Linux and macOS (`dsymutil` on macOS); not variables, and not Windows (PDB).
- **A REPL for every language** (`lang repl`, `docs/repl.md`), Python ones included: the inputs
  make a program that reruns in a process of its own, showing only new output; failing inputs are
  dropped, unfinished ones continue on the next line. `Language::repl_main` says where statements go
  in a language that runs them in `main`; `lang run -` reads the program from standard input.
- **The language server completes, finds references and renames** (`docs/editors.md`): names in
  scope, keywords, imported modules and their items, and after `.` the members of a value's type;
  references and renames across the files of a program through its imports. Edits are analyzed once
  they pause or a request needs them, so typing stays responsive in large programs.
- IR debug types (`Builder::debug_type`, `set_var_debug`), which lowering records for every local;
  `HirBuilder::set_closure_name` (`a0_hir_set_closure_name`); the runtime's debugger hooks
  (`a0_rt_write`, `a0_rt_set_output`, `a0_rt_set_fail_hook`, `a0_rt_set_exit_hook`);
  `SyntaxIndex::visible_at`; `HirModule::members`; `a0_driver::lsp::session_requests`.

### Changed

- `Language` has a field more, `repl_main`, and `serve_lsp` takes its input by value (it is read on
  a thread of its own).

### Fixed

- Kaleidoscope gave number literals the span of the expression built before them, and Lox gave its
  HIR no spans at all; both now place their code on its lines, which the debugger needs.

## [0.3.0] - 2026-10-25

Easy for everyone: a language can be started in minutes, in Rust, in Python or in a grammar alone,
and tried in a browser with nothing installed. Milestone M20 in `docs/milestones.md`.

### Added

- **`a0 new <name>`** creates a language project: grammar, compiler, driver, tests of its programs
  (interpreted and native) and of its language server, an example and VS Code settings; `cargo
  test` passes in it from the start.
- **Languages without host code**: grammar rules give their nodes HIR forms (`@hir(let)`, or a
  rule named `function`, `block`, `binary`, ...), and `a0 lang <grammar.a0g> run|build|hir|ir|lsp`
  runs such a language with both backends and a language server. Calc (`examples/calc`) is one.
- **The playground** (`docs/playground.md`, https://dan-k391.github.io/a0/): A0 itself, built for
  WebAssembly, in a page with a grammar, a program and its output, tree, HIR and IR. The runtime
  builds for wasm32-wasip1 (64-bit pointer slots where generated code reads pointers, generated
  call wrappers), and Lispy (`examples/lispy`) joins Calc as a grammar-only language.
- **Python bindings** (`bindings/python`, `docs/python.md`): the package `a0` over the C API with
  ctypes (grammars, trees, both builders, running and native executables, and a driver), and
  Pebble, a language written in Python. The C API is also a shared library, and gained
  `a0_diagnostics_render` and `a0_hir_text`.
- The VS Code extension expands `${workspaceFolder}` in commands and their `args` (which replace
  `lsp`), and colors keywords, strings, numbers and comments of every language.

### Changed

- The runtime's structures declare the pointers generated code reads with `A0_WORD_PTR`, 64-bit on
  every target (unchanged layout on 64-bit hosts); every function in the interpreter's symbol table
  needs its prototype in `a0rt.h`.
- The C API's library files are `a0_capi.lib` and `liba0_capi.a`, without a hash, now that the
  crate is also a shared library.

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

[Unreleased]: https://github.com/Dan-k391/a0/compare/v0.8.0...HEAD
[0.8.0]: https://github.com/Dan-k391/a0/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/Dan-k391/a0/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/Dan-k391/a0/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/Dan-k391/a0/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/Dan-k391/a0/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/Dan-k391/a0/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/Dan-k391/a0/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/Dan-k391/a0/releases/tag/v0.1.0
