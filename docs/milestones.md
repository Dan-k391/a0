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

## M13. An optimizer of our own

**Done**

- `opt.rs` (D27, about 875 lines) runs on every HIR module after monomorphization, on by default
  with no frontend work; `A0_OPT=0` or `LowerOptions { optimize: false }` turns it off. It inlines small functions three levels deep (turning mid-function `return`s into
  `if` values), decides `is`/`as` on `any` when the boxed type is known (also through locals, by
  splitting `let v = x as any` into the raw value and its box), drops the check from casts guarded
  by an `is` test (D28), and propagates constants and copies, removes dead code and flattens blocks.
- It never removes or duplicates an expression with an effect. Every harness (core IR and HIR
  programs, Minnow, the Lox suite) runs each program optimized and unoptimized, interpreted, native
  and under GC stress: six modes that must agree.
- What it is worth today, natively, as unoptimized time / optimized time: `fib` 3.1, `equality`
  2.9, `zoo` 1.2, `instantiation` 1.3, `binary_trees` 1.2, `invocation`, `method_call`, `properties`
  and `trees` 1.1 to 1.2, `string_equality` 1.0 (it reads globals, which the optimizer leaves alone).

**Changed**

- Functions that never return (runtime-error helpers) are not inlined: copying their bodies into
  every operator only grew the code.

**Open questions**

- Languages that emit the core IR directly (Kaleidoscope) do not get the optimizer. An inliner on
  the core IR would cover them; none needs it yet.

## M14. Native Lox at least as fast as with its hand-written runtime

**Done**

- Framework (no language involvement): fast `any` paths in lowering and the runtime (D28): inline
  type tests and equality, guarded casts, small array and map buffers from the free lists, a map's
  last-lookup cache, string equality by cached hash first. Reference locals are IR variables written
  through to their frame slots, blocks reuse slots, and frames are cleared by scope instead of when
  pushed (D30), which removed a `memset` from every call.
- Lox (D29): functions with up to three parameters take them directly instead of in an argument
  array, and call sites dispatch through `call0` .. `call3` in the prelude, which the optimizer
  inlines. The `+` operator tests for numbers first and leaves strings and errors to a helper.
- Every benchmark of the book is now faster natively than with the hand-written C runtime that Lox
  had before M6 (the M6 baseline), measured on x86-64 Windows, best of three:

  | Benchmark | M6 baseline | M6 (HIR) | now | now / baseline |
  |---|---|---|---|---|
  | `binary_trees` | 2.03 s | ×2.1 | 1.87 s | 0.92 |
  | `equality` | 0.48 s | ×1.4 | 0.15 s | 0.30 |
  | `fib` | 0.31 s | ×5.4 | 0.27 s | 0.87 |
  | `instantiation` | 0.75 s | ×1.0 | 0.48 s | 0.63 |
  | `invocation` | 0.74 s | ×1.0 | 0.40 s | 0.54 |
  | `method_call` | 0.31 s | ×1.6 | 0.23 s | 0.73 |
  | `properties` | 0.80 s | ×1.2 | 0.51 s | 0.63 |
  | `string_equality` | 0.44 s | ×1.4 | 0.42 s | 0.97 |
  | `trees` | 7.76 s | ×1.5 | 4.15 s | 0.53 |
  | `zoo` | 0.58 s | ×1.3 | 0.37 s | 0.63 |
  | `zoo_batch` (batches in 10 s, more is better) | 4769 | | 6265 | 1.31× the work |

- Tests: `tests/hir/fast_paths.hir` (map cache invalidation, guarded and unguarded casts, inline
  equality with NaN and -0.0, buffers growing out of the free lists, `break`, `continue` and `return`
  out of scopes holding references), a lowering test that guarded casts are unchecked and assigned
  ones are not, and the stress-mode check that every popped frame was cleared. Breaking a local's
  rooting still fails the Lox suite under GC stress (44 tests), as does skipping the clearing on
  `break`.

  | Lines | M12 | M14 |
  |---|---|---|
  | Lox (grammar + Rust + prelude) | 1486 | 1658 |
  | Minnow (grammar + Rust) | 692 | 692 |

**Changed**

