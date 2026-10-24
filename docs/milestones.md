# Milestones

One entry per milestone: what was done, what changed along the way, and open questions.

## M1. Core IR and interpreter

**Done**

- `a0-diag`: source map, spans, diagnostics with rustc-style rendering. Shared by every layer.
- `a0-ir`: the structured IR described in `docs/ir.md`, written down before the code.
  - Builder with matched `if_`/`else_`/`end_if` and `loop_`/`end_loop` calls, mutable variables, and
    automatic unique naming. Misuse becomes a diagnostic, never a panic.
  - Text format with an exact round-trip guarantee, tested byte for byte on a file that uses every
    opcode, every float edge case (`-0.0`, `inf`, `nan(0x...)`, `1e300`) and quoted names.
  - Verifier for types, value scoping, call signatures, variables, `break`/`continue` placement and
    termination of non-void functions. Diagnostics point at the text when there is one.
- `a0-runtime`: the C runtime (lifecycle, traps, output, memory, time) as a static library, a symbol
  table for the interpreter, and `a0_ffi_call`, a generated trampoline for calling C from Rust.
- `a0-interp`: compiles functions to a slot-resolved op tree, runs them on a shared stack with real
  host memory, calls C through the trampoline, reports traps as values and bounds recursion depth.
- `a0`: `check`, `fmt` and `run` subcommands.
- Tests: unit tests per crate and `tests/programs`, a corpus of IR programs with expected stdout,
  stderr and exit status, run end to end through `a0 run`.

**Changed**

- Rust was not installed on the development machine; it was installed with `rustup` (stable,
  MSVC toolchain) after asking.
- The text format grew `nan(0x...)` to keep NaN payloads exact, and all-digit names are reserved for
  unnamed values so that names, not internal ids, are what round-trips.
- IR traps (division by zero, `unreachable`) are checked explicitly so that the interpreter and native
  code print the same message and exit with the same status (101).

**Open questions**

1. *C-bindability.* D2 in `docs/decisions.md` spells out how "no Rust-specific types in public
   signatures" was interpreted: `&self`, `&str`/`&[T]` inputs and borrowed `&str`/`&[Handle]` returns
   are allowed, lifetimes on types, generics, closures and data-carrying enums are not. Is that the
   intended line?
2. *Interpreter FFI limits.* Calls from the interpreter to C support up to 8 integer/pointer/`f64`
   arguments; `f32` arguments and results are rejected with a diagnostic. Native code has no such limit.
   Good enough until a language needs more?
3. *Small integer arguments to C.* IR integers are signless, so `i8`/`i16` arguments to C functions are
   not extended according to the C type's signedness. This only matters for C functions with `char` or
   `short` parameters (and on Apple AArch64). Frontends should pass `i32`/`i64`.

## M2. Native executables from IR

**Done**

- `a0-target`: explicit `Target` (architecture + OS) with triple, object format, calling convention,
  PIC requirement, linker flavor and file suffixes. Codegen never asks the host.
- `a0-native`: lowers IR to Cranelift. IR variables are handed to `cranelift-frontend`, which builds the
  SSA; `if`/`loop` become blocks; dead code after a terminator is not emitted. Division by zero, signed
  overflow and `unreachable` call `a0_rt_trap`, so native code prints exactly what the interpreter
  prints. With an entry point, the IR `@main` is emitted as `a0.main` behind a generated C `main` that
  initializes the runtime.
- Linking uses the platform linker (`link.exe` found through the `cc` crate's MSVC discovery, `cc`
  elsewhere) with the runtime's embedded static library. Only the host target can be linked.
- `a0 build` (executable) and `a0 obj --target T` (object for any target).
- Tests: every corpus program runs natively and must meet the same expectations as in the
  interpreter; every program is compiled for every emittable target and the objects are inspected
  (format, architecture, exported and imported symbols). A new `semantics.a0ir` exercises integer
  wrapping, shifts, narrow types, `i1` arithmetic and saturating conversions in both backends.

