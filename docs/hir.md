# A0 high-level IR (HIR)

The HIR is the form a language's compiler (its *frontend*) gives A0 a program in: a typed tree of
expressions, with strings, arrays, maps, structs, enums, closures and garbage collection built in.
A frontend builds it with `HirBuilder`, or writes it as text. A0 then checks it, optimizes it and
lowers it, that is, translates it into the [core IR](ir.md), the low-level layer of scalars,
pointers and explicit memory. So a language built on the HIR runs in the interpreter, the JIT
(compiled into memory) and native executables without writing a runtime of its own.

Here is a small HIR program, `point.hir`:

```
module "point" version 1;

struct Point { x: int, y: int }

fn main() -> int {
  let p = new Point { x: 3, y: 4 };
  print("x + y = " + str(p.x + p.y) + "\n");
  0
}
```

```
$ a0 run point.hir
x + y = 7
```

`a0 lower point.hir` prints the core IR it becomes. Here is an excerpt of `main`:

- `a0_gc_frame_push` gives the function three slots on the garbage collector's *shadow stack*,
  where it keeps the references that must survive an allocation (see "Lowering").
- `a0_gc_new` allocates the `Point`.
- `p.x` and `p.y` are loads at offsets 0 and 8.
- `str` becomes a call to the runtime, `a0_string_from_int`.

```
func @hir.main() -> i64 {
  var $p: ptr
  %0: i64 = iconst 3
  %1: ptr = call @a0_gc_frame_push(%0)
  ...
  %3: ptr = addr @hir.type.Point
  %4: ptr = load %3
  %5: ptr = call @a0_gc_new(%4)
  ...
  %32: ptr = get $p
  %33: i64 = iconst 0
  %34: ptr = ptradd %32, %33
  %35: i64 = load %34
  %36: ptr = get $p
  %37: i64 = iconst 8
  %38: ptr = ptradd %36, %37
  %39: i64 = load %38
  %40: i64 = iadd %35, %39
  %41: ptr = call @a0_string_from_int(%40)
  ...
```

The full output also declares the runtime's functions and has a C-level `@main`. That `@main`
registers the type descriptors and string constants, then calls `@hir.main`.

Building a language directly on the core IR means writing your own values, strings, collections,
closures and garbage collector, as the first Lox did. The HIR provides those things, and is lowered
to the core IR plus calls into the default runtime (`a0rt`), so it runs the same in every backend.

## Goals

- A frontend emits a **typed expression tree**, close to what it already has, and gets strings,
  arrays, maps, structs, enums, closures and garbage collection for free.
- Statically typed languages use precise types. Dynamically typed languages use `any`, a value that
  can hold anything and is tested and unpacked at run time.
- No frontend ever sees a GC root, a frame or a capture list: lowering does that.
- A text format, so HIR can be written by hand (a language's runtime support can be written in
  HIR), printed for debugging and compared against saved output in tests.
- A builder API that C can call (handles), and diagnostics instead of panics, as in the core IR.

## Types

| Type | Kind | Notes |
|---|---|---|
| `unit` | value | the empty value, written `()` |
| `bool` | value | |
| `int` | value | 64-bit signed, wrapping arithmetic |
| `float` | value | IEEE double |
| `i8` `i16` `i32` `u8` `u16` `u32` `u64` | value | sized integers, wrapping at their width; `i64` is `int` |
| `f32` | value | IEEE single; `f64` is `float` |
| `any` | value | holds any value; see "Dynamic values" |
| `string` | reference | immutable bytes (UTF-8 by convention) |
| `array<T>` | reference | growable, mutable; `array<bool>` keeps a byte per element, the others a 64-bit word |
| `map<K, V>` | reference | hash map; `K` is `int`, `bool` or `string` |
| `object` | reference | a dynamic object: `any` fields added by name at run time; see "Dynamic objects" |
| `S`, `S<T, ...>` | reference | a declared struct: named, mutable fields; possibly generic |
| `S` (`inline struct`) | value | an inline struct: its fields live where it is used; see "Inline structs" |
| `ptr<T>` | value | a raw address of `T`s (plain data), unchecked and untraced; see "Pointers" |
| `E`, `E<T, ...>` | reference | a declared enum: variants with positional payloads; possibly generic |
| `fn(T, ...) -> R` | reference | a closure |
| `I` | reference | a declared interface: any struct or enum that implements it; see "Methods and interfaces" |
| `T?` | reference | a reference type `T`, or null |
| `never` | (internal) | the type of `return`, `break`, `exit`, ...; fits any type |