- Lox grew by 172 lines, almost all in the prelude: `call0` .. `call3` (D29) and the split `+`. This
  is the one place where the language author tuned for speed. It is the ordinary way to compile a
  language with run-time arity checks to a typed IR, but a second dynamic language would show
  whether the framework should offer it (for example closures with optional fixed-arity entries).
- Generated code now depends on two runtime layout offsets (an object's type before it, a type's
  family in its descriptor); `a0rt.h` documents them and `gc.c` checks them at compile time.

**Open questions**

- None. Remaining per-call costs, if more speed is wanted: the two runtime calls that push and pop
  a frame (they could be inline loads and stores), the indirect call through the closure, and Lox's
  own depth counter for its "Stack overflow." error.

## M15. A language server for every language

**Done**

- Grammar annotations for names (D31, `docs/grammar.md`): `@def(kind)`, `@def(kind: field)` and
  `@scope`. `SyntaxIndex` in `a0-syntax` turns a parsed file into classified tokens, declared
  symbols with their nesting, and resolved references; it works on files with syntax errors.
- `lang lsp` in `a0-driver` (D32, `docs/editors.md`): diagnostics from the language's compiler,
  semantic highlighting, the outline, go-to-definition and hovers from the index, and types on hover
  from the HIR when the compiler records spans (Minnow shows `variable total: int`). It speaks UTF-16
  positions, survives a panicking compiler, and needs about 400 lines plus a 300-line JSON module.
- Languages give the driver their grammar (`grammar: Some(GRAMMAR)`) and nothing else. Lox, Minnow
  and Kaleidoscope needed 7, 13 and 6 annotations; Lox's own-format errors now carry spans
  (`a0_driver::Message`), so editors can place them.
- `editors/vscode`: a generic VS Code client that starts `<command> lsp` for configured file types.
  Checked by hand in VS Code 1.143 on Windows with Lox: highlighting, the error squiggle, the outline
  and hovers all appear.
- Tests: the index (scoping, shadowing, hoisting, members, syntax errors, token classes, bad
  annotations), a protocol session in the driver (every request, UTF-16 columns, split multi-line
  tokens, malformed input, a compiler panic), and `tests/lsp.rs` in each example language, which
  runs its real server.

  | Lines | M14 | M15 |
  |---|---|---|
  | Lox (grammar + Rust + prelude) | 1658 | 1675 |
  | Minnow (grammar + Rust) | 692 | 705 |

  The growth is layout: annotated rules put `=` on a line of its own.

**Changed**

- `CompileError::Messages` holds `Message`s (text and an optional span) instead of strings.
- VS Code's themes color names from semantic tokens but keywords, strings, numbers and comments only
  through a TextMate grammar; the extension maps those token types to TextMate scopes for the
  languages it declares, so no language needs a TextMate grammar.

**Open questions**

- None. Resolution is lexical (a member name refers to every member of that name); renaming,
  completion and find-references are not implemented and can be built on `SyntaxIndex`.

## M16. The C API

**Done**

- `a0-capi` (D33, `docs/c-api.md`, `include/a0.h`): 65 functions over the core IR builder,
  modules and their text format, the interpreter, the native backend (objects for any supported
  target, executables for the host), grammars and syntax trees, and HIR text.
- `tests/host.c`, a C program that uses only the header: it builds a recursive function and a loop
  with the builder, prints the module and parses it back, runs it, writes an aarch64-linux object
  and a host executable (which the test then runs), shows that builder misuse comes back as
  diagnostics, compiles a calculator language (an A0 grammar plus 40 lines of C that walk the tree
  and emit IR), and runs a HIR program. `tests/c_host.rs` compiles it with the system C compiler,
  links it with the static library, and checks its output, in CI on every platform.
- D2 revised: the line holds; four clarifications (text returned from cached `String`s, opcodes by
  name, consuming methods, `Clone` where C reuses an object).

**Changed**

- `a0_ir::Module` is `Clone`, `SyntaxTree` has `node_count` and `field_name`, for the binding.

**Open questions**

- None. The HIR builder and `SyntaxIndex` are not bound yet; D2 says how they would be.

## M17. v0.1: the easy path, end to end

**Done**

