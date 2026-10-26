# Compatibility

From 1.0.0, A0 follows [Semantic Versioning](https://semver.org/): a release that is not a new
major version does not break what this page calls the public interface. A language built on A0 1.0
keeps building, and its programs keep running the same way, with every A0 1.x. Decision D86 says
why the line is drawn where it is.

## The public interface

| Part | Covered | Reference |
|---|---|---|
| The command line | `a0`'s commands and every language driver's (`run`, `build`, `check`, `hir`, `ir`, `lsp`, `dap`, `repl`, `explain`, `update`, ...), their options, exit codes, and the environment variables documented as settings (`A0_INTERP`, `A0_OPT`, `A0_HOME`, `A0_LINK`, `A0_CODEGEN_UNITS`, `A0_GC_STATS`, `A0_TIME`) | `a0 --help`, `docs/` |
| Text formats | HIR text and core IR text (format version 1), grammars (`.a0g`, version 1), manifests and lock files | `docs/hir.md`, `docs/ir.md`, `docs/grammar.md`, `docs/packages.md` |
| The C API | everything in `crates/a0-capi/include/a0.h`: functions, types, constants and their meaning; source and binary compatible | `docs/c-api.md` |
| The Python package | `a0` in `bindings/python`: its classes, functions and constants | `docs/python.md` |
| The Rust API for languages | the public items of `a0-diag`, `a0-syntax`, `a0-sema`, `a0-ir`, `a0-hir`, `a0-driver` and `a0-build`, and what they are documented to do | `cargo doc`, `docs/` |
| The standard library | the modules in `docs/std.md` and their functions | `docs/std.md` |
| The runtime for languages' C runtimes | the part of `a0rt.h` that `docs/runtime.md` lists | `docs/runtime.md` |
| Diagnostic codes | a code keeps its meaning and is never reused | `a0 explain`, `docs/diagnostics.md` |

What a program does is covered too: a program that a language compiles today prints the same and
exits the same way in every later 1.x release, interpreted, compiled into memory and compiled to an
executable, except where it depended on a bug.

## Not covered

- **Implementation crates**: `a0-native`, `a0-interp`, `a0-wasm`, `a0-runtime` (its Rust items),
  `a0-target`, `a0-capi` (its Rust items; the C header is covered), `a0-playground` and the `a0`
  binary crate. Their public items serve A0's own crates and may change in any release. What a
  language needs from them is re-exported by `a0-driver` (`a0_driver::backend`), and covered there.
  A language with a C runtime depends on `a0-runtime` for its header only.
- **Wording and layout**: the text of messages, labels and notes, how diagnostics, stack traces and
  `--help` are laid out, and the spacing of printed HIR and IR (which parses the same either way).
- **What the compiler makes of a program**: generated code, the optimizer's decisions, object
  files, debug information beyond what debuggers show, and performance (tracked in
  `docs/benchmarks.md`, not promised). Objects and executables are not compatible between
  releases: link only what one release built.
- **Settings for A0's own development**: `A0_GC_STRESS`, `A0_BLESS`, `A0_FUZZ_*`, `A0_RANDOM_*`,
  `A0_CROSS_RUNNERS`, and the test suites.
- **Anything marked unstable** in its documentation.

## Rules for each part

- **Rust.** Public enums that can grow are `#[non_exhaustive]`, and so are structs whose fields may
  grow: build them with their constructors (`Language::new`, `Label::new`,
  `LowerOptions::optimized`) or from `Default` and assign fields. A few enums that A0's own crates
  must match exhaustively, so that none misses a new variant (the core IR's `Opcode`, `Type`,
  `DebugKind` and `StructPass` and its `Item`, the HIR's `Item`, `a0_syntax::SymbolKind` and
  `TokenClass`, `a0_diag::Severity`), say in their documentation that they may grow: match them
  with a `_` arm. New items, variants and fields come in minor releases. The minimum Rust version
  is the `rust-version` in `Cargo.toml`; raising it is a minor change, announced in the changelog.
- **C.** Functions are never removed or given another signature within a major version; new ones
  are added. Handles and type codes keep their values. `A0_VERSION_MAJOR`, `A0_VERSION_MINOR` and
  `A0_VERSION_PATCH` in `a0.h` name the header's version, and `a0_version()` the library's: a
  host can check at run time that the library is at least as new as the header it was built with.
- **Python.** Names in `a0.__all__` stay; keyword arguments may be added.
- **Text formats.** A module's text says its format version: `module "m" version 1;` in HIR,
  `module "m" version 1` in core IR, `grammar g version 1;` in a grammar; A0 prints HIR and IR with
  it. Text without one is version 1.
  Every 1.x release reads every version-1 text that an earlier one read or printed. A minor release
  may add constructs to version 1, so text using them needs that release; an older release reports
  them as errors like any other. A text of a newer major version is refused with `E0004`, which
  names the versions this release reads.

## Deprecation

Nothing covered is removed or changed incompatibly within a major version. To retire something:

1. A minor release marks it deprecated: `#[deprecated(since = "1.x", note = "use ...")]` in Rust,
   `A0_DEPRECATED("use ...")` in `a0.h` (a compiler warning in GCC, Clang and MSVC), a
   `DeprecationWarning` in Python, a warning diagnostic for a construct of a text format, and a
   note on standard error for a command-line option. The note names what to use instead.
2. The changelog lists it under **Deprecated**, and so does the release's milestone.
3. It keeps working, unchanged, for the rest of the major version.
4. The next major release may remove it; its changelog lists it under **Removed**, with the
   replacement.

A security fix may break something sooner if there is no other way; the changelog then says so.

## Before 1.0

Releases 0.1 to 0.9 changed APIs in minor versions, as Semantic Versioning allows before 1.0. The
changelog lists every change.
