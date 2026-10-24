# Lox on A0

[Lox](https://craftinginterpreters.com/) is the language of Robert Nystrom's *Crafting
Interpreters*. This crate implements all of it (classes, inheritance, closures, garbage collection)
on A0, and passes the book's official test suite: all 241 tests of the `jlox` suite configuration,
interpreted and as native executables, plus `number/nan_equality.lox` and `limit/stack_overflow.lox`,
which the `jlox` suite skips because of JVM behavior.

| File | Role |
|---|---|
| `lox.a0g` | the grammar; it reproduces jlox's syntax errors and recovery |
| `src/analyze.rs` | jlox's parse-time checks and resolver; finds the variables closures capture |
| `src/codegen.rs` | A0 IR generation |
| `runtime/lox.c` | values, objects, strings, tables, garbage collector, dynamic operations |
| `src/main.rs` | the `lox` driver |
| `tests/craftinginterpreters` | the official test suite (MIT), run by `tests/suite.rs` |

```
lox script.lox                 # interpret (like jlox)
lox build script.lox -o app    # native executable
lox ir script.lox              # print the A0 IR
```

Compile errors exit with 65 and runtime errors with 70, in jlox's format. `LOX_GC_STRESS=1` makes the
collector run on every allocation; the suite also runs that way.

## How it works

- **Values** are NaN-boxed 64-bit words (numbers, `nil`, booleans, object pointers).
- **Roots.** Compiled functions keep every live value in a frame on the runtime's shadow stack
  (`lox_frame_enter`/`lox_frame_leave`). IR values never hold a Lox value across a runtime call that
  can allocate, so the mark-sweep collector finds every root precisely.
- **Calls.** A call puts `[callee, receiver, args...]` in consecutive frame slots;
  `lox_call_prepare` checks arity, binds `this` for methods and creates instances for classes, then
  returns the code to call with a pointer to that window. Every compiled function, native and stub
  has the IR signature `(ptr) -> i64`.
- **Closures.** Variables captured by inner functions live in heap cells; closures carry the cells,
  and a function copies them into its frame on entry.
- **Classes** copy their superclass's methods on creation; `super` and `this` are ordinary captured
  variables.

Number formatting uses the shortest round-trip representation, which matches jlox for every number
the suite prints (jlox would print `1.0E21` where this prints `1e+21`).
