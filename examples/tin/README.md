# Tin

A small C-like systems language built on A0: structs are values laid out as C lays them out,
pointers are raw, memory is the program's to allocate and free (no collector runs), and C functions
are declared and called directly, structs passed to them and returned by value as C does. It is
the example of a systems language on the HIR, and what v0.6 ("systems languages") is measured by.

```
struct Vec2 { x: f64, y: f64 }
struct Node { value: i64, next: *Node }

extern fn sqrt(x: f64) -> f64;

fn length(v: *Vec2) -> f64 {
  return sqrt(v->x * v->x + v->y * v->y);
}

fn push_front(head: *Node, value: i64) -> *Node {
  var n = alloc(sizeof(Node)) as *Node;
  n->value = value;
  n->next = head;
  return n;
}

fn main() -> i32 {
  var p = (Vec2){ .x = 3.0, .y = 4.0 };
  println(length(&p));                  // 5
  var list: *Node = null;
  for (var i = 1; i <= 3; i += 1) { list = push_front(list, i); }
  while (list != null) {
    var next = list->next;
    free(list);
    list = next;
  }
  return 0;
}
```

- **Types**: `i8` .. `i64`, `u8` .. `u64`, `f32`, `f64`, `bool`, structs, pointers `*T` and
  arrays `[]T`. Integer literals are `i64` unless the context wants another size; `'a'` is a `u8`,
  `0xff` a hexadecimal constant, `"text"` a `*u8` to NUL-terminated bytes, as in C.
- **Structs** are values: `(Vec2){ .x = 1.0 }` (C99's compound literal; fields left out are zero),
  copied when assigned, passed or returned. `sizeof(T)` is C's.
- **Pointers**: `&place` (a variable, a field, an element), `*p`, `p[i]`, `p->f`, `p + n`,
  `p - n`, `null`, and casts between pointers and integers (`e as T`). Nothing about them is
  checked.
- **Memory**: `alloc(n)` is `n` zeroed bytes from C's allocator and `free(p)` gives them back;
  arrays (`[1, 2, 3]`, `push`, `pop`, `len`) are A0's, with their bounds checked, and freed with
  `free` too. Freeing an array twice stops the program, with a stack trace.
- **C functions**: `extern fn name(params) -> T;` calls a C function, structs by value included,
  and `f32` as C's `float`. Programs run interpreted, in the JIT and natively, calling C the same
  way in all of them (`docs/ir.md`, "C structs").
- **Statements**: `var x: T = e;` (the type may be left out), `if (c) { } else { }`, `while (c)
  { }`, `for (init; cond; step) { }`, `break`, `continue`, `return`, and assignments with `=`,
  `+=`, `-=`, `*=`, `/=`, `%=`, `&=`, `|=`, `^=`, `<<=`, `>>=` (an operator-assignment evaluates its
  place twice). `print(a, b, ...)` and `println(...)` print numbers, booleans and C strings.
- `main` returns an `i32`, the exit status.

## The C library

`runtime/` is the C Tin programs link with: `tin.c` (a function to print C strings, and the table
of functions the interpreter and the JIT can call), `abi.c` (functions of structs by value, in the
shapes the C ABIs treat differently, which `tests/programs/c_structs.tin` calls) and
[cJSON](https://github.com/DaveGamble/cJSON) 1.7.18, vendored in `runtime/cjson` under its MIT
license. `tests/programs/json.tin` parses and builds JSON with it, reading `cJSON` structs through
pointers, declared with C's layout.

## Files

| File | What it is |
|---|---|
| `tin.a0g` | The grammar (with `@def`/`@scope` for the language server) |
| `src/lib.rs` | The compiler: one walk over the syntax tree, building HIR by name |
| `src/main.rs` | The `tin` command (`run`, `build`, `ir`, `hir`, `lsp`, `dap`, `repl`) |
| `runtime/` | The C library: Tin's runtime, the ABI test functions, cJSON |
| `tests/programs` | Programs with expected output, run in eight modes |
| `tests/diagnostics` | Snapshots of compile and runtime errors |
| `tests/lsp.rs`, `tests/dap.rs`, `tests/repl.rs` | The language server, the debugger and the REPL |

## Deliberate differences from C

No preprocessor, no implicit conversions between number types (write `as`), conditions are `bool`,
declarations read `name: type`, functions and types may be used before they are declared, and
there are no variadic functions (C's `printf` among them: Tin has `print`). Integer division by
zero stops the program instead of being undefined.
