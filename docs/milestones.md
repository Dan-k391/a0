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
  from the syntax tree to IR (about 650 lines), a C runtime (`putchard`, `printd`), and a
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

## M6. High-level IR, runtime GC, Lox on the HIR

**Done**

- `AGENTS.md` (commands, layer map and dependency rules, conventions, testing and docs-sync rules,
  commit rules) and five workflow skills in `.claude/skills/`: `add-ir-instruction`,
  `add-hir-feature`, `add-runtime-function`, `add-example-language`, `differential-tests`.
- `a0rt` gained the language runtime (D15, D16, D18): a precise mark-sweep collector generalized from
  Lox's (type descriptors with per-word layouts, a shadow stack, registered roots, `A0_GC_STRESS=1`),
  immutable strings, growable arrays, insertion-ordered hash maps, NaN-boxed `any` values and number
  formatting. Small objects come from size-class free lists; small arrays keep their elements inline.
- `a0-hir`, the high-level IR (`docs/hir.md`, D14, D17): a typed expression tree with strings, arrays,
  maps, structs, enums, closures and `any`; a handle-based builder that types eagerly; a checker; a
  text format parsed with A0's own grammar layer, with an exact round trip; and lowering to the core
  IR plus `a0rt` calls. Lowering keeps the collector precise without stack maps: reference and `any`
  locals live in shadow-stack slots, captured locals in heap boxes, and temporaries are spilled only
  when something after them can allocate. `a0 run|build|fmt|check|lower` accept `.hir` files.
- Tests: `tests/hir/*.hir` run interpreted, native and under GC stress next to the core IR programs;
  the HIR test suite checks round trips, precedence, 13 type errors, name and syntax errors and the
  lowering of closures. The program harnesses now kill a program after 60 seconds, so a collector bug
  that makes a program loop forever fails the test instead of hanging it.
- **Lox runs on the HIR.** Its C runtime (values, strings, tables, closures, classes and its own
  collector) is gone. Lox's semantics are written in HIR (`prelude.hir`); the compiler emits one
  closure per Lox function and leaves captures, rooting and collection to the HIR. All 241 tests of
  the `jlox` suite still pass interpreted, native and under GC stress.

  | Lines | Before | After |
  |---|---|---|
  | resolver and checks (`analyze.rs`) | 606 | 504 |
  | code generation (`codegen.rs` → `emit.rs`) | 935 | 504 |
  | runtime (`runtime/lox.c` → `prelude.hir`) | 783 | 212 (HIR) |
  | `lib.rs`, `main.rs`, `build.rs` | 242 | 243 |
  | grammar (`lox.a0g`) | 81 | 81 |
  | **total** | **2647** | **1544** |

- Found and fixed along the way: the runtime's symbol table did not compile with MSVC in release
  builds (`floor`, `ceil` and `log2` are intrinsics at `/O2`, and their addresses are not constant
  initializers).

**Changed**

- The collector moved into `a0rt` (D16 supersedes D13), as the fundamentals asked.
- Lox's stack-overflow limit is 3000 calls (it was 5000): each Lox call is now two IR calls, and the
  interpreter allows 10000 nested calls.
- Speed. The first port was 2.4 to 10.8 times slower than the hand-written C runtime on the book's
  benchmarks, natively. Two general runtime fixes (D18) and compiling Lox globals to HIR globals
  (D19) brought it to 1.0 to 2.1 times the old time on nine of the ten benchmarks
  (`instantiation`, `invocation` 1.0; `properties` 1.2; `zoo` 1.3; `equality`, `string_equality` 1.4;
  `trees` 1.5; `method_call` 1.6; `binary_trees` 2.1) and 5.4 on `fib`, which is nothing but calls
  and arithmetic. Measured natively on x86-64 Windows against the pre-port compiler.

**Open questions**

1. Lox is still slower than with its hand-written runtime: every operator is a call into the prelude,
   every call allocates an argument array, and string equality compares contents (the old runtime
   interned strings). Worth closing further? The options are an inlining hint in the HIR, interned
   strings in `a0rt`, or fixed-arity closure calls for small argument counts.
2. Memory management is per module, and `gc` is the only mode. Is a non-GC mode (arena or manual)
   needed before the typed example language, or only when a language asks for it?

## M7. A shared language driver

**Done**

- `a0-driver` (D20): a language describes itself with a `Language` value (name, compile function,
  exit codes, and for a C runtime its interpreter symbols and link libraries) and gets `run`,
  `build`, `ir` and `hir` commands, rustc-style diagnostics or its own message format, and its own
  exit codes for usage, I/O, compile and internal errors. A running program's exit status passes
  through unchanged.
- Kaleidoscope and Lox use it: their drivers went from 79 and 90 lines to 20 and 32. Lox keeps jlox's
  `lox file.lox` form and its exit codes (64, 66, 65).
- Unit tests for command-line parsing; the language suites cover the rest.

**Changed**

- The M4 open question (a reusable driver) is answered: two languages with different error formats
  and exit codes fit one driver once compile errors can be either diagnostics or preformatted lines.

**Open questions**

- None.

## M8. CI on Linux, macOS and Windows

**Done**

- `.github/workflows/ci.yml` builds the workspace and runs every test on x86-64 Linux, aarch64 Linux,
  aarch64 macOS and x86-64 Windows, including the native executables each system's linker produces
  and the Lox suite in all three modes. A separate job runs rustfmt and clippy with warnings as
  errors. Only GitHub's own actions are used; Rust comes from the runners' rustup.
