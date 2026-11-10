# Dynamic languages

A0's high-level IR (HIR) is typed, but a language on A0 can be dynamically typed: its programs
check the types of values while they run. [Lox](../../examples/lox/README.md), from the book
*Crafting Interpreters*, is the example. Built as an executable, Lox on A0 runs faster than clox, the book's own bytecode machine
in C, on five of its six benchmarks ([performance](../benchmarks.md)).

This chapter shows a Lox program, then what Lox's compiler does to run it.

## A dynamic program

```
class Point {
  init(x, y) {
    this.x = x;
    this.y = y;
  }
  sum() {
    return this.x + this.y;
  }
}

var p = Point(1, 2);
print p.sum();
p.label = "corner";
print p.label + "!";
print p.label + p.x;
```

```
$ lox dyn.lox
3
corner!
Operands must be two numbers or two strings.
[line 15]
```

The program exits with status 70, as jlox, the book's interpreter in Java, does. Three things in it
are dynamic:

- `+` adds numbers and joins strings. Which one it does depends on the values, so it is decided
  while the program runs. The last line adds a string and a number, which is an error.
- `p.label = "corner"` adds a field to `p` that its class never mentioned.
- Variables have no types: `p` could hold any value.

## Every value is an `any`

The HIR type `any` holds a value of any type. Every Lox value is an `any`. Three forms work with
it:

| Form | What it does |
|---|---|
| `v as any` | puts a value in an `any` |
| `v is T` | tests whether an `any` holds a `T` |
| `v as T` | takes the `T` out of an `any`, and panics if it holds something else |

Taking out the wrong type stops the program. In B0, `("x" as any) as int` panics with:

```
panic: expected `int`, found `string`
```

An `any` is 64 bits. Floats, booleans, references and integers that fit in 48 bits are stored in
those 64 bits. Larger integers are stored on the heap.

## Lox's rules are written in HIR

Lox's compiler does not build the checks for `+` itself. It calls a function of Lox's *prelude*,
`examples/lox/prelude.hir`. The prelude is written in the HIR's text format, and holds Lox's rules
for operators, truthiness, calls, classes and errors. Its `add` is:

```
fn add(a: any, b: any, line: int) -> any {
  if a is float && b is float {
    ((a as float) + (b as float)) as any
  } else {
    add_strings(a, b, line)
  }
}
```

The compiler parses the prelude, and adds the program to it. For each binary operator it calls
the prelude's function for that operator, with the line for error messages:

```rust
"+" => "add",
"-" => "subtract",
"*" => "multiply",
...
let line = self.line(op);
self.rt(f, &[a, b, line])
```

`lox hir` prints the HIR the compiler built. For the program `var a = 1; print a + 2;` it prints
this `main`:

```
fn main() -> int {
  g_clock = native_clock();
  d_clock = true;
  try {
    g_a = 1.0 as any;
    d_a = true;
    print_value(add({
      if !d_a {
        undefined_variable("a", 2);
      }
      g_a
    }, 2.0 as any, 2));
  } catch error {
    uncaught(error);
  }
  0
}
```

The Lox global `a` became two HIR globals: its value `g_a`, and `d_a`, which says whether it has
been defined yet. Every number became a float in an `any`, since Lox's numbers are floats.

Writing a language's rules in HIR keeps its compiler small. Lox's compiler is about 1,400 lines of
Rust. A0's interpreter, its JIT (which compiles to machine code in memory) and executables all run the
same prelude, so they behave the same.

## The optimizer removes the checks it can

Calling `add` for every `+` would be slow. A0's optimizer makes it fast:

1. It inlines small functions, such as `add` and `truthy`, where they are called.
2. After inlining, a test can often be decided while compiling. If `x` is known to be a
   `bool`, `(x as any) is bool` is always true, so the test and the branch disappear.

Cranelift, the code generator under A0, optimizes within one function. It does not inline across
functions, and it does not know what an `any` holds. That is why A0 does these two steps itself
([the high-level IR](../hir.md#optimization)).

## Dynamic objects

`new object` makes a *dynamic object*: an object whose fields are named while the program runs.
Its fields are `any` values:

| Builtin | What it does |
|---|---|
| `set_field(o, "name", v)` | sets a field, and adds it if `o` has none of that name |
| `field(o, "name")` | reads a field, and panics if there is none |
| `has_field(o, "name")` | whether `o` has the field |
| `field_names(o)` | the names of `o`'s fields |

A Lox instance keeps its fields in one dynamic object, and its class keeps its methods in another.
That is how `p.label = "corner"` above works. B0 has dynamic objects too, and writes their fields
as fields: `o.name = "x";` is `set_field(o, "name", "x" as any)`.

Looking up a field by name is fast:

- **Names are interned.** There is one string object for each distinct field name, so comparing
  two names compares two pointers.
- **Objects have shapes.** Objects that got the same field names in the same order share a shape.
  The shape says which slot holds each name, so finding a field is a question about the shape,
  not the object.
- **Each place in the program keeps a cache.** Where the field's name is written as a literal,
  generated code remembers the shape it saw last and the slot. Reading a field of an object of
  that shape is then one comparison and one load.

Programs that start threads do not use the caches, since two threads could tear them
([concurrency](concurrency.md)). Shapes and interned names live as long as the program, so data
with unboundedly many keys belongs in a `map`.

## Exceptions

```
fun check(n) {
  if (n > 2) throw "too big";
  return n;
}

try {
  print check(1);
  print check(5);
} catch (e) {
  print "caught: " + e;
}
check(7);
```

```
$ lox throw.lox
1
caught: too big
Uncaught exception: too big.
[line 2]
```

`throw` and `try`/`catch` are A0's additions to Lox. The HIR's `throw` raises any value, and
`try`/`catch` handles it, in the same function or in a caller. An exception nobody catches ends
the program with a stack trace. Lox's prelude catches it first, and ends the program with jlox's
message and exit status 70. Lox's runtime errors, such as adding a string to a number, are not
exceptions: they end the program at once, as in the book.

A typed language can return `std.Result` and use `?` instead, or have both, as B0 does
([the high-level IR](../hir.md#errors-exceptions-and-results)).
