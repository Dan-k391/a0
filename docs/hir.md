# A0 high-level IR (HIR)

Status: implemented in `crates/a0-hir`. Keep this document in sync with the code.

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
| `S`, `S<T, ...>` | reference | a declared struct: named, mutable fields; possibly generic |
| `E`, `E<T, ...>` | reference | a declared enum: variants with positional payloads; possibly generic |
| `fn(T, ...) -> R` | reference | a closure |
| `T?` | reference | a reference type `T`, or null |
| `never` | (internal) | the type of `return`, `break`, `exit`, ...; fits any type |

Reference types are garbage collected. They are **never null** unless their type is nullable: `T?`
holds a `T` or `null`. Value types are copied.

**Nullability.** `null<T>` has type `T?`. A `T` fits wherever a `T?` is expected. Fields, elements and
calls need a non-null value: `e!` turns a `T?` into a `T` and panics with `null reference` if it is
null, and `x == null<T>` tests for null. Because the types guarantee it, generated code checks for
null only at `!`. A global of reference type starts as null, so its type must be nullable.

**Generics.** Structs, enums and functions can take type parameters (`struct Pair<A, B>`,
`fn first<T>(xs: array<T>) -> Option<T>`). Inside a generic item a type parameter is opaque: values
of it can be stored, passed, returned, compared with `==` and turned into `any`, but not used as
numbers or strings. Type arguments are inferred at calls and constructions (`first(xs)`,
`new Pair { ... }`, `Option::None` where an `Option<int>` is expected) or written out
(`first::<int>(xs)`, `new Pair<int, string> { ... }`, `Option<int>::None`). A0 monomorphizes: each
generic item is copied for every combination of type arguments the program uses, before lowering,
so frontends never substitute types themselves.

**Inference.** Types a frontend does not know yet are inference variables: `HirBuilder::fresh_type`
for an unannotated variable, the element type of `[]`, the type arguments of generic calls, the
result of calling a closure of unknown type. Later uses fix them by unification. A node whose rule
needs a type that is still open (a field of an object whose type is unknown) is typed again when
`finish` runs; a type nothing determined is reported as "cannot infer". Type errors are reported at
the sub-expression they are about, in source terms (`mismatched types: expected `int`, found
`string``), so languages can show them to their users unchanged.

## Module

```
module "example";

struct Point { x: float, y: float }
enum Shape { Circle(float), Rect(float, float), Empty }
struct Cell<T> { head: T, tail: Cell<T>? }
global count: int;
extern fn putchar(int) -> int;

fn length<T>(c: Cell<T>?) -> int {
  if c == null<Cell<T>> { 0 } else { 1 + length(c!.tail) }
}

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

- `global` variables start at the zero value of their type (`0`, `0.0`, `false`, `()`, and `()` for
  `any`); one of reference type must be nullable and starts as `null`.
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
| `()`, `true`, `42`, `1.5`, `"text"`, `null<T>` | literals (`null<T>` has type `T?`) |
| `x`, `f`, `f::<T>` | a local, a global, or a function used as a closure value |
| `let x: T = e;`, `let x = e;` | declare a local (the type defaults to `e`'s) |
| `x = e`, `o.f = e`, `a[i] = e` | assignment (type `unit`) |
| `a + b`, `-a`, `a < b`, `a == b`, `a && b`, `!a` | operators, see below |
| `f(a, b)`, `f::<T>(a)`, `e(a, b)` | call a function (with explicit type arguments), extern or closure |
| `e!` | the value of a `T?`; panics with `null reference` if it is null |
| `fn(x: int) -> int { x + 1 }` | a closure; captures outer locals by reference |
| `new S { f: e, ... }`, `new S<T> { ... }`, `o.f` | struct construction (every field) and field access |
| `E::V(a, b)`, `E<T>::V`, `match e { V(x, y) => ..., _ => ... }` | enum construction and matching |
| `[a, b]`, `[]`, `new array<T>`, `a[i]` | arrays (index out of range panics) |
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
| `str(x)` | format an `int`, `float`, `bool` (or `string`) as a string (floats: shortest round-trip, `1` not `1.0`) |
| `print(s)`, `eprint(s)` | write a string to stdout / stderr (no newline added) |
| `exit(code)`, `panic(message)` | stop the program (`panic` prints `a0: panic: message`, exit 101) |
| `clock()` | processor time in seconds, as a `float` |

## Conversions and dynamic values

`e as T`: `int` to `float` and back (truncating, saturating, NaN to 0); any value to `any`; `any` to
any type it holds (panics with `expected T` otherwise; `()` becomes null for `T?`); `T` to `T?`.

`e is T` tests the type held by an `any`: `unit`, `bool`, `int`, `float`, `string`, a struct, or an
enum (true for every variant).

`==` on two `any` values: floats by IEEE equality, ints by value, strings by content, booleans and
`()` by value, other references by identity, different types are unequal.

`any` is NaN-boxed into 64 bits: floats are stored as themselves (NaNs canonicalized), `()` and the
booleans are special NaN patterns, references are tagged pointers, `int`s that fit in 48 bits are
stored inline, and larger `int`s are boxed on the heap.
Boxing a null reference yields `()`.

## Closures

A `fn(...) { ... }` expression captures the outer locals it uses **by reference**: assignments are
seen by the closure and by the enclosing function, and every execution of a `let` creates a fresh
variable (a closure created in a loop body sees that iteration's variables). A top-level function
used as a value becomes a closure with no captures.

## Building HIR from a frontend

A frontend walks its syntax tree once and calls `HirBuilder`. Two styles mix freely:

- **By handle**, for a language that resolves names itself (Lox, with jlox's rules): `new_local`,
  `local`, `call(FuncId, ...)`, `new_struct(StructId, ...)`.
- **By name** (`frontend.rs`), for everything else (Minnow): `begin_body`/`end_body` put a body's
  parameters (and a generic function's type parameters) in scope; `push_scope`/`pop_scope` follow
  blocks; `let_named` and `declare_local` declare locals (shadowing allowed; `is_declared_in_scope`
  for languages that forbid it); `name`, `call_named`, `assign_named` resolve locals, globals,
  functions and builtins; `named_type` resolves type names; `new_struct_named` takes fields in any
  order, keeps their evaluation order and reports unknown, repeated and missing fields;
  `variant_named` and `field_named` look up by name.

Errors are reported at the span set with `set_span`, in user-facing wording; `report` adds a
language's own errors and `error_expr` stands in for an expression that could not be built (its type
fits everything, so it causes no further errors). `set_type_syntax(TypeTag::Array, "[{}]")` makes
diagnostics write types in the language's syntax. Item names are free: one that is a HIR keyword
(`map`) or not an identifier is written in backquotes in the text format.

## Text format

The examples above are the text format. `print(parse(text))` is canonical and
`print(parse(print(m))) == print(m)`. Names are resolved lexically; a block opens a scope. Comments
are `//` to the end of the line. The parser is written with A0's own grammar layer
(`crates/a0-hir/src/hir.a0g`).

