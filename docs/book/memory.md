# Memory: the collector, manual memory and C

A language on A0 chooses how its programs' memory is managed, per program, with one line:
`b.set_memory(Memory::Gc)` (the default) or `b.set_memory(Memory::Manual)`. Everything else
about values is the same in both: the types, the code that uses them, the debugger.

## The collector

With `Memory::Gc`, structs, arrays, maps, strings, closures and dynamic objects live on a heap
that a precise mark-and-sweep collector manages (D16, D64). Generated code keeps the references a
function holds in a *shadow stack* the collector reads, so it knows exactly what is live; a
collection starts when the heap has grown to three times what survived the last one, which keeps
the collector's share of the time bounded whatever the heap's size (D84 measured it). With
threads, a collection stops every thread at a safe point (D76). `A0_GC_STATS=1` prints what the
collector did; `A0_GC_STRESS=1` collects at every allocation, which every test suite here runs
under, so that a missing root shows up at once.

## Manual memory

With `Memory::Manual` there is no collector at all: the program frees what it allocates with the
builtin `free(x)` (D72), and freeing twice is reported. B0 says `memory manual;`, Tin always runs
that way. Code that needs no collector (a kernel of numeric code, a program that allocates once)
runs as C does: Tin's `binarytrees` runs at clang -O0's speed (`docs/benchmarks.md`).

## Values in place: inline structs

An *inline struct* (`docs/hir.md`, "Inline structs"; D70) is its fields: a local, a field, an
array element holds its bytes, laid out as C lays out the struct, and assigning one copies it. An
array of `Vec2 { x: f32, y: f32 }` is the floats in a row, as in C. Inline structs hold numbers,
booleans, pointers and other inline structs, so the collector never looks inside one. In B0:
`inline struct Vec2 { x: f32, y: f32 }`; in Tin every struct is one.

## Raw pointers

`ptr<T>` is an address of `T`s, with no checks (D71): `addr(x)` takes a variable's address,
`data(xs)` an array's elements, `offset(p, n)` moves, `p[i]` reads and writes. A language decides
whether and where to offer them. Tin offers them as C does; B0 only inside `unsafe { ... }`
(`E2027` elsewhere); Minnow does not. `size_of::<T>()` (B0's `sizeof(T)`) is a type's size.

## C functions

`extern fn` declares a function written in C (`docs/hir.md`, "Types"): the C library's, a
library's linked into the program, or a language's own C runtime (`docs/runtime.md`). It takes
numbers, `bool`, pointers, strings (copied to a NUL-terminated buffer for the call), arrays of
numbers, and inline structs by value as C structs (D73: one implementation of each platform's C
ABI, in every backend), and returns one of them. The interpreter finds the function in the
runtime, in a language's tables, or in the process, so programs that call C behave the same
interpreted and compiled. Tin calls cJSON, a C library it ships, this way.

## Choosing

| | Collector | Manual |
|---|---|---|
| Who frees | the collector | the program, with `free` |
| Safety | no dangling references | the program's responsibility, as in C |
| Cost | collections (a third of `binarytrees`' time) | none beyond the allocator |
| Examples | Minnow, Lox, Kaleidoscope, B0 by default | Tin, B0 with `memory manual;` |
