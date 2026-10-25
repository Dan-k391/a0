# AGENTS.md

How to work on A0. Read `README.md` and `docs/` first. Keep this file current: when a convention
changes, change it here in the same commit.

## Build and test

```
cargo build --workspace
cargo test --workspace              # everything; the Lox suite runs six ways (~35 s)
cargo test -p a0-ir                 # one crate
cargo test -p lox --test suite      # the official Lox suite only
cargo clippy --workspace --all-targets   # must report no warnings
cargo fmt --all                     # run before committing
```

CI (`.github/workflows/ci.yml`) builds and runs every test, native linking included, on x86-64 and
aarch64 Linux, aarch64 macOS and x86-64 Windows, and checks rustfmt and clippy (`-D warnings`). Keep
it green; a test that only passes on one OS is a bug.

Needs a C compiler: MSVC on Windows (found automatically), `cc` on Linux/macOS. Useful while
debugging: `a0 run|build|fmt|check file.a0ir|file.hir`, `a0 lower file.hir` (the core IR a HIR
module becomes), `a0 parse lang.a0g input`, `a0 lang calc.a0g hir|ir|run file` (a language
defined by its grammar alone), `lox hir|ir file.lox`, `minnow hir|ir file.mn`, `kaleidoscope hir|ir file.k`, and
`A0_GC_STRESS=1` (the collector runs on every allocation), `A0_GC_STATS=1` (what the collector did,
printed at exit), `A0_OPT=0` (no HIR optimizer),
`a0 explain E2001` (what an error code means), `A0_BLESS=1` (rewrite the tutorial's listings and the
diagnostic snapshots after a deliberate change).
Every language driver also has `lang lsp`, the language server (`docs/editors.md`); the VS Code
extension in `editors/vscode` starts it.

## Layer map and dependency rules

| Layer | Crates | May depend on |
|---|---|---|
| base | `a0-diag` | nothing |
| frontend | `a0-syntax` (grammar DSL, parser, trees), `a0-sema` (scopes, static types) | `a0-diag` |
| core IR | `a0-ir` (model, builder, text format, verifier) | `a0-diag` |
| high-level IR | `a0-hir` (typed tree, builder with inference, checker, text, monomorphization, optimizer, lowering) | `a0-ir`, `a0-syntax`, `a0-sema`, `a0-diag` |
| targets | `a0-target` | nothing |
| runtime | `a0-runtime` (C in `include/`, `c/`; Rust glue) | nothing |
| backends | `a0-interp`, `a0-native` | `a0-ir`, `a0-runtime`, `a0-target`, `a0-diag` |
| tools | `a0` (CLI), `a0-build` (build-script helper), `a0-driver` (language drivers) | anything |
| C API | `a0-capi` (`include/a0.h`) | library crates, not the tools |
| examples | `examples/*` | anything |
| editors | `editors/vscode` (an extension that starts `lang lsp`; JavaScript) | nothing in Rust |

Never make a lower layer depend on a higher one, and keep the frontend and the core IR independent
of each other. `a0-hir` is the one layer that uses both (its text format is parsed with the grammar
layer, decision D17). A new crate goes into the table above and into `[workspace]` in `Cargo.toml`.

## Conventions

- **Public APIs stay C-bindable** (decision D2 in `docs/decisions.md`): opaque owner types, `u32`
  handles, `&str`/`&[T]` inputs, no lifetimes, generics, closures, trait objects or data-carrying
  enums in public signatures. Returning an owned `String` is fine (the C wrapper keeps it), as are
  enums with a text form (C passes their names) and methods that take `self` (they consume the C
  object). Anything public that C should reach gets a binding in `a0-capi` and its header.
- **Report, don't panic.** Libraries return `Diagnostics` (from `a0-diag`) for anything a user can
  cause. Builders record misuse as diagnostics. `panic!`/`unwrap` only for internal invariants.
