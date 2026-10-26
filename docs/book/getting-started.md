# Getting started

## Build A0

A0 is written in Rust and needs a C compiler for its runtime: `cc` on Linux and macOS, MSVC on
Windows (found automatically). From a checkout:

```
cargo build --release
target/release/a0 --help
target/release/b0 run b0/tests/programs/prelude.b0
```

Release archives (`docs/releases.md`) carry `a0`, the example languages and the target packs, so
that nothing needs building to use them.

## Make a language

`a0 new` makes a language project: a grammar, a compiler to HIR, a driver, tests, and editor
settings, which builds and passes its tests as it is:

```
a0 new mylang --ext ml
cd mylang
cargo run -- run examples/hello.ml
cargo test
```

The project depends on A0's crates (by path inside this repository, or `--a0 PATH`; from git,
pinned to a release, elsewhere: `a0-hir = { git = "https://github.com/Dan-k391/a0", tag =
"v1.0.0" }`). Its files:

| File | What to change |
|---|---|
| `mylang.a0g` | the syntax: tokens, rules, `@def` and `@scope` for the editor (`docs/grammar.md`) |
| `src/lib.rs` | the compiler: a walk over the syntax tree that builds HIR by name (`docs/hir.md`) |
| `src/main.rs` | the command: `Language::new(...)` and what it has (a REPL, modules, a C runtime) |
| `tests/` | programs with their expected output, run interpreted, in the JIT and natively (`programs.rs`); the language server and the debugger (`lsp.rs`, `dap.rs`) |

## Run it every way

With the project built (or through `cargo run --`), the command does all of this:

```
mylang run prog.ml          # compiled into memory and run (A0_INTERP=1 interprets)
mylang build prog.ml -o p   # an executable; --target x86_64-windows for another platform
mylang build -g prog.ml     # with debug information for gdb, lldb or Visual Studio
mylang hir prog.ml          # the HIR the compiler built
mylang check prog.ml        # diagnostics only
mylang repl                 # a read-eval-print loop
mylang lsp                  # the language server (VS Code: editors/vscode)
mylang dap                  # the debugger
mylang explain E2001        # what an error code means
```

## Next

[The tutorial](../tutorial.md) builds a complete small language, Kaleidoscope, and explains each
piece as it goes. To start a bigger language, read [Building a full language](b0.md) and B0's
compiler, which is meant to be started from.
