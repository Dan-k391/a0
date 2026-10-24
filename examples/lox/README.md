# Lox on A0

[Lox](https://craftinginterpreters.com/) is the language of Robert Nystrom's *Crafting
Interpreters*. This crate implements all of it (classes, inheritance, closures, garbage collection)
on A0's high-level IR, and passes the book's official test suite: all 241 tests of the `jlox` suite
configuration, interpreted, as native executables and under GC stress, plus
`number/nan_equality.lox` and `limit/stack_overflow.lox`, which the `jlox` suite skips because of JVM
behavior.

| File | Role |
|---|---|
| `lox.a0g` | the grammar; it reproduces jlox's syntax errors and recovery |
| `src/analyze.rs` | jlox's parse-time checks and resolver |
| `src/emit.rs` | HIR generation: one closure per Lox function, a `main` for the script |
| `prelude.hir` | Lox's runtime semantics, written in HIR: calls, classes, operators, errors |
| `src/main.rs` | the `lox` driver |
| `tests/craftinginterpreters` | the official test suite (MIT), run by `tests/suite.rs` |

```
lox script.lox                 # interpret (like jlox)
lox build script.lox -o app    # native executable
lox hir script.lox             # print the HIR (prelude and program)
lox ir script.lox              # print the core IR it lowers to
```

Compile errors exit with 65 and runtime errors with 70, in jlox's format. `A0_GC_STRESS=1` makes the
collector run on every allocation; the suite also runs that way.

## How it works

- **Values.** Every Lox value is a HIR `any`: numbers are floats, `nil` is `()`, strings are strings,
  and functions, classes, instances and bound methods are the prelude's structs.
- **Functions** become HIR closures `fn(this: any, args: array<any>) -> any` wrapped in a
  `LoxFunction`. The prelude's `call` checks arity, binds methods and creates instances. Captured
  variables, the shadow stack and the collector are the HIR's job, so the compiler never sees them.
- **Globals** are HIR globals, each with a flag saying whether it has been defined yet, so using an
  undefined global is a runtime error, as in jlox.
- **Classes** copy their superclass's methods on creation; `this` and `super` are ordinary locals that
  closures capture.
- **Stack overflow.** `call` counts nesting and reports `Stack overflow.` past 3000 Lox calls.

Number formatting uses the shortest round-trip representation, which matches jlox for every number
the suite prints (jlox would print `1.0E21` where this prints `1e+21`).

Before the HIR existed, Lox compiled straight to the core IR with its own C runtime and collector
(2647 lines); on the HIR it is 1544 lines, of which 212 are the HIR prelude.