## Lowering

HIR is lowered to the core IR plus calls into `a0rt`, after monomorphization (`mono.rs`) has
replaced generic items with one copy per combination of type arguments:

- Value types map to `i1`, `i64`, `f64`; `any` to `i64`; references to `ptr`; `unit` disappears.
- **Precise GC without stack maps.** Every reference or `any` that lives across something that can
  allocate is kept in a slot of the function's frame on the runtime's shadow stack
  (`a0_gc_frame_push`/`pop`), stored in the `any` encoding. Locals of these types are IR variables
  whose every assignment is also written to their slot (the collector never moves objects, so the
  variable stays valid); a local declared in a block has its slot only while the block runs.
  Temporaries are spilled only when a later operand or the operation itself may allocate, and reads
  of locals that are never reassigned are not spilled. A slot is cleared when its scope ends or is
  left by `return`, `break` or `continue`, so the stack above its top is always zero and pushing a
  frame clears nothing (D30). Globals of these types are registered as roots. Heap objects are
  traced by layout.
- **Objects.** Every struct, enum variant and closure environment gets a type descriptor registered
  at startup with a layout (one byte per word: raw, reference or `any`). Field access is an inline
  load or store; references are non-null by type, so only `!` checks for null.
- **Closures.** Each closure is a core IR function taking its environment first. Captured locals
  live in heap boxes, shared by the defining function and every closure that captures them.
- **`any`.** Type tests of objects are inline: the object's type sits in the collector's header and
  its family in the type descriptor, at offsets `a0rt.h` fixes (D28). Equality of two `any` words is
  inline unless both are references (strings and boxed integers compare by value in the runtime).
- Strings, arrays, maps, formatting and boxing of large integers are runtime calls.
- Integer division by zero, and dividing the minimum `int` by -1, trap like the core IR's `sdiv`
  (`a0: trap: ...`, exit 101).
- `a0 lower prog.hir` prints the core IR a module lowers to; `a0 run`/`a0 build` accept `.hir` files.

## Optimization

Before lowering, the HIR optimizer (`opt.rs`) runs on the monomorphized module. It is on by default,
needs nothing from frontends, and `A0_OPT=0` (or `LowerOptions { optimize: false }`) turns it off.
It complements Cranelift, which optimizes within each function:

- **Inlining** of small functions (up to 80 nodes, no closures inside, not calling themselves), three
  levels deep. A `return` in the middle of an inlined function becomes the value of an `if`
  (`if c { return a } rest` is `if c { a } else { rest }`); functions with other `return`s stay calls.
- **`any` folding.** `(x as any) is T` and `(x as any) as T` are decided from `x`'s type, also through
  a local: `let v = <... x as any>` (never reassigned) becomes `let raw = <... x>; let v = raw as
  any`, so tests of `v` fold and `v as T` reads `raw`.
- **Guarded casts.** In `if v is T { ... v as T ... }`, with `v` a local that is never assigned, the
  cast does not check again.
- **Simplification**: constant `if`, `&&`, `||` and `!`; casts to the same type; copies of locals and
  of constants; unused locals and statements without effects; nested blocks; code after something
  that never finishes.

Only expressions without effects are removed or duplicated, so traps, panics, output and evaluation
order stay as written. Every test program runs optimized and unoptimized, interpreted, native and
under GC stress, and all must agree.

## Not yet

Bounds on type parameters, methods, interfaces, exceptions, iteration protocols, float map keys, and
memory modes other than `gc`.
