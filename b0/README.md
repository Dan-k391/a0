# B0

B0 is A0's flagship language: a statically typed language that uses everything A0 offers, built
only on A0's public API (`docs/compatibility.md`), as anyone's language would be. Its compiler is
one walk over the syntax tree that builds HIR by name; A0 does the rest: names, type inference and
checking, generics, the optimizer, the collector or manual memory, threads, the interpreter, the
JIT, native code for every target, debug information, the language server, the debugger and the
REPL.

```
import std.thread;

struct Point { x: float, y: float }
enum Shape { Circle(float), Rect(float, float) }

interface Area { fn area(self) -> float; }
impl Area for Shape {
  fn area(self) -> float {
    return match self { Circle(r) => 3.14159 * r * r, Rect(w, h) => w * h };
  }
}

fn total<T: Area>(xs: [T]) -> float {
  return fold(xs, 0.0, fn(sum: float, x: T) -> float { return sum + x.area(); });
}

fn main() {
  let shapes = [Shape::Circle(1.0), Shape::Rect(2.0, 3.0)];
  print(total(shapes));
  let answers: thread.Channel<int> = thread.channel(1);
  let worker = thread.spawn(fn() { thread.send(answers, 6 * 7); });
  match thread.receive(answers) {
    Ok(n) => print(n),
    Err(e) => print(e),
  }
  thread.join(worker);
}
```

```
b0 run main.b0                  # run at once (A0_INTERP=1 interprets)
b0 build main.b0 -o main        # an executable (--target for another platform, -g for debuggers)
b0 hir main.b0 | b0 ir main.b0  # what the compiler built
b0 repl                         # the REPL
b0 lsp | b0 dap                 # the language server and the debugger (editors/vscode)
b0 run                          # in a package: the program its b0.toml names
```

`examples/wordfreq` is a complete program: a package of two modules that counts the words of
files on several threads.

## The language

**Values.** `int` (64 bits), `i8` to `i64` and `u8` to `u64`, `float` (64 bits) and `f32`, `bool`,
`string`, arrays `[T]`, maps `{K: V}` (keys are `int`, `bool` or `string`), structs, inline
structs, enums, closures `fn(A) -> R`, interfaces, `any` and `object`. A number literal takes the
type expected of it (`let b: u8 = 200;`); `0xff` and `1_000_000` are numbers too. `as` converts
between numbers, and into and out of `any`; `v is T` tests what an `any` holds. Sized integers wrap.

**Statements.** `let x = e;` (with `: T` to say the type), `x = e;` and `x += e;` (also `-=`, `*=`,
`/=`, `%=`, `&=`, `|=`, `^=`, `<<=`, `>>=`), `if`, `while`, `for x in xs` (an array), `for k in m`
(a map's keys), `for i in 0..n`, `break`, `continue`, `return`, `match`. Operators are C's,
`&&` and `||` short-circuit, and `~` flips bits.

**Structs** are references to objects that the collector frees when nothing refers to them:
`struct Node { value: int, next: Node? }`, made with `new Node { value: 1, next: null }`. **Inline
structs** are their fields, copied when assigned or passed and stored in place in arrays and other
structs: `inline struct Vec2 { x: f32, y: f32 }`, made with `new Vec2 { x: 1.0, y: 2.0 }`; they
hold numbers, booleans, pointers and other inline structs. **Enums** have variants with payloads
and are taken apart with `match`, which must cover every variant or have `_`.

**Nullable** references are `T?`: `if let n = node.next { ... }` and `while let` narrow them, `x!`
asserts, and `x?` returns `null` from the function when `x` is. Only `T?` can be `null`.

**Functions** are generic with bounds, `fn largest<T: Ord>(xs: [T]) -> T?`. **Methods** live in
`impl Type { fn name(self, ...) { ... } }`; an **interface** lists methods, `impl Iface for Type`
implements it, and a value of the interface's type finds them at run time. Numbers, booleans and
strings implement interfaces too.

**Closures** capture variables by reference: `let add = fn(x: int) -> int { return x + k; };`.
The prelude, written in B0 (`src/prelude.b0`), has `map`, `filter`, `fold`, `each`, `any_of`,
`range` and `sort` (stable) for arrays; a closure passed to one of them is inlined. A program's own
function of the same name takes the place of the prelude's. Builtins: `len`, `push`, `pop`,
`has`, `remove`, `keys`, `substr`, `byte_at`, `str`, `print`, `panic`, `exit`, `clock`.

**Errors.** A function that can fail returns `std.Result<T, E>` (`import std;`); `e?` is its value
or returns the failure. Exceptions are for what should not happen: `throw e;` raises any value,
`try { ... } catch e { ... }` handles it with the value in `e`, an `any`, and an exception nobody
catches ends the program with a stack trace, as panics do (`panic("...")`, an index out of
bounds, `null!`).

**Dynamic data.** `any` holds any value. `new object` is a dynamic object whose fields are named
at run time: `o.name = "x";` and `o.name as string`, `has_field(o, "name")`,
`field_names(o)`; for data whose shape is not known in advance.

**Threads.** `import std.thread;`: `thread.spawn(f)` runs the closure `f` on a thread of its own
and returns a handle for `thread.join`; `thread.channel(n)` makes a channel for `thread.send` and
`thread.receive`. The collector works with every thread, and threads run at the same time on all
processors.

**Memory.** The collector frees memory by default. A program that starts with `memory manual;`
frees it itself with `free(x)`, and has no collector at all. Raw pointers `ptr<T>` are allowed
only inside `unsafe { ... }`: `data(xs)` (an array's elements), `addr(x)`, `offset(p, n)` and
`p[i]` are unchecked there, and an error (`E2027`) anywhere else. `sizeof(T)` is a type's size in
bytes.

**C.** `extern fn strlen(s: string) -> u64;` declares a function of the C library (or of one
linked into the program): it takes numbers, `bool`, pointers, strings and inline structs (by
value, as C structs) and returns one of them.

**Modules and packages.** `import geometry.shapes;` makes the public (`pub`) items of
`geometry/shapes.b0` available as `shapes.Point`; `import geometry.shapes as s;` names them `s`.
`import std.fs;` imports a module of the standard library (`docs/std.md`). A package is a
directory with a `b0.toml` (`docs/packages.md`): its main file and its dependencies, local or from
git.

## The compiler

| File | Role |
|---|---|
| `b0.a0g` | the grammar (version 1 of the format) |
| `src/lib.rs` | the compiler: one walk over the syntax tree, building HIR by name |
| `src/prelude.b0` | the prelude, compiled into a module that every file imports from |
| `src/main.rs` | the `b0` command: `Language::new("b0", ...)`, an `a0-driver` language |
| `tests/` | programs run interpreted, in the JIT, natively and under GC stress, optimized and not; the language server, the debugger, the REPL, packages, threads, diagnostics |
| `examples/wordfreq` | a program of two modules |

B0 depends on `a0-diag`, `a0-syntax`, `a0-hir` and `a0-driver` only, all covered by A0's
compatibility promise; `tests/public_api.rs` checks that it stays that way. What it adds to A0 is
syntax (map literals, ranges, compound assignment), the prelude, and one rule A0 leaves to
languages: raw pointers only inside `unsafe`.
