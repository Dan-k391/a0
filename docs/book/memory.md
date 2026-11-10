# Memory: the collector, manual memory and C

A language on A0 chooses how its programs' memory is freed: by A0's garbage collector, or by the
program itself. It can also give programs values stored in place, raw pointers and calls to C.
This chapter shows a program in [B0](b0.md) that uses all of these, then what a language's compiler does to
offer each one.

## A program that frees its own memory

```
memory manual;

inline struct Vec2 { x: f32, y: f32 }
struct Node { value: int, next: Node? }

fn main() {
  let v = new Vec2 { x: 1.0, y: 2.0 };
  let w = v;
  w.x = 5.0;
  print(str(v.x) + " " + str(w.x) + " " + str(sizeof(Vec2)));

  let nums = [1, 2, 3];
  unsafe {
    let p = data(nums);
    p[0] = 10;
  }
  print(nums[0]);
  free(nums);

  let n = new Node { value: 7, next: null };
  print(n.value);
  free(n);
  free(n);
}
```

```
$ b0 run manual.b0
1 5 8
10
7
panic: freed twice
    at main (manual.b0:23:3)
```

The program exits with status 101. Line by line:

- `memory manual;` turns the collector off. The program frees what it allocates with `free`.
- `Vec2` is an *inline struct*: its value is its fields. `let w = v;` copies the fields, so
  changing `w.x` leaves `v.x` as it was. `sizeof(Vec2)` is 8 bytes, two 4-byte floats, as in C.
- `data(nums)` is a raw pointer to the array's elements. Writing `p[0]` changes `nums[0]`. B0
  allows pointers only inside `unsafe { ... }`.
- Freeing `n` a second time stops the program, with the place it happened.

## Choosing the collector or manual memory

A compiler chooses with one call to the [HIR builder](../hir.md#building-hir-from-a-frontend),
the part of A0 a compiler describes a program to. The choice holds for the whole program:

```rust
b.set_memory(Memory::Manual);   // or Memory::Gc, the default
```

B0 makes the call when a program starts with `memory manual;`:

```rust
match self.text(mode) {
    "gc" => self.b.set_memory(Memory::Gc),
    "manual" => self.b.set_memory(Memory::Manual),
    ...
}
```

[Tin](../../examples/tin/README.md), A0's example systems language, calls `set_memory(Memory::Manual)` for every program. The
other example languages keep the collector.

Nothing else about the language changes. The types are the same, the code that uses them is the
same, and the debugger shows the same values.

## The collector

With the collector, A0 frees structs, arrays, maps, strings, closures and dynamic objects when
nothing refers to them any more. The collector is precise: generated code keeps each function's
references in a *shadow stack* that the collector reads, so it knows exactly which values are
still in use. It marks those values, then frees the rest (mark and sweep).

A collection starts when the heap has grown to three times what survived the last collection, and
to at least 1 MB. Each collection costs about what survives it, so this keeps the collector's
share of the running time bounded however large the heap grows.

`A0_GC_STATS=1` prints what the collector did when the program exits. For a Minnow program that
builds 20 binary trees of depth 14:

```
$ A0_GC_STATS=1 minnow run trees.mn
655340
gc: 13 collections, 3.4 ms (1.5 marking), peak 3.0 MB, 32767 objects live after the last
```

`A0_GC_STRESS=1` makes the collector run at every allocation. A value the generated code forgot
to record in the shadow stack is then freed at once, so the bug shows up at once. Every test suite
in A0 runs its programs this way too.

A program with several threads is collected too: a collection stops every thread first
([concurrency](concurrency.md)).

## Manual memory

With `Memory::Manual` there is no collector at all: no shadow stack and nothing ever collected.
The builtin `free(x)` releases an object, array, string, map or closure. Freeing one twice panics,
as the program above shows.

`free` is only for programs without the collector. With the collector, A0 reports it:

```
$ b0 check freegc.b0
error[E2026]: `free` is for programs that manage their memory themselves
 --> freegc.b0:4:8
  |
4 |   free(n);
  |        ^ the collector frees this when nothing refers to it
  |
  = help: remove it, or declare the program's memory `manual`
```

Code without a collector runs as C does. Tin allocates with C's own `calloc` and `free`, and its
`binarytrees` benchmark runs at about the speed of the C version built with clang -O0
([performance](../benchmarks.md)).

## Values in place: inline structs

An inline struct is its fields. A local variable, a field or an array element holds the struct's
bytes, laid out as C lays out the struct, and assigning one copies it. An array of
`Vec2 { x: f32, y: f32 }` is the floats in a row, as in C.

A compiler makes a struct inline with one call after declaring it:

```rust
let s = self.b.declare_generic_struct(name, &params);
if self.opt(item, "inline").is_some() {
    self.b.set_struct_inline(s);
}
```

That is B0's code, for `inline struct`. In Tin every struct is inline.

An inline struct holds only numbers, booleans, pointers and other inline structs, so the collector
never needs to look inside one. [The high-level IR](../hir.md#inline-structs) lists the rest of
the rules.

## Raw pointers

`ptr<T>` is the address of values of type `T`. Nothing about it is checked: no bounds, no null
check, and it does not keep what it points at alive.

| Form | What it does |
|---|---|
| `addr(x)` | the address of a variable, a field or an element |
| `data(xs)` | the address of an array's elements |
| `offset(p, n)` | `p` moved by `n` elements |
| `p[i]`, `p[i] = v` | reads or writes element `i` |
| `size_of::<T>()` | the size of a `T` in bytes (B0 writes `sizeof(T)`) |

These are A0's builtins, and each language decides whether and where to offer them:

- **Tin** offers them everywhere, as C does.
- **B0** allows them only inside `unsafe { ... }`, and reports error `E2027` anywhere else
  ([B0's `unsafe` rule](b0.md#a-rule-of-b0s-own-unsafe)).
- **Minnow** adds no rule of its own. It builds HIR by name, so `data(xs)` and `p[i]` reach A0's
  builtins as they are.

[The high-level IR](../hir.md#pointers) has the complete list, conversions included.

## C functions

`extern fn` declares a function written in C. B0 writes:

```
extern fn strlen(s: string) -> u64;
```

and its compiler declares it with the builder:

```rust
self.b.declare_extern(name, &types, ret);
```

The function can be from the C library, from a library linked into the program, or from a
language's own C runtime ([the runtime](../runtime.md)). It takes and returns:

- numbers and `bool`;
- pointers;
- strings, passed as a pointer to the string's own bytes, which end with a NUL byte;
- arrays of `int`, `float` or `bool`, passed as a pointer to their elements;
- inline structs, passed by value as C structs, following each platform's C calling convention.

[The high-level IR](../hir.md#module) gives the exact C type of each.

Programs that call C behave the same interpreted and compiled. The interpreter looks for the
function in A0's runtime, in the tables a language registers, and then in the running process.
Tin calls cJSON, a C library it ships, this way.

## Choosing

| | Collector | Manual |
|---|---|---|
| Who frees | the collector | the program, with `free` |
| Safety | no dangling references | the program's responsibility, as in C |
| Cost | collections: about a third of Minnow's `binarytrees` time | none beyond the allocator |
| Examples | Minnow, Lox, Kaleidoscope, B0 by default | Tin, B0 with `memory manual;` |
