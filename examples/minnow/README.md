# Minnow

A small statically typed language built on A0: `int`, `float`, `bool`, `string`, arrays, structs,
enums, closures, generics and nullable references, type checked before it runs, compiled to A0's
high-level IR, and run in the interpreter or as a native executable. It is the example of a typed
language on the HIR; Lox is the dynamic one.

```
struct Point { x: float, y: float }
enum Shape { Circle(float), Rect(float, float), Dot }

fn largest<T>(xs: [T], size: fn(T) -> float) -> T? {
  let best: T? = null;
  for x in xs {
    if best == null || size(x) > size(best!) { best = x; }
  }
  return best;
}

fn area(s: Shape) -> float {
  return match s { Circle(r) => 3.0 * r * r, Rect(w, h) => w * h, Dot => 0.0 };
}

fn main() {
  let shapes = [Shape::Circle(1.0), Shape::Rect(2.0, 3.0), Shape::Dot];
  let big = largest(shapes, area);
  if big != null { print(area(big!)); }   // 6
}
```

A program can have several files. `import text.util;` makes the public (`pub`) items of
`text/util.mn`, found next to the program's main file, available as `util.twice(21)`,
`util.Point` and `util.Shape::Dot`; `import text.util as u;` names the module `u`. The standard
library's modules (`docs/std.md`) are imported the same way: `import std.fs;`, then
`match fs.read(path) { Ok(text) => ..., Err(e) => ... }`.

```
minnow run prog.mn               # interpret
minnow build prog.mn -o prog     # native executable
minnow hir prog.mn               # print the HIR
minnow ir prog.mn                # print the core IR
```

| File | Role |
|---|---|
| `minnow.a0g` | the grammar (89 lines) |
| `src/lib.rs` | the compiler: one walk over the syntax tree, building HIR by name (583 lines) |
| `src/main.rs` | the `minnow` command, an `a0-driver` language |
| `tests/programs` | programs with expected output, run interpreted, native and under GC stress |

Minnow has no type checker or name resolver of its own: the HIR builder resolves names, infers and
checks types, monomorphizes generics and reports errors in Minnow's terms. Before the builder could
do that, Minnow was 1513 lines without enums, generics or nullable types.

## The language

- **Items**: `struct Name<T> { field: type, ... }`, `enum Name<T> { Variant(type, ...), ... }` and
  `fn name<T>(param: type, ...) -> type { ... }`, in any order (type parameters are optional). The
  program starts at `fn main()`, which may return an `int` exit status.
- **Types**: `int` (64-bit, wrapping), `float`, `bool`, `string`, `[T]` arrays, `fn(T, ...) -> R`
  functions (no `->` means no result), struct and enum names with type arguments (`Pair<int, string>`,
  inferred when left out), and `T?` for a reference that may be null. Values of struct, enum, array and
  function type are references; `==` compares them by identity and everything else by value.
- **Null**: a reference is never null unless its type is `T?`. `null` is a `T?`, and a `T` can be
  used where a `T?` is expected. `if let x = e { ... }` runs its block with `x` holding `e`'s value
  when that is not null (`else` covers null), `while let x = e { ... }` repeats while it is not
  null, and `x!` gets the `T` back directly (stopping the program if `x` is null). Fields, elements
  and calls need a non-null value.
- **Statements**: `let x = e;` or `let x: T = e;`, assignment to variables, fields and elements,
  `if`/`else if`/`else`, `if let`, `while`, `while let`, `for x in array`, `match`, `return`,
  `break`, `continue`, blocks.
- **Expressions**: arithmetic `+ - * / %` (`+` also joins strings), comparisons, `&&`, `||`, `!`,
  `e as T` between `int` and `float`, calls, `a[i]`, `s.f`, `x!`, `[a, b]`, `new S { f: e, ... }`,
  `E::V(a)`, `match e { V(x) => e, _ => e }` (arms may be blocks), and closures
  `fn(x: int) -> int { return x + 1; }`, which capture variables by reference.
- **Builtins**: `print(x)` for numbers, booleans and strings (with a newline), `str(x)`, `len(a)`
  for arrays and strings, `push(a, x)`, `pop(a)`.
- **Inference**: a variable's type comes from its initializer; the element type of `[]`, the type
  arguments of generic calls and the type of `null` come from how they are used.
- Indexing out of bounds, `pop` on an empty array, `!` on null and integer division by zero stop the
  program with exit status 101.

Struct literals need `new` (`new Point { ... }`) so that `if p { ... }` is never ambiguous.