- **Diagnostics as good as rustc's.** A problem a program can have gets its own code in
  `a0_diag::codes` (with an explanation), a short message, a primary label, related labels where
  they help (the earlier declaration, the parameter, the result type) and a fix where one is
  likely. Words come from `Term`s and types from `display_type`/`quote_type`, never the HIR's own
  spellings. Misuse by a language's code keeps a descriptive code (`hir.builder`).
- **Interpreter and native code must behave identically**: same output, same stderr, same exit code.
  Traps and panics go through the C runtime (`a0_rt_fail`, exit 101), which prints the message and
  the stack trace from the program's location table; runtime functions that can fail take the
  location and frame pointer (`REPORTS_LOCATION`, `docs/ir.md`).
- No new dependencies without a reason recorded in `docs/decisions.md`. Never a code generator other
  than Cranelift and never our own linker. The one optimizer of our own is the HIR optimizer (D27):
  keep it small, on by default, behavior-preserving, and leave per-function optimization to
  Cranelift.
- Every target-dependent choice goes through `a0_target::Target`; never `cfg!` in codegen.
- Programs of several files are the driver's (`a0-driver/src/program.rs`, D46): it finds imports
  through the grammar's `@import` rules and compiles every file into one HIR builder, a module
  each. Languages never read files themselves; the language server does the same with open
  documents.
- The standard library is HIR text (`crates/a0-hir/std/*.hir`) over `extern "a0"` C functions in
  `crates/a0-runtime/c/std.c` (D48, `docs/std.md`); prefer writing a library function in HIR, and
  add a builtin to the HIR only for what the library cannot express.
- `a0 new` copies `crates/a0/templates/starter` (D54); `crates/a0/tests/new.rs` builds and tests a
  generated project, so a change to the APIs it uses must keep the template working.
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
  and run interpreted, natively, and interpreted with `A0_GC_STRESS=1`, each with the HIR optimizer
  on and off (`A0_OPT=0`) (`crates/a0/tests/programs.rs`). Add a program there for any new IR, HIR,
  optimizer or runtime behavior.
- **Random programs**: `crates/a0/tests/random.rs` generates seeded HIR programs and requires the
  six modes to agree (`A0_RANDOM_PROGRAMS`, `A0_RANDOM_SEED` to fuzz longer). When the optimizer or
  lowering learns something new, teach the generator to produce it.
- HIR text must round-trip: `print(parse(print(m))) == print(m)` is tested for every corpus file.
- Example languages test interpreted and native output (`examples/*/tests`), all three under GC
  stress too, optimized and not. The Lox suite must stay at all 241 passing in all six modes, and the tests of A0's extensions to
  Lox (`examples/lox/tests/a0`) with it.
- Each example language has `tests/lsp.rs`: its server's outline, definitions, hovers,
  diagnostics and quick fixes on a small program, through `a0_driver::lsp::session`.
- Each example language has `tests/diagnostics/`: programs whose full output (rendered
  diagnostics, runtime errors) must match their `.out` files, natively too
  (`a0_driver::snapshots::check`; `A0_BLESS=1` rewrites them). A new kind of diagnostic or runtime
  error gets a snapshot.
- `docs/tutorial.md` is checked against Kaleidoscope (`examples/kaleidoscope/tests/tutorial.rs`):
  listings are `ANCHOR` regions of the code, together they must show all of it, and its commands
  are run. After changing Kaleidoscope, run `A0_BLESS=1 cargo test -p kaleidoscope --test tutorial`
  to rewrite the listings and outputs, then reread the prose around them.
- Object emission is checked for every supported target except aarch64-windows (out of scope).
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
| Diagnostics: rendering, codes (`a0_diag::codes`), the builder's reporting API | `docs/diagnostics.md` (its code table is tested) |
| A design choice with alternatives | a new `D<n>` entry in `docs/decisions.md` |
| A milestone ends | `docs/milestones.md` (done / changed / open questions); `CHANGELOG.md` (under Unreleased) |
| A release | `CHANGELOG.md`, the version in `Cargo.toml`, `README.md` status, a `v<version>` tag |
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
