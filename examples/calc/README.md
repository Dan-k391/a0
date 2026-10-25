# Calc

A calculator language defined by its grammar alone: [`calc.a0g`](calc.a0g) is all there is. Each
rule says what its nodes mean, with `@hir(form)` or by being named after a form, and `a0 lang` runs
programs by those meanings (`docs/grammar.md`, "Languages without host code"; decision D55).

```
# compound interest
let total = 1000
repeat 10 {
  total = total * 1.05
}
print "after ten years:", round(total)

fn hypot(a, b) {
  return sqrt(a * a + b * b)
}
print hypot(3, 4)
```

- Numbers (floats), strings and booleans; `let`, assignment, `print` of several values.
- `if`/`else`, `while`, `repeat n { ... }`, functions with inferred parameter and result types,
  called before or after they are defined.
- The standard library's mathematics (`sqrt`, `floor`, `round`, `pow`, `pi()`, ...) and the HIR's
  builtins (`str`, `len`, ...).

Variables at the top level belong to the program, not to functions, which see only their
parameters.

```
a0 lang examples/calc/calc.a0g run examples/calc/programs/loops.calc
a0 lang examples/calc/calc.a0g build examples/calc/programs/loops.calc -o loops
a0 lang examples/calc/calc.a0g hir examples/calc/programs/functions.calc
```

Errors are A0's, in the program's terms:

```
error[E2001]: mismatched types
 --> bad.calc:2:11
  |
2 | print x + "a"
  |       -   ^^^ expected `float`, found `string`
  |       |
  |       expected because this is `float`
```

The language server is `a0 lang calc.a0g lsp`; in VS Code, an entry of `a0.languages`
(`docs/editors.md`):

```json
{ "extensions": [".calc"], "command": "a0", "args": ["lang", "${workspaceFolder}/examples/calc/calc.a0g", "lsp"] }
```

`crates/a0/tests/dsl.rs` runs the programs in `programs/` interpreted, natively, under GC stress
and without the optimizer, and checks the language server.