Reference types are garbage collected. They are **never null** unless their type is nullable: `T?`
holds a `T` or `null`. Value types are copied.

### Nullability

- `null<T>` has type `T?`. A `T` fits wherever a `T?` is expected.
- Fields, elements and calls need a non-null value.
- `if let x = e { a } else { b }` narrows. When `e` (a `T?`) is not null, `x` is a `T` holding it
  in `a`. Otherwise `b` runs.
- `e!` turns a `T?` into a `T` directly. It fails if `e` is null, with
  ``unwrapped a `null` value with `!` `` in the language's terms.
- `x == null<T>` tests for null.

Types do not change after a test: there is no flow typing (D34). Because the types guarantee it,
generated code checks for null only at `if let` and `!`. A global of reference type starts as
null, so its type must be nullable.

### Generics

Structs, enums and functions can take type parameters (`struct Pair<A, B>`,
`fn first<T>(xs: array<T>) -> Option<T>`). Inside a generic item a type parameter is opaque. Values
of it can be stored, passed, returned, compared with `==` and turned into `any`, but not used as
numbers or strings.

Type arguments are inferred at calls and constructions, or written out:

| Inferred | Written out |
|---|---|
| `first(xs)` | `first::<int>(xs)` |
| `new Pair { ... }` | `new Pair<int, string> { ... }` |
| `Option::None`, where an `Option<int>` is expected | `Option<int>::None` |

A0 monomorphizes: before lowering, it copies each generic item for every combination of type
arguments the program uses. So frontends never substitute types themselves.

### Methods and interfaces

```
interface Shape { fn area(self) -> float; fn bigger(self, Self) -> bool; }
impl Shape for Circle;
impl Circle {
  fn area(self: Circle) -> float { 3.14 * self.r * self.r }
  fn bigger(self: Circle, other: Circle) -> bool { self.r > other.r }
}
impl<T> Cell<T> {
  fn get(self: Cell<T>) -> T { self.value }
}
fn total(shapes: array<Shape>) -> float { ... shapes[i].area() ... }
```

A **method** (D52) belongs to one of these:

- a struct;
- an enum (for every instance of a generic one);
- a value type (`int`, `float`, `bool`, `string`).

Its first parameter, `self`, is the value it is called on. `x.m(args)` finds the method by `x`'s
type.

An **interface** lists methods by their parameters after `self`, where `Self` stands for the
implementing type. A struct or enum implements an interface by declaring so and having those
methods, with those signatures.

A value of an interface type is a reference to an object of a type that implements it. A struct or
enum converts to the interface where one is expected, or with `as`. `x.m(args)` on an interface
value finds the method at run time. Methods that use `Self` cannot be called that way (E2022).

A type parameter can be bounded by interfaces (`fn largest<T: Ord>(xs: array<T>) -> T`). Its values
then have the interfaces' methods, with `Self` being the parameter. Every type argument must
implement them; value types are checked too, statically. `x is I` and `x as I` test and convert an
`any`.

Builder:

- `declare_interface`, and `add_interface_method` (with `interface_self` for `Self`);
- `interface_type`;
- `declare_method`: the owner's type parameters come first, and `method_self_type` is `self`'s
  type;
- `implement` and `add_bound`;
- `method_call`, which also calls a closure held in a field of that name.

A method is the function `Owner.name` in the text (`` `Circle.area`(c) ``).

### Inference

Types a frontend does not know yet are *inference variables*. They come from:

- `HirBuilder::fresh_type`, for an unannotated variable;
- the element type of `[]`;
- the type arguments of generic calls;
- the result of calling a closure of unknown type.

Later uses fix them by unification, which makes two types equal by filling in the unknown parts.
Some nodes need a type that is still open when they are built, such as a field of an object whose
type is unknown. Such a node is typed again when `finish` runs. A type nothing determined is
reported as "cannot infer".

Type errors are reported at the sub-expression they are about, in source terms
(``mismatched types: expected `int`, found `string` ``), so languages can show them to their users
unchanged.

## Module