- The first run passed everywhere. That settles the M2 question: linking on Linux and macOS, and
  aarch64 code generation, now run in CI rather than only being validated structurally.

**Changed**

- Found before CI could: the runtime did not compile with MSVC in release builds (`floor`, `ceil` and
  `log2` are intrinsics at `/O2`), and the test harnesses could hang forever on a collector bug; both
  are fixed (M6).

**Open questions**

- None. aarch64-windows stays out of scope.

## M9. Static types and a typed example language

**Done**

- `a0_sema::Types` (D21): interned named, array and function types; nominal structs with fields;
  type variables with unification and an occurs check; `resolve`, `same`, `is_complete`, `display`
  in source syntax, and `expect`, which reports every mismatch in one form
  ("mismatched types: expected `int`, found `string`").
- `examples/minnow` (D22): `int`, `float`, `bool`, `string`, arrays, structs, closures that capture by
  reference, `for` over arrays, and `print`/`str`/`len`/`push`/`pop`. The checker resolves names and
  types on `Scopes` and `Types`, infers the element type of `[]` from later uses, and keeps errors
  from cascading. The emitter produces statically typed HIR (no `any`), and the `minnow` command is
  an `a0-driver` language. 75 lines of grammar and about 1400 of Rust.
- Tests: six programs (recursion, higher-order functions, closures in loops, sorting, nested
  arrays, strings, a tree of structs for GC pressure, an out-of-bounds panic) run interpreted,
  native and under GC stress; 26 compile errors are checked with their line and column; the emitted
  HIR round-trips through the HIR text format.

**Changed**

- Nothing below the frontend needed to change for a statically typed language: the HIR already had
  every construct Minnow uses.

**Open questions**

1. Generics. The toolkit's unification could support let-polymorphism (generalizing at `let`) or
   explicit generic functions; the HIR has no generics, so they would be monomorphized by the
   frontend. Which, if either, is wanted?

## M10. Known issues from phase 2

**Done**

- `any` stores integers that fit in 48 bits inline (D15); only larger ones are boxed. The runtime
  tests cover the boundaries, and `tests/hir/dynamic.hir` checks inline and boxed integers in all
  three modes.
- The 48-bit address assumption of tagged references is documented in `a0rt.h` and D15 and checked:
  the collector panics if a heap address it hands out does not fit, instead of corrupting references.
- String equality compares cached hashes before contents (D16), so comparing the same long strings
  again is O(1).
- D12 and D13 are marked as superseded.

**Changed**

- Lox's `string_equality` benchmark did not get faster: its time goes into calls, not into comparing
  strings. The optimizer (M13) is where that is addressed.

**Open questions**

- None.

## M11. HIR types: inference, generics, nullable references

**Done**

- The HIR builder infers types (D24): inference variables, unification, and nodes typed again at
  `finish` when their rule needed a type that was still open. Errors point at the sub-expression they
  are about, in user-facing wording.
- References are non-null unless their type is `T?` (D23): `null` is a `T?`, a `T` fits a `T?`, `e!`
  unwraps (panicking on null), and generated code checks for null only there. Globals of reference type
  must be nullable. `T?` works in generic code; each instance is checked.
- Generic structs, enums and functions, type checked once with opaque type parameters and
  monomorphized by A0 before lowering (D25); type arguments are inferred or written out.
- The text format covers all of it (`struct Pair<A, B>`, `Cell<T>?`, `f::<int>(x)`, `x!`, `[]`).
  `tests/hir/generics.hir` runs in all three modes, the round trip holds for every corpus file, and the
  HIR tests check the builder API directly.

**Changed**

- The checker no longer re-derives types on its own; it types every finished node again as a safety
  net and keeps the structural rules.
- A latent bug in the HIR text parser (closure parameters bound under their uniquified names) showed up
  and was fixed.

**Open questions**

1. Flow typing (`if x != null` making `x` non-null) was left out (D23); frontends bind `x!` where
   needed. Wanted?

## M12. The frontend API: a typed language without a checker

**Done**

- Name resolution and construction by name in the builder (D26): bodies and scopes, `let_named`,
  `name`, `call_named`, `assign_named`, `named_type`, `new_struct_named`, `variant_named`, `report`,
  `error_expr`. An error type keeps one mistake from causing more errors; diagnostics can show types
  in the language's syntax; item names may be HIR keywords.
- Framework helpers every frontend needed: `a0_syntax::unquote` for string literals,
  `SyntaxTree::nodes` and `nodes_of_kind`.
- Minnow rewritten as one walk over its tree and extended with enums and `match`, generics and nullable
  references (brief item 4). Its 26 error cases grew to 37.

  | Lines | Before | After |
  |---|---|---|
  | Minnow (grammar + Rust) | 1513 | 692 (with enums, generics, nullable types) |
  | Lox (grammar + Rust + prelude) | 1486 | 1486 |

**Changed**

- Lox keeps its own resolver: jlox's error messages and rules (declared-but-not-defined, `this`
  outside a class, ...) are Lox's, not the framework's, so the name helpers do not apply. Its count
  may change with the speed work (M14).
- Some error messages changed wording to the framework's (for example "this function can end without
  returning a value of type `int`" is now reported at the function body rather than its name).

**Open questions**

- None.

## Overall open questions

These need a decision from you:

1. **C-bindable APIs** (M1, D2): is the interpretation in D2 the intended line?
2. **Flow typing for nullable references** (M11): add it, or keep `x!`?

Settled since earlier milestones: aarch64-windows is out of scope; the collector lives in `a0rt`
(D16); linking on Linux and macOS is tested in CI on every push (M8); a reusable language driver
exists (M7).
