# AGENTS.md

How to work on A0. Read `README.md` and `docs/` first. Keep this file current: when a convention
changes, change it here in the same commit.

## Build and test

```
cargo build --workspace
cargo test --workspace              # everything; the Lox suite runs three ways (~20 s)
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
module becomes), `a0 parse lang.a0g input`, `lox hir|ir file.lox`, `minnow hir|ir file.mn`, `kaleidoscope ir file.k`, and
`A0_GC_STRESS=1` (the collector runs on every allocation), `A0_OPT=0` (no HIR optimizer).
Every language driver also has `lang lsp`, the language server (`docs/editors.md`); the VS Code
extension in `editors/vscode` starts it.

## Layer map and dependency rules

| Layer | Crates | May depend on |
|---|---|---|
| base | `a0-diag` | nothing |
| frontend | `a0-syntax` (grammar DSL, parser, trees), `a0-sema` (scopes, static types) | `a0-diag` |
| core IR | `a0-ir` (model, builder, text format, verifier) | `a0-diag` |
| high-level IR | `a0-hir` (typed tree, builder, checker, text, lowering) | `a0-ir`, `a0-syntax`, `a0-sema`, `a0-diag` |
| targets | `a0-target` | nothing |
| runtime | `a0-runtime` (C in `include/`, `c/`; Rust glue) | nothing |
| backends | `a0-interp`, `a0-native` | `a0-ir`, `a0-runtime`, `a0-target`, `a0-diag` |
| tools | `a0` (CLI), `a0-build` (build-script helper), `a0-driver` (language drivers) | anything |
| examples | `examples/*` | anything |
| editors | `editors/vscode` (an extension that starts `lang lsp`; JavaScript) | nothing in Rust |

Never make a lower layer depend on a higher one, and keep the frontend and the core IR independent
of each other. `a0-hir` is the one layer that uses both (its text format is parsed with the grammar
layer, decision D17). A new crate goes into the table above and into `[workspace]` in `Cargo.toml`.

## Conventions

- **Public APIs stay C-bindable** (decision D2 in `docs/decisions.md`): opaque owner types, `u32`
  handles, `&str`/`&[T]` inputs, no lifetimes, generics, closures, trait objects or data-carrying
  enums in public signatures.
- **Report, don't panic.** Libraries return `Diagnostics` (from `a0-diag`) for anything a user can
  cause. Builders record misuse as diagnostics. `panic!`/`unwrap` only for internal invariants.
- **Interpreter and native code must behave identically**: same output, same stderr, same exit code.
  Traps and panics go through the C runtime (`a0_rt_trap`, exit 101).
- No new dependencies without a reason recorded in `docs/decisions.md`. Never a code generator other
  than Cranelift, never our own optimizer or linker.
- Every target-dependent choice goes through `a0_target::Target`; never `cfg!` in codegen.
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
- HIR text must round-trip: `print(parse(print(m))) == print(m)` is tested for every corpus file.
- Example languages test interpreted and native output (`examples/*/tests`); languages on the HIR
  (Lox, Minnow) also run under GC stress, optimized and not. The Lox suite must stay at all 241
  passing in all six modes.
- Each example language has `tests/lsp.rs`: its server's outline, definitions, hovers and
  diagnostics on a small program, through `a0_driver::lsp::session`.
- Object emission is checked for every supported target except aarch64-windows (out of scope).
- Runtime or GC changes: run the Lox suite under GC stress, and prove the stress test still fails
  when roots are broken if you touch rooting.

## Keeping docs in sync

| When you change | Update |
|---|---|
| IR instructions, types, text format, builder | `docs/ir.md` |
| HIR types, expressions, builtins, text format, lowering | `docs/hir.md` (and `hir.a0g` + `print.rs` together) |
| Grammar DSL (including `@def`/`@scope`) | `docs/grammar.md` |
| The language server or the editor extension | `docs/editors.md` |
| A design choice with alternatives | a new `D<n>` entry in `docs/decisions.md` |
| A milestone ends | `docs/milestones.md` (done / changed / open questions), `README.md` status |
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