```
module "example" version 1;

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

`version 1` names the version of the text format (`docs/compatibility.md`). It is optional and
means 1 when absent; the printer always writes it. A0 1.x reads version 1 and refuses a later one
with `E0004`.

**Globals.** `global` variables start at the zero value of their type: `0`, `0.0`, `false`, `()`,
and `()` for `any`. One of reference type must be nullable, and starts as `null`.

**C functions.** `extern fn` declares a C function (D47). Its parameters are passed as C sees them:

| HIR parameter | C sees |
|---|---|
| `int` | `int64_t` |
| `float` | `double` |
| `bool` | an `int64_t` 0 or 1 |
| sized integers | `int8_t` .. `uint64_t` |
| `f32` | `float` |
| pointers | pointers |
| inline structs of numbers, `bool`, pointers and such structs | C structs by value, laid out as C lays them out (`docs/ir.md`, "C structs") |
| `string` | a NUL-terminated `const char *` to the string's own bytes, so a string with a NUL byte looks shorter |
| arrays of `int`, `float` or `bool` | a pointer to the array's elements (`int64_t *`, `double *` or `bool *`), which the function may change but not add to |
| `string?`, and `T?` of those arrays | the same, or `NULL` for null |

A C function returns one of these:

- a number;
- `bool` (C's `bool`);
- a pointer;
- `unit`;
- such an inline struct;
- `string`, which is copied. Write `string?` if it may return `NULL`; for `string`, `NULL` panics.

The interpreter looks for the function in the runtime and in the tables a language registers, and
otherwise in the process (the C library, loaded libraries). So `extern fn getenv(string) -> string?;`
works in both backends.

**Runtime functions.** `extern "a0" fn` declares a function of the runtime, or of a language's
runtime library, that takes and returns A0's own values, of any type. Strings are `A0String *`,
arrays `A0Array *`, structs and closures pointers to their objects, and `any` its 64-bit word (see
`a0rt.h`). Such a function may allocate: its reference arguments stay rooted during the call. The
standard library is built on these (`docs/std.md`). Several modules may declare the same C
function, with the same signature.

**Threads.** A program that declares the runtime's `a0_thread_spawn` (as `std.thread` does) starts
threads (D75, D76). Lowering then changes three things:

- it keeps the exception in flight in the current thread's slots instead of globals;
- it does not use per-site field caches;
- with the collector, it polls for a collection at every loop head and function entry (a load and a
  branch), so that collections can stop every thread.

Programs without threads are lowered as before.

**Entry point.** The entry point is `fn main() -> int`, `fn main()`, or `main` returning a sized
integer (`i32` for C's `int main`). Its result is the exit status, as a C `int`.

**Memory.** Memory management is the program's choice (D72):

- `gc`, the default: precise tracing collection.
- `manual`, set with `memory manual;` after the module line, or `set_memory` in the builder. There
  is no collector at all: no shadow stack, no roots, nothing ever collected. The builtin `free(r)`
  releases an object, array, string, map or closure, and an object freed twice panics. `print` and
  `eprint` free the text they make for a value that is not a string. Without the collector, inline
  structs and pointers may hold references too, since nothing traces them.

## Expressions

Everything is an expression with a type. A block `{ a; b; c }` evaluates to its last expression
when that has no trailing `;`, and otherwise to `()`.

| Form | Meaning |
|---|---|
| `()`, `true`, `42`, `1.5`, `"text"`, `null<T>` | literals (`null<T>` has type `T?`) |
| `x`, `f`, `f::<T>` | a local, a global, or a function used as a closure value |
| `let x: T = e;`, `let x = e;` | declare a local (the type defaults to `e`'s) |
| `x = e`, `o.f = e`, `a[i] = e` | assignment (type `unit`) |
| `a + b`, `-a`, `a < b`, `a == b`, `a && b`, `!a` | operators, see below |
| `f(a, b)`, `f::<T>(a)`, `e(a, b)` | call a function (with explicit type arguments), extern or closure |
| `e!` | the value of a `T?`; a runtime error if it is null |
| `fn(x: int) -> int { x + 1 }` | a closure; captures outer locals by reference |
| `new S { f: e, ... }`, `new S<T> { ... }`, `o.f` | struct construction (every field) and field access |
| `E::V(a, b)`, `E<T>::V`, `match e { V(x, y) => ..., _ => ... }` | enum construction and matching |
| `[a, b]`, `[]`, `new array<T>`, `a[i]` | arrays (index out of range panics) |
| `new map<K, V>`, `m[k]` | maps (a missing key panics) |
| `new object` | a dynamic object with no fields |
| `e as T`, `e is T` | conversions and dynamic type tests |
| `if c { } else { }`, `while c { }`, `loop { }` | control flow; `if` with `else` is a value |
| `if let x = e { } else { }` | narrowing: `x` is `e`'s value in the first block when `e: T?` is not null |
| `break`, `continue`, `return e` | type `never` |
| `throw e` | raises an exception holding `e` (of any type, as an `any`); type `never` |
| `try { a } catch x { b }` | `a`, or if `a` raises an exception, `b` with its value in `x: any` |

**Operators.**

| Operators | On |
|---|---|
| `+ - * / %` | two integers of one type, wrapping at its width. `/` and `%` by zero panic, and are unsigned for unsigned types. Or two floats of one type (no `%`). |
| `+` | also two `string`s, which it concatenates |
| `< <= > >=` | numbers (unsigned compare for unsigned types) or strings (byte order) |
| `~` | an integer, whose bits it flips |
| `shl`, `shr` | an integer and an amount of its own type, modulo its width; `shr` keeps the sign of signed types |
| `==`, `!=` | any two values of the same type: by value for value types and strings, by identity for other references; on `any` see "Conversions and dynamic values" |
| `!` | a `bool`, which it negates |

`&`, `|` and `^` work on two integers or two `bool`s, with no short-circuit. `&&` and `||` work on
two `bool`s and short-circuit.

The text format writes the shifts as `shl` and `shr`, not `<<` and `>>`, because `>>` would close
two type argument lists, as in `array<array<int>>`.

**Sized numbers.** A number literal is an `int` or a `float`, unless a sized number type is
expected of it. Examples:

- `let b: u8 = 200;`
- `x + 1` with `x: i32`
- an argument or a result
- `[1, 2]` for an `array<u8>`
- expressions made only of literals and operators, or branches ending in them
  (`if c { 1 } else { 2 }`)

The literal must fit (E2023). Arrays of sized numbers keep each element in its own width
(`array<u8>` is bytes), which C sees as `uint8_t *` and so on. A sized number converted to `any`
becomes an `int` (or a `float`). `any` does not convert back to a sized type directly.

**Builtins** (called like functions):

| Builtin | Meaning |
|---|---|
| `len(x)` | length of a string (bytes), array or map |
| `filled(n, v)` | a new array of `n` elements, each `v` (a negative `n` or a byte count too large to allocate panics) |
| `push(a, v)`, `pop(a)` | append to / remove the last element of an array (`pop` on empty panics) |
| `has(m, k)`, `remove(m, k)`, `keys(m)` | map membership, removal, keys as an array (deterministic order) |
| `has_field(o, name)` | whether a dynamic object has a field |
| `field(o, name)` | a field's value, as an `any`; a missing field panics |
| `set_field(o, name, v)` | sets or adds a field (any value, as an `any`) |
| `field_names(o)` | the field names, in the order they were added |
| `substr(s, start, count)`, `byte_at(s, i)` | string slicing and bytes |
| `str(x)` | format an `int`, `float`, `bool` (or `string`) as a string (floats: shortest round-trip, `1` not `1.0`) |
| `print(s)`, `eprint(s)` | write a string to stdout / stderr (no newline added) |
| `exit(code)`, `panic(message)` | stop the program (`panic` prints `panic: message` and the stack trace, exit 101) |
| `free(r)` | release a reference's memory (`memory manual` only, E2026) |
| `clock()` | processor time in seconds, as a `float` |

## Errors: exceptions and results

The HIR has both styles of error handling, and they work the same in every backend.

**Exceptions.** `throw e` unwinds to the innermost `try` around it, in its function or a caller
(through closures too). The `try`'s handler gets the value as an `any`. An exception nothing
catches ends the program: it prints `panic: uncaught exception: <value>` and the stack where it was
thrown, and exits with 101.

Runtime errors, such as an index out of bounds or a failed conversion, are not exceptions: they end
the program, as before. There is no `finally`, so a language puts the cleanup in both places.
Builder: `throw`, `try_catch`, and `declare_catch` for the handler's local.

**Results.** A result is `std.Result<T, E>`, or a language's own enum whose first variant holds the
value. It is matched like any enum. `propagate(e)` is a language's `e?`:

- when `e` succeeded, it is the value;
- otherwise it is a `return` of the failure, made again as the function's result (the same enum);
- for `e: T?`, the failure is null and it is a `return null`.

`propagate` is built from `match` (or `if let`) and `return`, so lowering and the optimizer see
nothing new. Misuse is E2020.

## Conversions and dynamic values

`e as T` converts:

- between any two number types: integers wrap or extend by the source's sign; floats to integers
  truncate and saturate, NaN to 0; floats round to `f32`;
- any value to `any`;
- `any` to any type it holds. It panics with `expected T` otherwise, and `()` becomes null for
  `T?`;
- `T` to `T?`;
- `any` to a type parameter `T`, for generic code that keeps values in `any` (`std.thread`). Once
  `T` is known, it is checked where it can be: numbers are checked to be numbers. Arrays, maps,
  functions and pointers are taken as they are, since what a generic function put in an `any` it
  takes out as the same type.

`e is T` tests the type held by an `any`: `unit`, `bool`, `int`, `float`, `string`, a struct, or an
enum (true for every variant).

`==` on two `any` values compares:

- floats by IEEE equality;
- ints by value;
- strings by content;
- booleans and `()` by value;
- other references by identity.

Values of different types are unequal.

`any` is NaN-boxed into 64 bits: it uses the many bit patterns that mean NaN to hold other values.

- Floats are stored as themselves, with NaNs canonicalized.
- `()` and the booleans are special NaN patterns.
- References are tagged pointers.
- `int`s that fit in 48 bits are stored inline, and larger `int`s are boxed on the heap.

Boxing a null reference yields `()`.

## Inline structs

`inline struct Vec2 { x: f32, y: f32 }` declares a struct whose values are their fields (D70). A
local, a field of an object, an array element or a global holds the bytes, laid out as C lays out
the struct. Assigning, passing or returning one copies them:

- `v.x = 1.0` changes `v` and nothing else;
- `a[i].x = 1.0` changes the element in place;
- `let w = v;` makes a copy.

Arrays of them keep each element's bytes in a row (`filled(n, v)`, `push`, `pop`), and objects
that hold one hold its bytes in their words. Methods, generics (`array<Vec2>`,
`fn id<T>(x: T) -> T`), enum payloads and closures that capture one all work.

An inline struct holds only numbers, booleans and other inline structs (E1015). It never holds
itself (E1016) and takes no type parameters, so the collector never needs to look inside one.

Its values have no identity and no type information of their own. So they cannot be:

- compared with `==`;
- converted to `any` or thrown;
- a map's value or a field of a dynamic object;
- used as interfaces (E2024).

They can satisfy bounds of generic functions. Builder: `set_struct_inline` after
`declare_struct`. Text: `inline struct`.

## Pointers

`ptr<T>` is a raw address of values of a plain type `T`: numbers, booleans, pointers or inline
structs. It is what a systems language and C pass around (D71). Nothing about it is checked or
traced:

| Form | Meaning |
|---|---|
| `p[i]`, `p[i] = v` | element `i` at `p` (`p[i].x` for inline structs), no bounds check |
| `offset(p, n)` | `p` moved by `n` elements |
| `addr(place)` | the address of a local (which then lives in memory), a field, or an element (checked) |
| `data(a)` | an array's elements (`ptr<T>`), or a string's bytes (`ptr<u8>`), valid until it grows |
| `size_of::<T>()` | the bytes a `T` takes (C's `sizeof`) |
| `p as ptr<U>`, `p as int`, `n as ptr<T>` | conversions; `0 as ptr<T>` is a null pointer |

C functions take and return pointers (`extern fn malloc(int) -> ptr<u8>;`). Checked operations stay
the default: arrays check their bounds and references cannot be null. A pointer is the way out that
a language chooses where it needs one. A pointer does not keep what it points at alive, so a
pointer into an object of the collector is only good while something else holds the object.

## Dynamic objects

`object` is for dynamically typed languages, whose objects get fields by name at run time: Lox's
instances, a JavaScript object, a Python `__dict__`. Its fields are `any` values:

- `set_field(o, "name", v)` sets one, and adds it if `o` has no field of that name;
- `field(o, "name")` reads one; a missing field panics with ``no field `name` ``;
- `has_field` and `field_names` ask what fields there are.

Name lookup is the framework's work, and fast (D66):

- Field names are interned: there is one string object per distinct name, so names compare as
  pointers.
- Objects that got the same names in the same order share a *shape*, which says which slot holds
  each name. Finding a field is a question about the shape, not the object.
- Where the name is a string literal, lowering keeps a cache at that place in the code: the shape
  seen last and the slot. A literal counts after the optimizer too, for example in a prelude
  function inlined with a literal argument. An object of that shape is then one comparison and one
  load away. Setting a field that adds it to objects of a cached shape is a comparison and two
  stores. Other names go through the runtime and a table by shape and name.

A language with classes keeps an instance's fields in an `object` and its class's methods in
another (`examples/lox/prelude.hir`). Shapes and interned names live as long as the program. A
program that makes up unboundedly many distinct field names (a dictionary keyed by data) should
use a `map`.

## Closures

A `fn(...) { ... }` expression captures the outer locals it uses **by reference**. Assignments are
seen by the closure and by the enclosing function. Every execution of a `let` creates a fresh
variable, so a closure created in a loop body sees that iteration's variables.

A top-level function used as a value becomes a closure with no captures. Stack traces and debuggers
call a closure `f::{closure}` after the function it is in, unless the frontend names it
(`set_closure_name`, for a language whose functions are closures).

## Building HIR from a frontend

A frontend walks its syntax tree once and calls `HirBuilder`. Two styles mix freely.

**By handle**, for a language that resolves names itself (Lox, with jlox's rules): `new_local`,
`local`, `call(FuncId, ...)`, `new_struct(StructId, ...)`.

**By name** (`frontend.rs`), for everything else (Minnow):

| Call | Does |
|---|---|
| `begin_body`, `end_body` | put a body's parameters (and a generic function's type parameters) in scope |
| `push_scope`, `pop_scope` | follow blocks |
| `let_named`, `declare_local` | declare locals; shadowing is allowed, and `is_declared_in_scope` is for languages that forbid it |
| `declare_if_let` | declares the binding of an `if let`, typed from the value, for `if_let` |
| `name`, `call_named`, `assign_named` | resolve locals, globals, functions and builtins |
| `named_type` | resolves type names, and `array` and `map` with their type arguments unless the language declares types of those names |
| `new_struct_named` | takes fields in any order, keeps their evaluation order, and reports unknown, repeated and missing fields |
| `variant_named`, `field_named` | look up by name |

**Errors** are reported at the span set with `set_span`, with codes, related locations and
suggested fixes, in the language's words (`docs/diagnostics.md`).

- `report_code` adds a language's own errors.
- `error_expr` stands in for an expression that could not be built. Its type fits everything, so
  it causes no further errors.
- `set_type_syntax(TypeTag::Array, "[{}]")` makes diagnostics write types in the language's
  syntax, and `set_term` names its concepts.
- `set_name_span` and `set_param_span` say where names and parameters are, for fixes and related
  locations.

**Text.** `add_text` adds items written in the text format to a builder, for runtime code a
language keeps as text (Lox's `natives.hir`, which uses the standard library). Item names are free.
One that is a HIR keyword (`map`) or not an identifier is written in backquotes in the text format.

**Debugging.** Spans also place the code for debuggers (`docs/debugging.md`). Each new line of a
function is a place to stop, and lowering describes every local by its source name and type. So a
frontend that sets spans on what it builds, literals included, gets a debugger with nothing more to
do.

`a0_hir::dsl` is a frontend of this kind that needs no code from the language. It builds HIR by
name from any syntax tree whose grammar gives its rules `@hir` forms (`docs/grammar.md`,
"Languages without host code"). It is what `a0 lang` runs.

## Modules

A program can have several files, each a module of one HIR module. A language opts in with two
things:

- it gives the driver an `a0_driver::Modules`: its file extension, a `start` that sets up the
  builder, and a `compile` for one file;
- it marks its import rule `@import` in its grammar.

The driver then loads the main file and everything it imports, and rejects import cycles and
missing files. It compiles the files in dependency order, each after `begin_module`, with its
imports already in scope.

- Module 0 is the prelude: everything declared before the first `begin_module`. Every module sees
  it.
- The main file's items keep their names, and its `main` is the entry point. Another module's items
  are `module.item` in the HIR, the IR and stack traces (`text.json.parse`).
- `set_public(bool)` says whether the declarations that follow can be used from other modules. The
  default is public; Minnow makes items private unless they are `pub`.
- `import_module(name, alias)` lets the current module use `alias.item`. `import_item(module, name)`
  imports one item by its name. The driver calls both from the `@import` rules.
- By-name helpers (`name`, `call_named`, `named_type`, `lookup_type_name`) see the module's own
  items, the items it imported by name, and the prelude.
- `member`, `call_member`, `member_type` and `member_item` reach `alias.item`. `is_module(alias)`
  tells a frontend that `alias.item` is not a field access.
- A private item (E1011), an unknown one (E1001, with a suggestion) and an unknown module (E1010)
  are reported.
- `finish_keeping` returns the module with its diagnostics even when it has errors, for tools.
- `std` and `std.*` are the standard library's modules (`docs/std.md`). Importing one adds it to
  the program, and the driver never looks for them as files.

## Text format

The examples above are the text format. `print(parse(text))` is canonical and
`print(parse(print(m))) == print(m)`. Names are resolved lexically, and a block opens a scope.
Comments are `//` to the end of the line. The parser is written with A0's own grammar layer
(`crates/a0-hir/src/hir.a0g`).

