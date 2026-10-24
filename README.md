# A0

A0 is an easy-to-use, front-to-back compiler framework: from a grammar to a native executable, for
general-purpose languages and DSLs alike. Ease of use comes first; efficiency comes mainly from reusing
existing backends (Cranelift) rather than building our own.

The core is written in Rust; the runtime is written in C.

## Layout

| Path | What it is |
|---|---|
| `crates/a0-diag` | Source files, spans and diagnostics shared by every layer |
| `crates/a0-ir` | The core IR: data model, builder, text format, verifier |
| `crates/a0-runtime` | The C runtime (`include/a0rt.h`, `c/`) and its Rust build glue |
| `crates/a0-interp` | Interpreter for the IR |
| `crates/a0-target` | Explicit target descriptions |
| `crates/a0-native` | Native code through Cranelift, linked by the system linker |
| `crates/a0` | The `a0` command-line driver |
| `tests/programs` | IR programs with expected output, run by every backend |
| `docs/ir.md` | IR design |
| `docs/decisions.md` | Design decisions |

## Building and testing

Requires a Rust toolchain and a C compiler (MSVC on Windows, `cc` elsewhere).

```
cargo build
cargo test
```

## Using the driver

```
a0 check prog.a0ir     # parse and verify
a0 fmt prog.a0ir       # print the canonical form
a0 run prog.a0ir       # interpret @main
a0 build prog.a0ir     # native executable for the host
a0 obj --target aarch64-linux prog.a0ir   # object file for any supported target
```
