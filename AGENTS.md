# AGENTS.md

How to work on A0. Read `README.md` and `docs/` first. Keep this file current: when a convention
changes, change it here in the same commit.

## Build and test

```
cargo build --workspace
cargo test --workspace              # everything; the Lox suite runs eight ways (~35 s)
cargo test -p a0-ir                 # one crate
cargo test -p lox --test suite      # the official Lox suite only
cargo test -p a0-capi --test python # the Python bindings and Pebble (needs Python 3)
WASI_SDK=/path/to/wasi-sdk sh playground/build.sh && node playground/test.mjs target/playground
                                    # the playground, built for WebAssembly (Linux, macOS), and the
                                    # IR and HIR corpus compiled to WebAssembly and run there
python3 targets/build.py            # the target packs cross-compilation links against (docs/cross.md;
                                    # Linux, macOS); then `cargo test -p a0 --test programs pack`
python3 bench/run.py                # the benchmark (docs/benchmarks.md; Linux, macOS; not in CI)
A0_FUZZ_ITERATIONS=20000 cargo test --release -p a0 --test fuzz -- --test-threads 1 --nocapture
                                    # fuzz IR and HIR text and grammars longer (D85); likewise
                                    # `-p a0-capi --test fuzz` (the C API) and `-p a0-driver --lib
                                    # mutated_manifests`