## Lowering

HIR is lowered to the core IR plus calls into `a0rt`. Before that, monomorphization (`mono.rs`)
replaces generic items with one copy per combination of type arguments.

- Value types map to `i1`, `i64`, `f64`; `any` to `i64`; references to `ptr`; `unit` disappears.
- **Precise GC without stack maps.** The collector finds references through a *shadow stack*: a
  stack of frames, kept by the runtime, holding the references each active function has
  (`a0_gc_frame_push`/`pop`).
  - Every reference or `any` that lives across something that can allocate is kept in a slot of
    the function's frame, stored in the `any` encoding.
  - Locals of these types are IR variables whose every assignment is also written to their slot.
    The collector never moves objects, so the variable stays valid. A local that nothing binds,
    assigns or reads, such as one whose `let` the optimizer removed, always holds zero and gets
    no slot.
  - A local declared in a block has its slot only while the block runs.
  - Temporaries are spilled to a slot only when a later operand or the operation itself may
    allocate. Reads of locals that are never reassigned are not spilled.
  - A slot is cleared when its scope ends or is left by `return`, `break` or `continue`. So the
    stack above its top is always zero, and pushing a frame clears nothing (D30).
  - A function that cannot start a collection pushes no frame: one that allocates nothing,
    itself or through the functions it calls, such as a tree walk that only reads. Its callers
    spill nothing around calls of it. A call through a closure, an interface or to a C function
    counts as allocating, and in a program with threads every function keeps its frame, since
    its loops wait for collections.
  - Globals of these types are registered as roots. Heap objects are traced by layout.