- AGENTS.md checked against the decisions: it no longer forbids an optimizer of our own (D27), counts
  six test modes, states D2's revision and the release workflow; D3's no-optimizer line is marked as
  changed; the skills follow.
- `if let x = e { ... } else { ... }` in the HIR (D34): a `T?` is narrowed by binding a new local of
  type `T` for one branch, with one null test and no flow typing. It has a text form, a by-name helper
  (`declare_if_let`) and a C binding. Minnow gained `if let` and `while let` (the latter is built
  from `loop`, `if let` and `break` in Minnow alone) and its programs lost three `!`; Lox's call fast
  paths use it instead of testing and unwrapping a field.
- The C API covers the HIR builder: every function, by handle and by name. The C layer checks each
  handle against the module and reports bad ones, so misuse stays a diagnostic; an `Item` is a kind
  and a handle, operators and builtins are names, name lists are `A0Str` arrays. The C test program
  compiles a small language with variables to HIR by name (its unknown names reported by the
  builder) and builds a struct, `if let` and a closure by handle. A test keeps `a0.h` and the
  library's exports identical.
- Kaleidoscope is built on the HIR like Minnow (D35), with the builder's name resolution and the
  optimizer, and runs in six test modes like the other languages. Lowering skips the frame of a
  body without references, so its numeric code makes no runtime calls: `fib` runs as fast as from
  the core IR, a loop that calls a user-defined operator 40% faster.

  | Lines (grammar + Rust) | Before | After |
  |---|---|---|
  | Kaleidoscope | 745 | 579 (not counting the tutorial's marker lines) |
  | Minnow | 705 | 762 (with `if let` and `while let`) |
  | Lox (with the prelude) | 1675 | 1675 |

- `docs/tutorial.md` builds Kaleidoscope on the HIR in LLVM's chapter order (D36). Every listing is
  an `ANCHOR` region of the example's code; a test checks that each listing is that code, that the
  listings together show every line of the compiler, grammar, runtime and command, and that every
  command shown prints what is shown (`A0_BLESS=1` rewrites them after a change). Chapters add
  regions instead of changing earlier code; the program as it stands after chapters 3, 4 and 5 was
  built and run by hand.
- A newcomer-facing README, a CHANGELOG and the v0.1.0 tag. `build -o name` now adds `.exe` on
  Windows.

**Changed**

- The HIR's "may be null" errors suggest `if let` instead of a null test, which cannot narrow.
- Kaleidoscope's unknown names and argument counts are now reported in the builder's words
  (``unknown name `y` ``, `expected 2 arguments, found 1`); its own rules (operators, precedence,
  `main`, redefinition) keep their messages. A function declared with `extern` can no longer be
  defined later in the same program, since it is a C function.

**Open questions**

- None.

## M18. Diagnostics as good as rustc's, at compile time and at run time

**Done**

- Rendering (D39): every label in a file shares one snippet, primary `^^^` and related `---`, with
  labels that do not fit hanging below on connectors; spans over several lines get a margin track;
  gaps are elided, other files show under `:::`, long lines are cut around the span; a short fix of
  the primary span becomes its label, others show as the corrected line. ANSI colors on terminals
  (`NO_COLOR`, `CLICOLOR_FORCE`), enabled on Windows consoles without a dependency.
- Codes (D40): 33 errors and 2 warnings in `a0_diag::codes`, numbered by kind and explained by
  `lang explain` and `a0 explain`; a test keeps `docs/diagnostics.md`'s table in step. A language
  shows A0's codes, none, or its own (`Codes::Own`).
- Messages (D41): a short message, a primary label, and related locations from what the builder
  already knows: the declaration an expected type comes from (variable, parameter, field, result),
  the first of two declarations, a called function's declaration, the variants a `match` misses.
  Concepts come from `Term`s (`set_term`), the unit type shows as `()`, an untyped `null` as
  `null`; nothing says `unit` or `_?`.
- Suggestions (D42): unknown names, functions, types, fields and variants get the closest declared
  one; it is a fix where the builder knows the name's span (`set_name_span`, or the current span
  when it is exactly the name). The language server publishes codes and related information and
  offers fixes as quick fixes (`textDocument/codeAction`). A language reports its own errors with
  codes, labels, notes and fixes (`report_code`, `report_label`, ...), also from C.