cargo clippy --workspace --all-targets   # must report no warnings
cargo fmt --all                     # run before committing
```

CI (`.github/workflows/ci.yml`) builds and runs every test, native linking included, on x86-64 and
aarch64 Linux, macOS and Windows, builds the target packs and runs the corpus through them
(`docs/cross.md`), and checks rustfmt and clippy (`-D warnings`). Keep it green; a test that only
passes on one OS is a bug.

Needs a C compiler: MSVC on Windows (found automatically), `cc` on Linux/macOS. Useful while
debugging: `a0 run|build|fmt|check file.a0ir|file.hir`, `a0 lower file.hir` (the core IR a HIR
module becomes), `a0 parse lang.a0g input`, `a0 lang calc.a0g hir|ir|run file` (a language
defined by its grammar alone), `lox hir|ir file.lox`, `b0 hir|ir file.b0`, `minnow hir|ir file.mn`, `kaleidoscope hir|ir file.k`, `tin hir|ir file.tin`, and
`A0_INTERP=1` (`run` interprets instead of compiling into memory with the JIT), `A0_TIME=1` (how
long each stage of compiling takes), `A0_CODEGEN_UNITS=n` (compile into `n` objects, as large
programs are),
`A0_GC_STRESS=1` (the collector runs on every allocation), `A0_GC_STATS=1` (what the collector did,
printed at exit), `A0_OPT=0` (no HIR optimizer),
`a0 explain E2001` (what an error code means), `A0_BLESS=1` (rewrite the tutorial's listings and the
diagnostic snapshots after a deliberate change).
Every language driver also has `lang lsp`, the language server (`docs/editors.md`), and `lang dap`,
the debugger (`docs/debugging.md`); the VS Code extension in `editors/vscode` starts both.
`lang build -g` writes DWARF for gdb and lldb, and `lang repl` is a REPL (`docs/repl.md`).

## Layer map and dependency rules

| Layer | Crates | May depend on |
|---|---|---|
| base | `a0-diag` | nothing |
| frontend | `a0-syntax` (grammar DSL, parser, trees), `a0-sema` (scopes, static types) | `a0-diag` |
| core IR | `a0-ir` (model, builder, text format, verifier) | `a0-diag` |
| high-level IR | `a0-hir` (typed tree, builder with inference, checker, text, monomorphization, optimizer, lowering) | `a0-ir`, `a0-syntax`, `a0-sema`, `a0-diag` |
| targets | `a0-target` | nothing |
| runtime | `a0-runtime` (C in `include/`, `c/`; Rust glue) | nothing |
| backends | `a0-interp`, `a0-native`, `a0-wasm` (WebAssembly modules that link against a runtime instance, D78) | `a0-ir`, `a0-runtime`, `a0-target`, `a0-diag` |
| tools | `a0` (CLI), `a0-build` (build-script helper), `a0-driver` (language drivers), `a0-playground` (the playground's engine; no native backend, builds for wasm32-wasip1) | anything |
| C API | `a0-capi` (`include/a0.h`) | library crates and `a0-driver` (D67), not the other tools |
| flagship | `b0` (B0, D87) | only the crates `docs/compatibility.md` covers (`b0/tests/public_api.rs`) |
| examples | `examples/*` | anything |
| editors | `editors/vscode` (an extension that starts `lang lsp`; JavaScript) | nothing in Rust |
| bindings | `bindings/python` (the package `a0`, ctypes over the C API's shared library) | the C API |
| web | `playground/` (the browser playground: a page, a WASI shim and a worker; JavaScript) | the engine's wasm |
| benchmark | `bench/` (programs in Minnow, Kaleidoscope, C and Go; `run.py`, Python) | the built tools |

Never make a lower layer depend on a higher one, and keep the frontend and the core IR independent
of each other. `a0-hir` is the one layer that uses both (its text format is parsed with the grammar
layer, decision D17). A new crate goes into the table above and into `[workspace]` in `Cargo.toml`.

## Conventions

- **Public APIs stay C-bindable** (decision D2 in `docs/decisions.md`): opaque owner types, `u32`
  handles, `&str`/`&[T]` inputs, no lifetimes, generics, closures, trait objects or data-carrying
  enums in public signatures. Returning an owned `String` is fine (the C wrapper keeps it), as are
  enums with a text form (C passes their names) and methods that take `self` (they consume the C
  object). Anything public that C should reach gets a binding in `a0-capi` and its header, and a
  builder function also gets a line in the Python table (`bindings/python/a0/__init__.py`), which a
  test checks against the header (D56).
- **Compatibility** (`docs/compatibility.md`, D86). From 1.0, nothing covered breaks before 2.0:
  the command line, the text formats, the C header, the Python package, the Rust API of
  `a0-diag`, `a0-syntax`, `a0-sema`, `a0-ir`, `a0-hir`, `a0-driver` and `a0-build`, and the part of
  `a0rt.h` in `docs/runtime.md`. Add rather than change; a new public enum or struct that may grow
  is `#[non_exhaustive]` (with a constructor), except enums A0's own crates must match
  exhaustively, which say in their docs that they may grow. Retire things by deprecating them
  (`#[deprecated]`, `A0_DEPRECATED`, `a0._deprecated`) and listing them under **Deprecated** in the
  changelog. Languages use only the covered crates (`a0_driver::backend` for C runtimes); a text
  format change keeps every version-1 text reading the same.
- **Report, don't panic.** Libraries return `Diagnostics` (from `a0-diag`) for anything a user can
  cause. Builders record misuse as diagnostics. `panic!`/`unwrap` only for internal invariants.
- **Diagnostics as good as rustc's.** A problem a program can have gets its own code in
  `a0_diag::codes` (with an explanation), a short message, a primary label, related labels where
  they help (the earlier declaration, the parameter, the result type) and a fix where one is
  likely. Words come from `Term`s and types from `display_type`/`quote_type`, never the HIR's own
  spellings. Misuse by a language's code keeps a descriptive code (`hir.builder`).
- **Interpreter, JIT and native code must behave identically**: same output, same stderr, same exit
  code. `run` uses the JIT (D63); the interpreter is the reference and the debugger's engine.
  Traps and panics go through the C runtime (`a0_rt_fail`, exit 101), which prints the message and
  the stack trace from the program's location table; runtime functions that can fail take the
  location and frame pointer (`REPORTS_LOCATION`, `docs/ir.md`).
- No new dependencies without a reason recorded in `docs/decisions.md`. Never a code generator other
  than Cranelift (and the direct translation to WebAssembly, D78) and never our own linker. The one optimizer of our own is the HIR optimizer (D27):
  keep it small, on by default, behavior-preserving, and leave per-function optimization to
  Cranelift.
- Every target-dependent choice goes through `a0_target::Target`; never `cfg!` in codegen.
- Programs of several files are the driver's (`a0-driver/src/program.rs`, D46): it finds imports
  through the grammar's `@import` rules and compiles every file into one HIR builder, a module
  each. Languages never read files themselves; the language server does the same with open
  documents. Packages (`package.rs`, D81) give a program its manifest and dependencies; builds
  must stay reproducible (`examples/minnow/tests/packages.rs` compares two builds byte for byte).
- The standard library is HIR text (`crates/a0-hir/std/*.hir`) over `extern "a0"` C functions in
  `crates/a0-runtime/c/std.c` (D48, `docs/std.md`); prefer writing a library function in HIR, and
  add a builtin to the HIR only for what the library cannot express.
- `a0 new` copies `crates/a0/templates/starter` (D54); `crates/a0/tests/new.rs` builds and tests a
  generated project, so a change to the APIs it uses must keep the template working.
- The runtime builds for wasm32-wasip1 too (D57, `docs/playground.md`): fields that generated code
  reads as pointers are 64-bit words (`A0_WORD_PTR`), and the interpreter calls C through
  wrappers generated from `a0rt.h`, so every function in the symbol table needs its prototype
  there. `cfg!` is fine for what the host can do (threads, stack); codegen still goes through
  `Target`.
- Debuggers need spans on everything a frontend builds (a stale span puts code on another line)
  and output that goes through `a0_rt_write` (C runtimes of languages included), D58.
- Languages without host code (D55): `@hir` forms in `a0_hir::dsl`, run by `a0 lang`; Calc
  (`examples/calc`, tested by `crates/a0/tests/dsl.rs`) is the example. Keep the forms few.
- Generalize only when two example languages need it (except the IR layers and the default runtime).
- Rust edition 2024, rustfmt defaults, clippy clean. Comments explain why, not what; match the
  surrounding density. C code: C11, `a0_` prefix for runtime symbols, no global state without a reason.
- Files are LF (`.gitattributes`); Windows checkouts must not reintroduce CRLF.

## Testing requirements

- Every change comes with tests in the crate it touches; bugs get a regression test.
- **Differential testing**: core IR programs in `tests/programs/*.a0ir` and HIR programs in
  `tests/hir/*.hir` declare `expect-stdout:`, `expect-stderr:` and `expect-exit:` (after `;` or `//`)
  and run interpreted (`A0_INTERP=1`), in the JIT, natively, and in the JIT with `A0_GC_STRESS=1`,
  each with the HIR optimizer on and off (`A0_OPT=0`) (`crates/a0/tests/programs.rs`). Add a program there for any new IR, HIR,
  optimizer or runtime behavior.
- **Random programs**: `crates/a0/tests/random.rs` generates seeded HIR programs and requires the
  eight modes to agree (`A0_RANDOM_PROGRAMS`, `A0_RANDOM_SEED` to fuzz longer). When the optimizer or
  lowering learns something new, teach the generator to produce it. One program in four manages its
  memory itself and one in three runs calls on threads.
- **Threads**: races show up on machines with few processors more than on many. After touching
  the collector, the runtime's locks or threaded lowering, run a threaded program under
  `A0_GC_STRESS=1` many times pinned to two and four processors (`taskset -c 0,1` on Linux), as
  CI's runners have; `examples/minnow/tests/parallel.rs` checks that threads really run at once.
- HIR text must round-trip: `print(parse(print(m))) == print(m)` is tested for every corpus file.
- **Fuzzing** (D85): every input A0 accepts is fuzzed from the corpus in every test run, a few
  hundred inputs each: IR and HIR text and grammars (`crates/a0/tests/fuzz.rs`), the C API
  (`crates/a0-capi/tests/fuzz.rs`, through Python) and manifests (`a0-driver`). Errors are fine; a
  panic, a hang or an internal error is a bug. After changing a parser, the verifier, lowering or
  the C API, fuzz longer (`A0_FUZZ_ITERATIONS`, `A0_FUZZ_SEED`), and give what it finds a
  regression test where the bug is.
- B0 (`b0/`) is tested like an example language, and its features each have a program in
  `b0/tests/programs`; a change to A0 that B0 needs must not need anything outside the covered API.
- Example languages test interpreted, JIT and native output (`examples/*/tests`), and the JIT
  under GC stress, optimized and not. The Lox suite must stay at all 241 passing in all eight modes, and the tests of A0's extensions to
  Lox (`examples/lox/tests/a0`) with it.
- Each example language has `tests/lsp.rs`: its server's outline, definitions, hovers,
  diagnostics and quick fixes on a small program, through `a0_driver::lsp::session`; and
  `tests/dap.rs`: a debugging session (breakpoint, stack, locals, steps) through
  `a0_driver::dap::Client`; and `tests/repl.rs`: a REPL session (`lang repl` on piped input).
- Each example language has `tests/diagnostics/`: programs whose full output (rendered
  diagnostics, runtime errors) must match their `.out` files, natively too
  (`a0_driver::snapshots::check`; `A0_BLESS=1` rewrites them). A new kind of diagnostic or runtime
  error gets a snapshot.
- `docs/tutorial.md` is checked against Kaleidoscope (`examples/kaleidoscope/tests/tutorial.rs`):
  listings are `ANCHOR` regions of the code, together they must show all of it, and its commands
  are run. After changing Kaleidoscope, run `A0_BLESS=1 cargo test -p kaleidoscope --test tutorial`
  to rewrite the listings and outputs, then reread the prose around them.
- Object emission is checked for every supported target; aarch64-windows needs the patched
  `cranelift-object` in `vendor/` (`vendor/README.md`).
- Native debug information (`crates/a0/tests/debuginfo.rs`): DWARF and CodeView are read back from
  objects on every host, and gdb, lldb or cdb debugs a built executable where one is installed.
  After changing `dwarf.rs`, `codeview.rs` or what `-g` does in codegen, run them with a debugger
  present (on Windows, WinDbg's cdb: `winget install Microsoft.WinDbg`).
- Cross-compilation: with packs built (`python3 targets/build.py`), `cargo test -p a0 --test
  programs pack` and `cargo test -p tin --test tin pack`; `A0_CROSS_RUNNERS` runs other targets'
  executables (QEMU).
- Runtime or GC changes: run the Lox suite under GC stress, and prove the stress test still fails
  when roots are broken if you touch rooting.

## Keeping docs in sync

| When you change | Update |
|---|---|
| IR instructions, types, text format, builder, locations and traces | `docs/ir.md` |
| HIR types, expressions, builtins, text format, lowering | `docs/hir.md` (and `hir.a0g` + `print.rs` together) |
| The standard library | `docs/std.md` |
| Grammar DSL (including `@def`/`@scope`) | `docs/grammar.md` |
| The language server or the editor extension | `docs/editors.md` |
| The debugger (`dap.rs`, debug types, the runtime's hooks) or native debug information | `docs/debugging.md` |
| The REPL (`repl.rs`, `Language::repl_main`) | `docs/repl.md` |
| Linking, target packs (`link.rs`, `targets/build.py`, `start_windows.c`) | `docs/cross.md` |
| Packages, manifests, lock files (`package.rs`, `program.rs`) | `docs/packages.md` |
| Diagnostics: rendering, codes (`a0_diag::codes`), the builder's reporting API | `docs/diagnostics.md` (its code table is tested) |
| A design choice with alternatives | a new `D<n>` entry in `docs/decisions.md` |
| A milestone ends | `docs/milestones.md` (done / changed / open questions); `CHANGELOG.md` (under Unreleased) |
| A release | `CHANGELOG.md`, the version in `Cargo.toml`, `A0_VERSION_*` in `a0.h`, `__version__` in `bindings/python/a0`, `editors/vscode/package.json`, `README.md` status, a `v<version>` tag |
| The covered API (`docs/compatibility.md`) or what languages' C runtimes may use | `docs/compatibility.md`, `docs/runtime.md`, and the changelog (**Deprecated** for what is retired) |
| What the benchmark measures, or anything that moves its numbers much (codegen, the optimizer, the collector, linking) | `docs/benchmarks.md`, rerunning `bench/run.py` |
| A workflow or convention | this file and `.claude/skills/` |

## Commits

- Small, focused commits of working states: build, tests, clippy and rustfmt pass before committing.
- Imperative subject line (< 72 chars), a body that says why, and the trailer
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` for agent-written commits.
- Commit as the repository's configured git user. Push only when the user has allowed it.
- Do not commit `instructions.md` or other files the user left untracked.

## Skills

Step-by-step workflows live in `.claude/skills/`:
`add-ir-instruction`, `add-hir-feature`, `add-runtime-function`, `add-example-language`,
`differential-tests`.