- **Objects.** Every struct, enum variant and closure environment gets a type descriptor,
  registered at startup, with a layout: one byte per word, saying raw, reference or `any`. Field
  access is an inline load or store. References are non-null by type, so only `!` checks for null.
- **Closures.** Each closure is a core IR function taking its environment first. Captured locals
  live in heap boxes, shared by the defining function and every closure that captures them.
- **`any`.** Type tests of objects are inline. The object's type sits in the collector's header and
  its family in the type descriptor, at offsets `a0rt.h` fixes (D28). Equality of two `any` words
  is inline unless both are references (strings and boxed integers compare by value in the
  runtime).
- **Inline loads.** Lengths of strings and arrays, `byte_at` and array elements are inline loads
  and stores after a bounds check. Out of bounds, the runtime function fails with the message and
  the trace. Everything else about strings, arrays and maps, formatting, and boxing of large
  integers are runtime calls.
- **Division.** Integer division and remainder by zero for `int` and sized integers, and signed
  division of the minimum value by -1, trap like the core IR. They print `panic: integer division
  by zero` or `panic: integer overflow`, with the stack trace, and exit with 101.
- **Locations.** Every IR instruction is attributed to the HIR expression it comes from (its span),
  and to the call it was inlined at if the optimizer inlined it. Every function gets its HIR name
  for stack traces: closures are `f::{closure}`, and entry points and wrappers get none. So runtime
  errors name source locations and source functions (`docs/diagnostics.md`).
