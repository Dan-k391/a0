# A0 high-level IR (HIR)

Status: design, being implemented in `crates/a0-hir`. Keep this document in sync with the code.

The core IR (`docs/ir.md`) is a low-level layer: scalars, pointers, explicit memory. Building a language
directly on it means writing your own values, strings, collections, closures and garbage collector, as
the first Lox did. The HIR sits above it and provides those things built in. It is lowered to the core
IR plus calls into the default runtime (`a0rt`), so it runs in the interpreter and natively unchanged.

## Goals

- A frontend emits a **typed expression tree**, close to what it already has, and gets strings, arrays,
  maps, structs, enums, closures and garbage collection for free.
- Statically typed languages use precise types; dynamically typed languages use `any`, a value that
  can hold anything and is tested and unpacked at run time.
- No frontend ever sees a GC root, a frame or a capture list: lowering does that.
- A text format, so HIR can be written by hand (a language's runtime support can be written in HIR),
  printed for debugging and golden-tested.
- C-bindable builder API (handles), diagnostics instead of panics, same as the core IR.

## Types

| Type | Kind | Notes |
|---|---|---|
| `unit` | value | the empty value, written `()` |
| `bool` | value | |
| `int` | value | 64-bit signed, wrapping arithmetic |
| `float` | value | IEEE double |
| `any` | value | holds any value; see "Dynamic values" |
| `string` | reference | immutable bytes (UTF-8 by convention) |
| `array<T>` | reference | growable, mutable |
| `map<K, V>` | reference | hash map; `K` is `int`, `bool` or `string` |
| `S` | reference | a declared struct: named, mutable fields |
| `E` | reference | a declared enum: variants with positional payloads |
| `fn(T, ...) -> R` | reference | a closure |
| `never` | (internal) | the type of `return`, `break`, `exit`, ...; fits any type |

Reference types are garbage collected and nullable (`null<T>`). Using a null reference panics with
`null reference`. Value types are copied.

## Module

```
module "example";

struct Point { x: float, y: float }
enum Shape { Circle(float), Rect(float, float), Empty }
global count: int;
extern fn putchar(int) -> int;

fn area(s: Shape) -> float {
  match s {
    Circle(r) => 3.14159 * r * r,
    Rect(w, h) => w * h,
    Empty => 0.0,
  }
}

fn main() -> int {
  let p = new Point { x: 1.0, y: 2.0 };
  let shapes = [Shape::Circle(1.0), Shape::Rect(p.x, p.y)];
  let total = 0.0;
  let i = 0;
  while i < len(shapes) {
    total = total + area(shapes[i]);
    i = i + 1;
  }
  print(str(total) + "\n");
  0
}
```

- `global` variables start at the zero value of their type (`0`, `0.0`, `false`, `()`, `null`, and
  `()` for `any`).
- `extern fn` declares a C function; its parameter and result types must be `int`, `float`, `bool`
  or `unit` (the interpreter's FFI rules apply).
- The entry point is `fn main() -> int` or `fn main()`; its result is the exit status.
- Memory management is a per-module choice; `gc` (precise tracing collection) is the only mode so
  far and the default.

## Expressions

Everything is an expression with a type. Blocks `{ a; b; c }` evaluate to their last expression when
it has no trailing `;`, otherwise to `()`.

| Form | Meaning |
|---|---|
| `()`, `true`, `42`, `1.5`, `"text"`, `null<T>` | literals |
| `x`, `f` | a local, a global, or a function used as a closure value |
| `let x: T = e;`, `let x = e;` | declare a local (the type defaults to `e`'s) |
| `x = e`, `o.f = e`, `a[i] = e` | assignment (type `unit`) |
| `a + b`, `-a`, `a < b`, `a == b`, `a && b`, `!a` | operators, see below |
| `f(a, b)`, `e(a, b)` | call a function, extern or closure |
| `fn(x: int) -> int { x + 1 }` | a closure; captures outer locals by reference |
| `new S { f: e, ... }`, `o.f` | struct construction (every field) and field access |
| `E::V(a, b)`, `match e { V(x, y) => ..., _ => ... }` | enum construction and matching |
| `[a, b]`, `new array<T>`, `a[i]` | arrays (index out of range panics) |
| `new map<K, V>`, `m[k]` | maps (a missing key panics) |
| `e as T`, `e is T` | conversions and dynamic type tests |
| `if c { } else { }`, `while c { }`, `loop { }` | control flow; `if` with `else` is a value |
| `break`, `continue`, `return e` | type `never` |

**Operators.** `+ - * / %` on two `int`s (wrapping; `/` and `%` by zero panic) or two `float`s (no
`%`); `+` on two `string`s concatenates. `< <= > >=` on `int`, `float` or `string` (byte order).
`==` and `!=` on any two values of the same type: by value for value types and strings, by identity
for other references; on `any` see below. `&&`, `||` short-circuit; `!` negates a `bool`.

**Builtins** (called like functions):

| Builtin | Meaning |
|---|---|
| `len(x)` | length of a string (bytes), array or map |
| `push(a, v)`, `pop(a)` | append to / remove the last element of an array (`pop` on empty panics) |
| `has(m, k)`, `remove(m, k)`, `keys(m)` | map membership, removal, keys as an array (deterministic order) |
| `substr(s, start, count)`, `byte_at(s, i)` | string slicing and bytes |
| `str(x)` | format an `int`, `float` or `bool` as a string (floats: shortest round-trip, `1` not `1.0`) |
| `print(s)`, `eprint(s)` | write a string to stdout / stderr (no newline added) |
| `exit(code)`, `panic(message)` | stop the program (`panic` prints `a0: panic: message`, exit 101) |
| `clock()` | processor time in seconds, as a `float` |

## Conversions and dynamic values

`e as T`: `int` to `float` and back (truncating, saturating, NaN to 0); any value to `any`; `any` to
any type it holds (panics with `expected T` otherwise).

`e is T` tests the type held by an `any`: `unit`, `bool`, `int`, `float`, `string`, a struct, or an
enum (true for every variant).

`==` on two `any` values: floats by IEEE equality, ints by value, strings by content, booleans and
`()` by value, other references by identity, different types are unequal.

`any` is NaN-boxed into 64 bits: floats are stored as themselves (NaNs canonicalized), `()` and the
booleans are special NaN patterns, references are tagged pointers, and `int`s are boxed on the heap.
Boxing a null reference yields `()`.

## Closures

A `fn(...) { ... }` expression captures the outer locals it uses **by reference**: assignments are
seen by the closure and by the enclosing function, and every execution of a `let` creates a fresh
variable (a closure created in a loop body sees that iteration's variables). A top-level function
used as a value becomes a closure with no captures.

## Text format

The examples above are the text format. `print(parse(text))` is canonical and
`print(parse(print(m))) == print(m)`. Names are resolved lexically; a block opens a scope. Comments
are `//` to the end of the line. The parser is written with A0's own grammar layer
(`crates/a0-hir/src/hir.a0g`).

## Lowering

HIR is lowered to the core IR plus calls into `a0rt`:

- Value types map to `i1`, `i64`, `f64`; `any` to `i64`; references to `ptr`; `unit` disappears.
- **Precise GC without stack maps.** Every reference or `any` that lives across something that can
  allocate is kept in a slot of the function's frame on the runtime's shadow stack
  (`a0_gc_frame_push`/`pop`), stored in the `any` encoding. Locals of these types always live in
  frame slots; temporaries are spilled only when a later operand or the operation itself may
  allocate. Globals of these types are registered as roots. Heap objects are traced by layout.
- **Objects.** Every struct, enum variant and closure environment gets a type descriptor registered
  at startup with a layout (one byte per word: raw, reference or `any`). Field access is an inline
  load or store after a null check.
- **Closures.** Each closure is a core IR function taking its environment first. Captured locals
  live in heap boxes, shared by the defining function and every closure that captures them.
- Strings, arrays, maps, formatting, boxing and dynamic equality are runtime calls.

## Not yet

User-defined generics, methods, interfaces, exceptions, iteration protocols, float map keys, and
memory modes other than `gc`.