- Warnings (D43): unused variables (with a fix that adds `_`) and unreachable code, turned on per
  language (Minnow: both), found for programs without errors, kept in the module, printed by the
  driver and shown by the language server.
- Snapshots (D44): 18 programs in the three languages' `tests/diagnostics` whose full output must
  match a checked-in file, natively too; reviewing them improved several messages.
- Runtime errors (D45): the message with its values, then the stack of source functions, inlined
  ones included, identical in the interpreter and native code, optimized or not:

  ```text
  panic: index out of bounds: the length is 3 but the index is 3
      at at (runtime_index.mn:4:10)
      at last (runtime_index.mn:8:10)
      at main (runtime_index.mn:14:9)
  ```

  The IR carries spans, inline sites and trace names; both backends give the runtime the same
  location table; failing runtime functions take the location and frame pointer; native code walks
  the frame pointer chain through generated frames using a table from return addresses to calls.
  Core IR programs get traces too (`at main (trap.a0ir:6:3)`). Native Lox benchmarks run as fast as
  before (geometric mean ×1.00, interleaved, best of five; the first version was ×1.036 until
  message formatting left the fast paths), interpreted ones within noise.
- The optimizer's inlining is bounded (D37): callees first, a callee measured after its own
  inlining (at most 120 nodes), a body growing to at most 4000 nodes or twice its size. A chain of
  40 helpers that each call the previous one twice (`tests/hir/inline_chain.hir`) compiles in 0.2 s;
  before, it did not finish. Compile time of the Lox suite (189 programs that compile, release
  build, best of three, no linking; `cargo test --release -p lox --test suite compile_time --
  --ignored --nocapture`):

  | | front end | lowering | Cranelift | total | IR lines |
  |---|---|---|---|---|---|
  | `A0_OPT=0` | 0.72 s | 0.21 s | 1.41 s | 2.34 s | 1 045 158 |
  | optimized | 0.72 s | 0.60 s | 1.86 s | 3.18 s | 1 465 539 |

  The optimizer adds 36% to compile time, a third of it its own work and the rest Cranelift
  compiling the 40% more IR that inlining makes. The old inliner took the same time on the suite
  (3.09 s).
- Random programs (D38): a seeded generator of well-typed, terminating HIR programs (inlined
  helpers, `any`, closures over assigned locals, loops, recursion, runtime errors), each run in six
  modes that must agree, stack traces included; 24 per test run, 300 more by hand. Its first run
  found that the text format typed an `if let` binding only after its body, so a `match` there
  could not find its enum.

  | Lines (grammar + Rust) | M17 | M18 |
  |---|---|---|
  | Kaleidoscope | 579 | 580 (`codes` in its driver) |
  | Minnow | 762 | 794 (declarations at their names, name spans, codes and labels on its own errors, warnings) |
  | Lox (with the prelude) | 1675 | 1675 |

**Changed**

- Runtime errors print `panic: <message>` and a trace instead of `a0: panic:` and `a0: trap:`.
- `HirBuilder::report` gives an error without a code; `report_code` gives one. Declarations are
  attributed to their names in Minnow and the text format, so labels point at names.
- Programs with errors get no warnings, as with rustc.

**Open questions**

- Kaleidoscope's own errors (redefinition, operators) have no codes or labels yet; adding them would
  change the tutorial's listings.
- A frontend that knows the span of every name could pass it to every by-name call; the span
  heuristic (D42) has been enough for the three languages.

## M19. v0.2: real programs

**Done**

- Modules (D46): a program is its main file and what it imports, found through the grammar's
  `@import` rules; the driver loads and orders the files (no cycles), and each is a module of one
  HIR builder with its own names (`text.util.twice` in the IR and in traces), `set_public` for
  visibility, and `alias.item` through `member`, `call_member` and `member_type`. Missing modules,
  private items and cycles are diagnostics with the files involved. The language server analyzes
  the program around an open document, publishes diagnostics per file, and follows definitions
  and hovers into other modules. Minnow has `import` and `pub`.