- **Interfaces.** A type descriptor has an interface table (offset 32, `a0rt.h`). For each
  interface of the program, it holds the type's methods for it, or null. Lowering fills the tables
  at startup. A call on an interface value loads the object's type from its header, then the table,
  then the method, and calls it. Converting an object to an interface costs nothing. Methods of
  type parameters are plain calls once monomorphization knows the type.
- **Only what `main` reaches** is lowered: the functions it calls, directly or through others, and
  the closures it can make. A prelude or a library costs only what the program uses.
- **Exceptions** (D49) are values in two globals, a flag and the value (a root), and explicit
  checks.
  - `throw` sets them and records where it happened (`a0_rt_throw`). It then leaves to the
    innermost `try` of its function, whose body runs in an IR loop left with `break N`. If there is
    none, it returns a zero value with the exception pending.
  - After a call that may raise one, generated code tests the flag and does the same.
  - A function may raise if it throws or calls one that may. A call through a closure may raise if
    any closure the program makes may.
  - So a program that never throws has no checks at all, and one that does pays a load and a
    branch after the calls that can raise.
- `a0 lower prog.hir` prints the core IR a module lowers to. `a0 run` and `a0 build` accept `.hir`
  files.

## Optimization

Before lowering, the HIR optimizer (`opt.rs`) runs on the monomorphized module. It is on by
default and needs nothing from frontends. `A0_OPT=0` (or `LowerOptions { optimize: false }`) turns
it off. It complements Cranelift, which optimizes within each function. Like lowering, it skips
the functions that `main` and the interface tables cannot reach through calls and function values;
they are still checked, but never optimized or lowered. It does five things.

