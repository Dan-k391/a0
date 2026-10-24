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
| `docs/ir.md` | IR design |
| `docs/decisions.md` | Design decisions |

## Building and testing

```
cargo build
cargo test
```