- The standard library (D48, `docs/std.md`): `std.io`, `std.fs`, `std.env`, `std.time`,
  `std.math` and `std.str` (411 lines of HIR over 656 of C), added to a program when imported,
  with `std.Result` for what can fail. C functions take strings and arrays as pointers into A0's
  objects and return copied strings, and the interpreter finds C library functions by itself
  (D47): `extern fn getenv(string) -> string?;` is all a program needs. `extern "a0"` declares
  runtime functions that take A0's own values.
- Errors in both styles, both backends: `throw` and `try`/`catch` (D49), a pending flag and checks
  after calls that may raise, which a program that never throws does not get, with the IR's new
  `break N` to leave a `try` from inside loops; and `propagate` (`?`) for results, a `match` and a
  `return` (E2020 when misused).
- Abstractions (D52): methods on structs, enums and value types; interfaces, whose values are the
  objects themselves, dispatched through a table on each type's descriptor; type parameters bounded
  by interfaces, resolved to direct calls by monomorphization (value types satisfy bounds too).
  Implementations and bounds are checked (E1013, E2021, E2022). Minnow has `impl`, `interface`
  and `T: Bound`; the text format, the C API and the random program generator have them too.
- Lox uses exceptions (D51): `throw` and `try`/`catch` as A0's extension, and natives over the
  standard library (`readFile`, `readLine`, `writeFile`, `env`, `number`, `sqrt`, `floor`) whose
  failures are exceptions; the book's 241 tests still pass in all six modes, with 4 tests of the
  extensions.
- Acceptance: `json` (`examples/minnow/json`), a JSON tool in five Minnow modules (479 lines):
  reading with line and column in errors, pretty and compact writing, path queries, statistics.
  Writing it found, and A0 now fixes: the optimizer never inlined a function ending in `return`
  (all of Minnow's); `named_type` did not know `map`; lengths, `byte_at` and array elements were
  runtime calls (inline now); float formatting took up to 17 tries per number (6x faster); the
  collector's sweep followed a list of every object (D53: 10x faster). Functions that return early
  are inlined only up to 32 nodes, which keeps small accessors inlined without growing Lox's code
  with its larger checks (native Lox benchmarks: geometric mean ×1.01 against M18, within noise).
  On a 5 MB file the native tool now reads it in 0.30 s and writes it in 0.29 s (0.60 s and 0.64 s
  before), the collector marking the growing tree taking most of what remains.
- Lowering compiles only what `main` reaches (D50), so libraries and preludes cost only what is
  used. `A0_GC_STATS=1` reports the collector's work.

  | Lines (grammar + Rust, HIR) | M18 | M19 |
  |---|---|---|
  | Minnow | 794 | 1020 (modules, `?`, methods and interfaces, maps) |
  | Lox (with the prelude and natives) | 1676 | 1881 (exceptions, natives) |

**Changed**

- `extern fn` takes strings and arrays; a C function's `bool` is passed as a 64-bit integer.
- HIR text: `extern "a0"`, `throw`, `try`/`catch`, `interface`, `impl`, bounds `<T: I>`, and
  `(o.f)(x)` for calling a closure in a field (`o.f(x)` calls a method). New keywords: `throw`,
  `try`, `catch`, `interface`, `impl`, `for`.
- `lang ir` shows only the functions `main` reaches.

**Open questions**

- The collector: marking the whole live heap at every collection dominates allocation-heavy
  programs; a generational collector would need the shadow stack to know every reference precisely.
- The standard library's code has no spans (it is not in the driver's source map), so diagnostics
  and go-to-definition cannot point into it.
- No default methods, generic interfaces, or `finally`.

## M20. v0.3: easy for everyone

**Done**

- One command creates a working language project (D54): `a0 new mylang` writes a grammar, a
  compiler to HIR by name, the driver (`run`, `build`, `hir`, `ir`, `lsp`, `explain`), tests of its
  programs interpreted and native and of its language server, an example, VS Code settings and a
  README (599 lines in all). `cargo test` passes in the new project; a test builds one and runs
  its tests on every CI platform, so the template cannot rot.
