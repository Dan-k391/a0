# {{Name}}

A language built on [A0](https://github.com/Dan-k391/a0), made with `a0 new {{name}}`. It starts
as a small typed language (functions, `int`, `float`, `bool`, `string`, `let`, `if`, `while`,
`print`); change it into yours.

```
cargo build
target/debug/{{name}} run examples/hello.{{ext}}               # interpret
target/debug/{{name}} build examples/hello.{{ext}} -o hello    # native executable
target/debug/{{name}} hir examples/hello.{{ext}}               # the HIR it compiles to
cargo test                                                # its programs, both ways, and its language server
```

| File | |
|---|---|
| `{{name}}.a0g` | the grammar: tokens, rules, operator precedence, error messages, and `@def`/`@scope` for editors |
| `src/lib.rs` | the compiler: a walk over the syntax tree building A0's HIR by name |
| `src/main.rs` | the `{{name}}` command: `run`, `build`, `ir`, `hir`, `lsp`, `explain` |
| `tests/programs/*.{{ext}}` | programs with their expected output, run interpreted and natively |
| `tests/lsp.rs` | the language server on a small program |
| `.vscode/settings.json` | VS Code: starts `{{name}} lsp` for `.{{ext}}` files |

To add a feature, add its syntax to the grammar (try it with `a0 parse {{name}}.a0g file.{{ext}}`),
build it in `src/lib.rs` with the builder (`docs/hir.md` in A0), and add a program to
`tests/programs` that shows it. Type errors, runtime errors, garbage collection, the optimizer and
the editor features come from A0.

## Editors

With A0's VS Code extension (`editors/vscode` in A0), `.vscode/settings.json` starts the language
server built here for `.{{ext}}` files: diagnostics, highlighting, outline, go-to-definition and
hover. Any other LSP client runs `{{name}} lsp`.