**Changed**

- The differential tests found one real bug: `i1` lives in an `i8` in Cranelift, so `iadd` on `i1`
  produced 2. Arithmetic results on `i1` are now masked to one bit.
- Cranelift 0.137 enables only the host ISA by default; both the `x86` and `arm64` backends are now
  enabled explicitly.

**Open questions**

1. **aarch64-windows cannot be produced.** `cranelift-object` 0.137 has no AArch64 relocations for
   COFF (it hits `unimplemented!`). The backend now reports a diagnostic instead of crashing. Fixing it
   means either contributing COFF/ARM64 relocation support to Cranelift upstream, or writing that part
   of object emission ourselves, which is close to the "don't write our own linker" line. I have not
   worked around it. Which way do you want to go, or should the target wait?
2. Linking was exercised on x86-64 Windows only, the machine available here. The Linux and macOS link
   paths are written but not run; their objects are checked structurally.
3. Executables get an 8 MiB main-thread stack where the linker can set it (Windows, macOS). On Linux
   the stack size comes from `ulimit`.

## M3. Frontend: grammar layer and diagnostics

**Done**

- `a0-syntax`: grammars are text files in a small DSL (`docs/grammar.md`), loaded at run time.
  - Lexer: our own longest-match NFA regex engine; literals used in rules become tokens; ties go to
    literals, then to patterns in file order. Error patterns (`error "Unterminated string." /.../`) and
    unknown characters are reported and skipped.
  - Parser: ordered choice with backtracking, fields (`cond:expr`), lookahead, `@inline` rules,
    `@prec` blocks (Pratt parsing with prefix, infix and postfix forms), per-expectation messages
    (`x ~ "Expect ';' after value."`) and `@recover` panic-mode recovery that yields `error` nodes.
  - Grammar validation: undefined names, duplicates, left recursion, repetition of empty matches,
    patterns that match empty text.
  - `resolve_precedence` for operator tables known only after parsing.
  - Trees are arenas with `NodeId` handles, field lookup and an s-expression dump.
- `a0-sema`: lexical scopes with duplicate detection, declared/defined state and capture detection.
- `a0 parse grammar.a0g file` prints the tree or rendered errors.
- Tests: regex engine, grammar validation, lexing, tree shapes, precedence and associativity, and a
  complete Lox grammar that reproduces jlox's error messages and recovery on 23 cases taken from the
  official test suite (including the multi-error `for` cases and the unexpected-character case).

**Changed**

- The first version reset the furthest-failure position at every recovery point. That lost the error
  inside a broken `else` branch (backtracking had already abandoned it), so the furthest failure is now
  kept across recovered constructs (D10).