- A language with no host code (D55): grammar rules give their nodes HIR forms (`@hir(let)`, or a
  rule named `function`, `block`, `binary`, ...), `a0_hir::dsl` builds HIR from the tree by those
  forms, and `a0 lang <grammar> run|build|hir|ir|lsp` runs the language with both backends and a
  language server. Calc (`examples/calc`, a 54-line grammar) and Lispy (`examples/lispy`, 41
  lines, the same forms in parentheses) are tested interpreted, native, under GC stress and
  unoptimized; the VS Code extension starts their server with `args`.
- Languages in Python (D56, `docs/python.md`): the package `a0` (`bindings/python`, 755 lines)
  binds the C API with ctypes, from a table of the builders' C signatures that a test checks
  against `a0.h`, and gives a Python language the driver's commands. Pebble, a typed language with
  arrays and `for`, has a 233-line compiler in Python; its programs run interpreted and native from
  `cargo test` on every platform. The C API is also a shared library and gained
  `a0_diagnostics_render` and `a0_hir_text`.
- A browser playground (D57, `docs/playground.md`), published from `main` to
  https://dan-k391.github.io/a0/: a grammar, a program, and their output, syntax tree, HIR and IR,
  with errors as `a0` renders them; a program that never ends can be stopped, and a link carries
  the grammar and program. The engine (`crates/a0-playground`) is A0 built for wasm32-wasip1
  (1.4 MB, 450 KB compressed): the runtime keeps 64-bit slots for the pointers generated code
  reads and calls C through generated wrappers in WebAssembly, and the page brings its own WASI
  calls (114 lines). CI builds it with wasi-sdk and tests it with Node.

**Changed**

- Runtime: `A0Type`, `A0Array` and the collector's header declare their pointers with
  `A0_WORD_PTR` (the layout is unchanged on 64-bit targets); `A0Type.itable` is a `void *`.
  Every function in the symbol table needs its prototype in `a0rt.h`.
- The interpreter runs on the calling thread in WebAssembly, where calls nest at most 1000 deep.
- `a0_driver::lsp::session_with_args` starts a server with other arguments than `lsp`.
- The C API's library files lost their hash (`a0_capi.lib`, `liba0_capi.a`), since the crate is
  also a cdylib.

**Open questions**

- A Python language has no language server: the driver is not bound to C (D20); a compile
  callback would be enough.
- The playground's engine is built on Linux or macOS only (wasi-sdk for Windows is 600 MB); its
  WebAssembly is not optimized further (no wasm-opt).
- The `@hir` forms have numbers as floats only and no types, modules or collections of a
  language's own; past them a language is a compiler, which `a0 new` starts.

## M21. v0.4: tooling for language users

**Done**

- A debugger for every language with no code of its own (D58, `docs/debugging.md`): `lang dap`
  serves the Debug Adapter Protocol over the interpreter, which marks each new source line and
  calls the debugger there and when the program fails, before unwinding. Breakpoints, pause, steps
  over, in and out, the stack, locals shown by their source types (structs, arrays, maps, enum
  variants, nullables, `any`, captured variables, interface values by their dynamic type), hovers,
  and a stop at a panic with its message. Lowering records IR debug types for every local; the
  runtime gives the debugger the program's output, failures and end. The VS Code extension has a
  debugger of type `a0`; Minnow, Lox, Kaleidoscope, Calc and new projects test a session. Writing
  it found that Lox gave its HIR no spans and Kaleidoscope's literals took stale ones (both fixed),
  and Lox's functions now carry their names (`set_closure_name`).
