# Minnow

A small statically typed language built on A0: `int`, `float`, `bool`, `string`, arrays, structs and
closures, type checked before it runs, compiled to A0's high-level IR, and run in the interpreter or
as a native executable. It is the example of a typed language on the HIR; Lox is the dynamic one.

```
struct Point { x: float, y: float }

fn scale(p: Point, k: float) -> Point {
  return new Point { x: p.x * k, y: p.y * k };
}

fn main() {
  let points: [Point] = [];
  for i in [1, 2, 3] {
    push(points, scale(new Point { x: 1.0, y: 2.0 }, i as float));
  }
  let total = 0.0;
  let add = fn(p: Point) { total = total + p.x + p.y; };
  for p in points { add(p); }
  print(total);   // 18
}
```

```
minnow run prog.mn               # interpret
minnow build prog.mn -o prog     # native executable
minnow hir prog.mn               # print the HIR
minnow ir prog.mn                # print the core IR
```

| File | Role |
|---|---|
| `minnow.a0g` | the grammar |
| `src/check.rs` | name resolution and type checking on `a0-sema`'s `Scopes` and `Types` |
| `src/emit.rs` | HIR generation |
| `src/main.rs` | the `minnow` command, an `a0-driver` language |
| `tests/programs` | programs with expected output, run interpreted, native and under GC stress |

## The language

- **Items**: `struct Name { field: type, ... }` and `fn name(param: type, ...) -> type { ... }`, in
  any order. The program starts at `fn main()`, which may return an `int` exit status.
- **Types**: `int` (64-bit, wrapping), `float`, `bool`, `string`, `[T]` arrays, `fn(T, ...) -> R`
  functions (no `->` means no result), and struct names. Values of struct, array and function type
  are references; `==` compares them by identity and everything else by value.
- **Statements**: `let x = e;` or `let x: T = e;`, assignment to variables, fields and elements,
  `if`/`else if`/`else`, `while`, `for x in array`, `return`, `break`, `continue`, blocks.
- **Expressions**: arithmetic `+ - * / %` (`+` also joins strings), comparisons, `&&`, `||`, `!`,
  `e as T` between `int` and `float`, calls, `a[i]`, `s.f`, `[a, b]`, `new S { f: e, ... }` and
  closures `fn(x: int) -> int { return x + 1; }`, which capture variables by reference.
- **Builtins**: `print(x)` for numbers, booleans and strings (with a newline), `str(x)`, `len(a)`
  for arrays and strings, `push(a, x)`, `pop(a)`.
- **Inference**: a variable's type comes from its initializer. The element type of `[]` comes from
  how the array is used later (`let xs = []; push(xs, 1);` makes `xs` a `[int]`), through
  unification in `a0_sema::Types`.
- Indexing out of bounds, `pop` on an empty array and integer division by zero stop the program
  with exit status 101.

Struct literals need `new` (`new Point { ... }`) so that `if p { ... }` is never ambiguous.
