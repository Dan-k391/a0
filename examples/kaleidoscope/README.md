# Kaleidoscope on A0

[Kaleidoscope](https://llvm.org/docs/tutorial/MyFirstLanguageFrontend/) is the language of LLVM's
tutorial. This crate implements it through chapter 7 on A0: doubles only, `def`/`extern`,
`if`/`then`/`else`, `for`/`in`, mutable variables (`var`/`in`, `=`), and user-defined binary and unary
operators.

| File | Role |
|---|---|
| `kaleidoscope.a0g` | the grammar (binary operators are parsed as flat chains) |
| `src/lib.rs` | syntax tree to A0's high-level IR, built by name; `resolve_precedence` for operator chains |
| `runtime/kaleidoscope.c` | `putchard` and `printd` |
| `src/main.rs` | the `kaleidoscope` command, an `a0-driver` language |

```
kaleidoscope run tests/programs/mandelbrot.k
kaleidoscope build tests/programs/fib.k -o fib
kaleidoscope hir tests/programs/fib.k     # the HIR it builds
kaleidoscope ir tests/programs/fib.k      # the core IR that becomes
kaleidoscope lsp                          # the language server
```

`docs/tutorial.md` builds this crate step by step. It is built on the HIR like Minnow, so the HIR
builder resolves its names and checks its calls, and the optimizer inlines its user-defined
operators. Before (when it emitted the core IR itself, with its own scopes) it was 745 lines of Rust
and grammar; now it is 579, not counting the markers the tutorial's listings are cut from.

Differences from the tutorial: `putchard` and `printd` write to stdout (the tutorial uses stderr);
top-level expressions run in order when the program starts instead of being evaluated by a REPL;
redefining a function, `main`, or a built-in operator is an error; operators must be defined before
they are used (the tutorial's parser has the same requirement, but reports it differently).