- Native debug information (D59): `lang build -g` (and `a0 build|obj -g`) writes DWARF line tables
  and functions with gimli. gdb stops at `file:line` and prints backtraces in source terms, tested
  on x86-64 and aarch64 Linux in CI, and lldb does the same on aarch64 macOS from the `.dSYM` that
  `dsymutil` gathers. That took a fix: dsymutil and lldb read the addresses in Mach-O debug
  sections as written, so A0 writes each function's address in the object there (as LLVM does),
  not 0 with a relocation to the function; a test checks it on every host. The object files' DWARF
  is read back for every DWARF target.
  Not described: variables (they would need Cranelift's value locations) and Windows (PDB).
- The language server completes, finds references and renames (D60): names in scope, keywords,
  imported modules and their items, and the members of a value's type after `.`; references and
  renames across a program's files through its imports, from the grammar's indexes, so they work
  with type errors. It stays responsive: edits only reindex, and the program is analyzed when they
  pause or a request needs it. On a generated program of 30 files and 9,664 lines (release build):
  the first analysis and a hover take 178 ms, twenty edits in a row and a completion 79 ms (no
  analysis), the next hover 164 ms (one analysis), find-references 1 ms.
- A REPL for every language (D61, `docs/repl.md`), Python ones included: the inputs make a program
  that reruns in a process of its own (`lang run -`), showing new output only; failing inputs are
  dropped, unfinished ones continue. A language whose statements live in `main` says so in one line
  (`repl_main`).

**Changed**

- `Language` gained `repl_main`; `serve_lsp` takes its input by value. `CodegenOptions` and
  `BuildOptions` gained `debug_info`, `LinkOptions` likewise.
- Runtime output goes through `a0_rt_write`; a language's own C runtime should use it too.
- `a0_runtime::FailFn` is unsafe; the interpreter has `new_debug` and `debug_*` inspection.

**Open questions**

- Native variables for gdb and lldb need Cranelift's value labels and DWARF location lists.
- The REPL reruns the whole session for each input; an incremental one needs the runtime to run
  several programs in one process.
- Debugging globals and evaluating expressions beyond names.
- A Python language has no language server or debugger yet (the driver is not bound to C).

## M22. A lightweight benchmark

**Done**

- `docs/benchmarks.md` and `bench/` (D62). Three comparisons, each A0 with its optimizer on and off:
  - Kaleidoscope on A0 against LLVM's tutorial: lines of code, compile time and run time.
  - Minnow against C (clang -O0 and -O2) and Go, on four programs.
  - Lox on A0 against clox, on six of the book's benchmarks.

  `bench/run.py` reproduces it in about ten minutes on Linux or macOS. It reports compile and run
  times as medians, checks that every implementation prints the same, records every tool's version
  and flags, and reports what A0's collector did in the allocation-heavy programs.
- The results, on one machine:
  - A0 builds executables in 110–160 ms, of which its own work is 2–6 ms; the rest is the system
    linker.
  - Minnow runs at clang -O0's speed on calls and floating point, half of -O2's. It is 3 times
    slower than C on arrays, half as fast as malloc on allocation, and faster than Go there.
  - Kaleidoscope's native code is as fast as LLVM's JIT on `fib` and 1.6 times faster on `mandel`,
    thanks to the HIR optimizer's inlining, which the tutorial's passes do not do.
  - Native Lox is 2.9 times faster than clox on `fib`, even on `binary_trees`, and 1.3–1.7 times
    slower on method calls and property access. A0's interpreter is 14–48 times slower than clox.

**Changed**

Nothing in A0 changed for the benchmark. What it found along the way:

- Linking dominates builds. `cc` is gcc's driver on Ubuntu, which takes about 105 ms to link even a
  C object, against clang's 72 ms; `CC=clang` brings Minnow's builds to 76–85 ms.
- The tutorial's chapter 7 code (LLVM 18) runs no mem2reg, though the chapter is about it, and LLVM's
  JIT compiles functions lazily, so a compile time must keep the calls (D62).
- clox cannot run the book's `string_equality` benchmark (more than 256 constants in a chunk).
  clox prints numbers with `%g`, where A0's Lox prints integers in full (`1e+07` against
  `10000002`).

**Open questions**

- Link with clang where `cc` is gcc, or with a faster linker, when one is installed: about 35 ms of
  every build.
- Minnow's arrays: a way to allocate n elements, and arrays of `bool` and bytes narrower than a
  word.
- Lox: method calls without a bound method (as clox's `OP_INVOKE`), and property names resolved
  once instead of looked up by string.
- The collector marks the whole live heap at each collection: half of Lox's `binary_trees` time,
  41% of Minnow's `binarytrees`.

## Overall open questions

None at the moment.

Settled since earlier milestones: nullable references are narrowed with `if let` rather than flow
typing (D34); aarch64-windows is out of scope; the collector lives in `a0rt`
(D16); linking on Linux and macOS is tested in CI on every push (M8); a reusable language driver
exists (M7); D2's line for C-bindable APIs held when the library was bound in C (M16).