**Inlining** of small functions: those with no closures inside that do not call themselves.
Callees are inlined first.

- A function is inlined if it has at most 120 nodes after its own inlining.
- A body grows to at most 4000 nodes or twice its size (D37), so inlining stays bounded however
  calls are chained.
- A `return` in the middle of an inlined function becomes the value of an `if`:
  `if c { return a } rest` is `if c { a } else { rest }`. Functions with other `return`s stay
  calls.
- The copies remember where they were inlined, so stack traces show the inlined functions.

**Inlining of known closures** (D84). After a function's calls are inlined, a call of a closure the
function makes is inlined too, under the same limits. The closure must be called through a local
bound to it and never assigned: `let f = fn(...) {...}`, or the parameter of an inlined
`fold(xs, 0, fn(a, b) { a + b })`. The copy uses the variables the closure captures, so it shares
them with the closure as a call would. A closure no longer made after that is dropped, and the
variables it captured need no box.

**`any` folding.** `(x as any) is T` and `(x as any) as T` are decided from `x`'s type, except
for a sized integer, which boxes as an `int`, and an `f32`, which boxes as a `float`: tests of
those are left to run time. This also works through a local: `let v = <... x as any>` (never
reassigned) becomes `let raw = <... x>; let v = raw as any`, so tests of `v` fold and `v as T`
reads `raw`. A branch may also end in a local already known to hold `raw as any`, as in Lox's
`{ let left = a; if truthy(left) { b } else { left } }`, unless `raw` is an `int` or a sized
integer, whose boxing can allocate.

**Guarded casts.** In `if v is T { ... v as T ... }`, with `v` a local that is never assigned, the
cast does not check again.

**Simplification** of:

- constant `if`, `&&`, `||` and `!`;
- casts to the same type;
- copies of locals and of constants;
- unused locals, and statements without effects;
- nested blocks;
- code after something that never finishes.

Only expressions without effects are removed or duplicated, so traps, panics, output and evaluation
order stay as written. Every test program runs optimized and unoptimized, interpreted, native and
under GC stress, and all must agree, stack traces included. So must generated random programs
(`crates/a0/tests/random.rs`, D38).

## Not yet

Default methods in interfaces, generic interfaces, `finally`, iteration protocols, float map keys,
and memory modes other than `gc` and `manual`.