- Methods and functions need different jlox messages ("Expect method name." vs "Expect function
  name."), and the DSL has no parameterized rules, so the Lox grammar spells out both forms.

**Open questions**

1. The DSL has no parameterized rules or macros; small duplication (as in Lox's `fun_decl`/`method`)
   is the price. Worth adding when a second language needs it?
2. No static type checker in `a0-sema` yet (D12), since neither example language has static types.

## M4. Kaleidoscope end to end

**Done**

- `examples/kaleidoscope`: the LLVM tutorial language through chapter 7. Grammar file, a compiler
  from the syntax tree to IR (about 600 lines), a C runtime (`putchard`, `printd`), and a
  `kaleidoscope run|build|ir` driver.
- User-defined operators work as in the tutorial: binary operators are parsed as flat chains and
  resolved with the operator table in effect at that point in the program (a `def binary|`
  installs its precedence before its own body). `for`, `var`/`in`, assignment and parameter
  mutation follow the tutorial's semantics, including its quirks (the loop body always runs once, and
  the end condition sees the old loop value).
- `a0-build`: a build-script helper that compiles a language's C runtime, links it into the driver
  for the interpreter and keeps a copy for native linking. Kaleidoscope is its first user and Lox will
  be its second.
- Tests: four programs (recursive and iterative fib, the tutorial's Mandelbrot with its operators,
  variables and scoping, custom operators with recursion) run interpreted and native with the same
  expected output; 16 diagnostic cases plus multi-error, ordering and IR round-trip checks.

**Changed**

- Nothing in the lower layers needed to change for Kaleidoscope. It used the IR builder, the grammar
  layer, `resolve_precedence` and `Scopes` as they were.
- Deliberate differences from the tutorial are listed in `examples/kaleidoscope/README.md`: output
  goes to stdout, top-level expressions run in order at startup, and redefinitions are errors.

**Open questions**

1. Should the framework offer a reusable "language driver" (run/build/ir subcommands, diagnostics
   rendering)? Kaleidoscope's driver is small, and Lox needs its own exit codes and error format, so I
   kept the drivers per language for now.

## M5. Lox end to end

**Done**

- `examples/lox`: all of Lox (closures, classes, inheritance, `super`, initializers, native `clock`).
  - The grammar reproduces jlox's syntax errors and panic-mode recovery.
  - `analyze.rs` implements jlox's parse-time checks (assignment targets, the 255 argument and
    parameter limits) and its resolver on `a0-sema` scopes, and computes which variables closures
    capture and each function's upvalue list.
  - `codegen.rs` emits IR in which every live Lox value sits in a shadow-stack frame slot; calls go
    through `lox_call_prepare` and `call_indirect`; captured variables live in heap cells.
  - `runtime/lox.c`: NaN-boxed values, interned strings, hash tables, closures, classes, bound methods
    and a precise mark-sweep collector, with `LOX_GC_STRESS=1` to collect on every allocation.
  - `lox file.lox` behaves like jlox (exit 65 for compile errors, 70 for runtime errors); `lox build`
    makes a native executable.
- The official test suite is vendored (MIT, commit `4a840f7`) and `tests/suite.rs` checks it with the
  same rules as the book's `test.dart`. **All 241 tests of the `jlox` suite configuration pass**
  interpreted, as native executables, and interpreted under GC stress. Two tests that the `jlox`
  suite skips because of JVM behavior (`number/nan_equality.lox`, `limit/stack_overflow.lox`) also pass.
  The remaining skips are the same as jlox's: early-chapter tests, benchmarks, and clox's bytecode
  limits.
- Checks on the checks: a deliberately wrong expectation makes the suite fail; disabling the shadow
  stack roots makes 11 tests fail under GC stress; two million allocations peak at about 6 MB.
- `Builder::patch_iconst`, so a prologue can use a frame size known only after the body is built.
- Workspace-wide clippy (no warnings) and rustfmt.

**Changed**

- No lower layer needed a redesign for Lox. The additions were `patch_iconst` and the grammar-layer
  error behavior that M3 had already shaped around jlox's messages.
- The collector, strings and tables went into Lox's own runtime rather than `a0rt` (D13): only one
  language needs them so far.

**Open questions**

1. Should the collector move into `a0rt` now, as the fundamentals list memory management as a runtime
   responsibility, or wait for a second language that needs it (D13)?
2. Lox's dynamic operations are all runtime calls. Inline fast paths for numbers in the IR would make
   native Lox much faster; the tests did not need it, so it is not done.
3. Numbers print in the shortest round-trip form, which equals jlox's output for everything the suite
   prints, but extreme magnitudes differ (`1e+21` here, `1.0E21` in jlox).

## Overall open questions

These need a decision from you:

1. **aarch64-windows** (M2): Cranelift's object writer cannot produce AArch64 COFF objects. Contribute
   the relocations upstream, write that part ourselves, or leave the target out for now?
2. **Linking on Linux and macOS** (M2) is implemented but untested, because only an x86-64 Windows
   machine was available. Objects for those targets are validated structurally.
3. **C-bindable APIs** (M1, D2): is the interpretation in D2 the intended line?
4. **Garbage collector placement** (M5, D13): in `a0rt` now, or when a second language needs one?
