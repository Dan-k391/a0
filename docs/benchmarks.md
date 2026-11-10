# Benchmarks

This page answers three questions:

1. **How fast does code compiled by A0 run**, compared with established tools: C (clang), Go,
   LLVM and clox?
2. **How long does A0 take to compile** a program?
3. **What do A0's own options cost**: running with the JIT, running in the interpreter, and turning
   A0's optimizer off?

The programs are small, written the obvious way, and written the same way in each language.
Nothing was tuned for this page, and it is not a CI check. The numbers are for A0 v1.0.0.

## How to read the tables

- **Times are in milliseconds** (`ms`), or seconds (`s`) where marked. **Lower is better.**
- Each time is the median of 5 runs. It is wall-clock time, so it includes starting the process
  (about 1 ms). Compile times include linking.
- **Each table answers one question**, written above it. A0's columns come first; the columns
  after them are what A0 is compared with. The sentence under the table says what it shows.
- The machine is a shared cloud machine with 4 processors. Differences under about 10% are noise,
  and some programs vary more than that between runs (Minnow's `nbody` took 125–195 ms). Earlier
  releases were measured on a faster machine, so compare numbers within this page only.

The words used for A0's ways of running a program:

| Word | Meaning | Command |
|---|---|---|
| executable | A0 builds a native executable, which is then run | `minnow build prog.mn -o prog`, then `./prog` |
| JIT | A0 compiles the program into memory and runs it at once; the time includes compiling | `minnow run prog.mn` |
| interpreted | A0's interpreter runs the program without compiling it | `A0_INTERP=1 minnow run prog.mn` |
| no HIR optimizer | the same, with A0's own optimizer turned off (it mostly inlines small functions) | `A0_OPT=0 ...` |

Minnow, Tin, Kaleidoscope and Lox are example languages built on A0
([the language guides](book/languages.md)).

## Results at a glance

| A0 language | Compared with | Result |
|---|---|---|
| Minnow (typed, with a collector) | C and Go | About as fast as clang -O0, about half as fast as clang -O2, and slower than Go except on allocation |
| Tin (no collector) | C | About as fast as clang -O0 |
| Kaleidoscope | LLVM's tutorial | Slightly faster than the tutorial's code on `mandel` (916 against 1015 ms), 4.2 times slower than LLVM's full -O2 |
| Lox (dynamic) | clox | Faster on five of six programs (up to 2.7 times), slower on `invocation` (417 against 382 ms) |
| Minnow with threads | itself on one thread | 3.8 times faster on 4 processors |
| A large program | | 100,000 lines in 175 modules build in 1.3 s; the language server answers after an edit in about 0.4 s |
| Compiling | clang and Go | A0's own work takes under 20 ms on these programs; most of a build is the linker |

v1.0 changed nothing in the code A0 generates, and runs as fast as v0.9
([From release to release](#from-release-to-release)).

## Minnow against C and Go

Minnow is A0's typed example language, with a garbage collector. Each program is written in
Minnow, C and Go:

| Program | What it exercises |
|---|---|
| `fib` | function calls: naive recursive Fibonacci, `fib(35)` |
| `sieve` | arrays: the primes below 2,000,000, ten times |
| `nbody` | floating point and structs: the Benchmarks Game's n-body, 1,000,000 steps |
| `binarytrees` | allocation: the Benchmarks Game's binary-trees at depth 16 (the C version uses `malloc` and `free`) |
| `closures` | a closure that captures a variable, passed to a small higher-order function `fold`, 100,000 times over 1,000 numbers (the C version passes a function pointer and its environment) |

**How fast does the compiled code run?**

| Program | Minnow (executable) | clang -O0 | clang -O2 | Go |
|---|---|---|---|---|
| fib | 63 ms | 76 ms | 41 ms | 57 ms |
| sieve | 150 ms | 165 ms | 72 ms | 103 ms |
| nbody | 195 ms | 185 ms | 60 ms | 129 ms |
| binarytrees | 475 ms | 363 ms | 316 ms | 730 ms |
| closures | 144 ms | 232 ms | 64 ms | 49 ms |

Minnow runs at about clang -O0's speed, and clang -O2 is about twice as fast. On allocation
(`binarytrees`), Minnow is 1.3–1.5 times slower than `malloc` and `free`, and faster than Go.

**How long does building an executable take?**

| Program | Minnow | clang -O0 | clang -O2 | go build |
|---|---|---|---|---|
| fib | 37 ms | 93 ms | 92 ms | 198 ms |
| sieve | 40 ms | 101 ms | 106 ms | 187 ms |
| nbody | 61 ms | 97 ms | 147 ms | 180 ms |
| binarytrees | 38 ms | 93 ms | 122 ms | 197 ms |
| closures | 40 ms | 98 ms | 108 ms | 176 ms |

Minnow builds faster than clang and Go. Go's time is with its standard library already cached.

**What do A0's options cost?**

| Program | executable | executable, no HIR optimizer | JIT (`minnow run`) | build, no HIR optimizer |
|---|---|---|---|---|
| fib | 63 ms | 68 ms | 62 ms | 34 ms |
| sieve | 150 ms | 143 ms | 109 ms | 43 ms |
| nbody | 195 ms | 134 ms | 168 ms | 55 ms |
| binarytrees | 475 ms | 581 ms | 526 ms | 43 ms |
| closures | 144 ms | 782 ms | 198 ms | 42 ms |

The JIT's times include compiling, and are still close to the executable's: from 27% faster
(`sieve`) to 37% slower (`closures`). The HIR optimizer matters only on `closures`. The other four
programs have no small functions for it to inline, so their differences are noise.

### Why

- **Cranelift's code is like clang -O0's.** A0 leaves optimizing each function to Cranelift, its
  code generator. clang -O2 runs LLVM's whole pipeline: inlining, loop and vector optimizations,
  which Cranelift does not attempt. On `fib`, for example, -O2 turns one of the two recursive calls
  into a loop.
- **`closures`:** A0's optimizer inlines `fold`, and then the closure passed to it. That takes the
  program from 782 ms to 144 ms, between clang -O0 (232 ms) and -O2 (64 ms). clang -O2 also inlines
  the function pointer once it has inlined `fold`, and Go's 49 ms comes from the same inlining.
- **`sieve`:** the array holds one byte per boolean, as in C. What remains is Cranelift's code and
  a bounds check on every index, which Go has too. Tin, which has no check, takes 105 ms.
- **`nbody`:** the bodies are structs on the heap, reached through a bounds-checked array of
  references, and `math.sqrt` is a call to C's `sqrt` rather than an instruction.
- **`binarytrees`:** close to a third of A0's time is the collector. With `A0_GC_STATS=1` it
  reports 56 collections taking 152.5 ms, 82.8 ms of it marking, with a peak of 22.4 MB and 240,329
  objects live after the last collection. A0's collector is a non-moving mark and sweep, so every
  collection marks every live object, including the long-lived tree. A generational collector
  would avoid most of that work. Go is slower still (730 ms): its collector is built for short
  pauses beside a running program, not for the throughput of a small single-threaded one.

## Tin against C

Tin is A0's systems language: no collector, structs as values, raw pointers and C's allocator. Its
programs are written as the C ones are: `binarytrees` allocates with `alloc` and `free`, `sieve`
uses memory from `alloc` indexed without checks, and `nbody` keeps the bodies in one block of
memory reached through pointers (`bench/tin`). The Minnow column is the same program with a
collector, from the section above.

**How fast does the compiled code run?**

| Program | Tin (executable) | Minnow (executable) | clang -O0 | clang -O2 |
|---|---|---|---|---|
| fib | 68 ms | 63 ms | 62 ms | 37 ms |
| sieve | 105 ms | 150 ms | 166 ms | 84 ms |
| nbody | 152 ms | 195 ms | 184 ms | 63 ms |
| binarytrees | 488 ms | 475 ms | 402 ms | 325 ms |

Tin runs at about clang -O0's speed. `binarytrees` varied from 372 to 488 ms across runs of this
benchmark, around clang -O0's 402 ms.

**What do A0's options cost?**

| Program | executable | executable, no HIR optimizer | JIT (`tin run`) | build |
|---|---|---|---|---|
| fib | 68 ms | 72 ms | 64 ms | 42 ms |
| sieve | 105 ms | 106 ms | 119 ms | 45 ms |
| nbody | 152 ms | 122 ms | 100 ms | 55 ms |
| binarytrees | 488 ms | 387 ms | 809 ms | 39 ms |

### Why

Tin's programs use the same compiler, optimizer and code generator as Minnow's, without the
collector, the bounds checks and the references.

- **`binarytrees`:** Tin allocates with C's `calloc` and frees each tree as the C program does, so
  the collector's time is gone. In the JIT it takes 809 ms because `tin run` runs `main` on a
  thread of its own, and glibc's allocator is slower on a thread's memory arena than on the main
  one for this pattern of allocating and freeing.
- **`nbody`:** the five bodies are structs in one block of memory, reached through pointers, where
  Minnow goes through a checked array of references. The 122 ms without the HIR optimizer is noise
  of this run, not a real difference.
- **`sieve`:** bytes from `alloc`, indexed without checks, land between clang -O0 and -O2.
- **`fib`** is only calls, and runs as Minnow's does.

## Kaleidoscope against LLVM

Kaleidoscope is the language of LLVM's tutorial. A0's version is 498 lines: grammar, compiler,
driver and runtime, not counting blank lines and comments. The tutorial's chapter 7 is 901 lines
(`toy.cpp` and `KaleidoscopeJIT.h`).

| Program | What it does |
|---|---|
| `fib` | naive recursive Fibonacci, `fib(32)` |
| `mandel` | the tutorial's own Mandelbrot iteration over a 1000 × 1000 grid, summed. It uses the operators `|` and `>`, which the program defines itself, and a recursive function that iterates. |

A0 is compared with two builds of the tutorial's code, both JIT compilers:

- **LLVM tutorial:** chapter 7 as written. It compiles each definition in a module of its own and
  runs four passes on each function.
- **LLVM -O2:** the same code with LLVM's standard -O2 pipeline, run on one module that holds the
  whole program. `bench/run.py` patches `toy.cpp` to do this (`TOY_O2`).

**How fast does the compiled code run?**

| Program | A0 (executable) | LLVM tutorial | LLVM -O2 |
|---|---|---|---|
| fib | 19 ms | 9 ms | 9 ms |
| mandel | 916 ms | 1015 ms | 217 ms |

A0 is slightly faster than the tutorial's code on `mandel` and 4.2 times slower than LLVM -O2. LLVM's
times leave out its compile time (see [How each tool was run](#how-each-tool-was-run)); A0's include
starting the process.

**How long does compiling take?**

| Program | A0 build (an executable, linking included) | A0 build, no HIR optimizer | LLVM tutorial (start and compile) | LLVM -O2 (start and compile) |
|---|---|---|---|---|
| fib | 33 ms | 31 ms | 25 ms | 24 ms |
| mandel | 41 ms | 44 ms | 38 ms | 48 ms |

**What do A0's options cost?**

| Program | executable | executable, no HIR optimizer | JIT (`kaleidoscope run`) | interpreted |
|---|---|---|---|---|
| fib | 19 ms | 15 ms | 21 ms | 453 ms |
| mandel | 916 ms | 980 ms | 926 ms | 17.8 s |

### Why

The tutorial's chapter 7 (as of LLVM 18) runs four passes on each function: instcombine,
reassociate, GVN and simplifycfg. It has no inliner and no tail-call elimination. The chapter is
about mem2reg, but its code does not run it, so every argument and variable stays in memory on the
stack. So `mandel`'s inner loop calls the program's `|` and `>` operators on every iteration, and
recurses for every step.

A0's HIR optimizer inlines small functions like those operators. That makes `mandel` faster than
the tutorial's code; without the optimizer, A0 is about as fast (980 ms). `fib` has nothing to
inline.

LLVM's -O2 pipeline, given the whole program, does everything: it inlines the operators and
`mandelconverge`, turns the recursion into loops, keeps every variable in a register and
vectorizes. Its 217 ms shows what a full optimizing pipeline buys over Cranelift's per-function
optimization plus A0's inlining. On these small programs it compiles as fast as the tutorial's
code.

## Lox against clox

Lox is the dynamically typed language of the book Crafting Interpreters, and clox is the book's
bytecode interpreter in C. The programs are the benchmarks from the book's own test suite, except
two:

- `string_equality`: clox rejects it, because it needs more than 256 constants in one chunk.
- `zoo_batch`: it runs for ten seconds rather than doing a fixed amount of work.

**How fast does a program run?**

| Program | A0's Lox (executable) | clox | A0 against clox |
|---|---|---|---|
| fib | 496 ms | 1116 ms | 2.2 times faster |
| binary_trees | 994 ms | 2661 ms | 2.7 times faster |
| method_call | 214 ms | 242 ms | 1.1 times faster |
| invocation | 417 ms | 382 ms | 1.1 times slower |
| equality | 98 ms | 4777 ms | (see below) |
| zoo | 218 ms | 409 ms | 1.9 times faster |

`equality` compares constants and throws the results away. A0's optimizer removes the comparisons,
so its 98 ms is the loop alone, and the comparison with clox means nothing.

**What do A0's options cost?**

| Program | executable | executable, no HIR optimizer | JIT (`lox file`) | interpreted |
|---|---|---|---|---|
| fib | 496 ms | 2483 ms | 487 ms | 30.7 s |
| binary_trees | 994 ms | 3176 ms | 1310 ms | 49.4 s |
| method_call | 214 ms | 546 ms | 416 ms | 9821 ms |
| invocation | 417 ms | 886 ms | 573 ms | 17.3 s |
| equality | 98 ms | 1076 ms | 153 ms | 5935 ms |
| zoo | 218 ms | 1044 ms | 306 ms | 16.9 s |

The JIT time includes compiling, so it is the time to run a file as clox does (`lox file` against
`clox file`). It is up to about 300 ms slower than the executable (`fib` is as fast), because a
Lox program first compiles the functions of Lox's prelude that it reaches. Without the HIR
optimizer, Lox is 2–11 times slower.

**How long does building an executable take?**

| Program | A0's Lox | A0's Lox, no HIR optimizer |
|---|---|---|
| fib | 62 ms | 58 ms |
| binary_trees | 105 ms | 69 ms |
| method_call | 184 ms | 86 ms |
| invocation | 202 ms | 87 ms |
| equality | 75 ms | 70 ms |
| zoo | 115 ms | 73 ms |

### Why

A0's Lox compiles every Lox value to `any`, A0's type for a value of any type. Lox's semantics
(calls, classes, properties) are HIR functions in Lox's prelude, which the optimizer inlines and
specializes. That is why the optimizer matters more here than anywhere else.

- **`fib`:** once the prelude's call functions are inlined, arithmetic and comparisons are native
  code, and a call is a few type tests and an indirect call. clox dispatches bytecode for every
  operation.
- **Fields and methods** (`zoo`, `method_call`): an instance's fields and a class's methods are
  slots found through the object's shape. Every place in the program that names a field keeps a
  cache of the shape it saw last, so a property access is usually a comparison and a load. clox
  looks the name up in each instance's hash table.
- **Method calls** (`invocation`): A0 looks the method up and calls it on the object without
  allocating a bound method, as clox's `OP_INVOKE` does. This is the one program where clox is
  faster.
- **`binary_trees`:** an instance is a struct plus one object of slots. The collector takes 261.2 ms
  of the time: 144 collections, 150.4 ms of it marking, with a peak of 20.9 MB and 96,759 objects
  live after the last collection.
- **Interpreted**, A0 is 1.2–45 times slower than clox. A0's interpreter runs the core IR and is
  not built for speed: it is the reference that every test checks native code against, and the
  debugger's engine.

## Threads

`parallel` (`bench/minnow/parallel.mn`) counts the primes below 5,000,000. With `n` threads, each
thread tests every `n`-th odd number. The machine has 4 processors.

**How much faster do more threads make it?**

| Threads | executable | speedup | JIT (`minnow run`) | speedup |
|---|---|---|---|---|
| 1 | 2406 ms | 1.0x | 2416 ms | 1.0x |
| 2 | 1213 ms | 2.0x | 1230 ms | 2.0x |
| 4 | 626 ms | 3.8x | 636 ms | 3.8x |
| 8 | 618 ms | 3.9x | 626 ms | 3.9x |

The threads share nothing but the counts they return, so the speedup is the processors'; 8
threads on 4 processors gain nothing more. Work that allocates speeds up less, because threads that
allocate share one lock on the heap, and each collection stops them all.

## Compiling

**Where does the time of a build go?** Each of A0's steps below includes the ones before it. The
object file is made from the IR text with `a0 obj`.

| Program | A0: source to HIR | A0: to IR | A0: to an object file | whole build (linked by `cc`) | whole build (linked by clang) |
|---|---|---|---|---|---|
| fib | 4 ms | 4 ms | 4 ms | 33 ms | 63 ms |
| nbody | 11 ms | 17 ms | 13 ms | 54 ms | 79 ms |

**How long does linking alone take?** The same linkers, linking a plain C object file:

| Program | `cc` (gcc 13) | clang |
|---|---|---|
| fib | 27 ms | 53 ms |
| nbody | 32 ms | 48 ms |

A0's own work, from source to an object file, takes a few milliseconds: parsing, building and
checking the HIR, optimizing, lowering and Cranelift together. Most of a build is linking. A0 runs
the platform's C compiler driver (`cc`) on the object file and the runtime library, and the
driver takes most of that time. A whole build takes about as long as A0's work plus linking with
each linker. lld, run directly on a target pack, links in about 10 ms
([other platforms](cross.md)).

To run a program at once, A0 does not link at all: `run` compiles into memory. That is why the JIT
columns above are close to the executables'.

## Large programs

`bench/large/gen.py` makes a Minnow package of 175 modules and 100,273 lines. Each module imports
up to three earlier ones and calls into them. When the program runs, each module's `check` calls
the `check` of one module it imports and a function of the others, so it does little work: the
times below are A0's.

**How long does it take to build and run?**

| Build | `minnow run` (compiling included) | the executable |
|---|---|---|
| 1276 ms | 1302 ms | 2 ms |

**Where does the build's time go?** (`A0_TIME=1`)

| Step | Time |
|---|---|
| parse | 218 ms |
| analyze (build and check the HIR) | 391 ms |
| monomorphize | 0 ms |
| optimize (HIR) | 481 ms |
| lower to IR | 135 ms |
| verify IR | 9 ms |
| code generation | 89 ms |
| link | 38 ms |

**How fast does the language server answer?** `minnow lsp` on the same program:

| File opened | Program it belongs to | open to diagnostics | an edit to the next answer | hover | go to definition | completion |
|---|---|---|---|---|---|---|
| m87.mn | half the modules | 533 ms | 169 ms | 2 ms | 1 ms | 0 ms |
| main.mn | every module | 865 ms | 413 ms | 4 ms | 3 ms | 0 ms |

"An edit to the next answer" is the time from an edit to the answer of the next request, which
waits for the edited program to be analyzed again.

## From release to release

**Is v1.0.0 as fast as v0.9.0?** Both were built from their tags and run side by side on the same
machine and the same `bench/`, alternating, two rounds. Medians of 7 runs, and of 5 for the large
program.

| | v0.9.0 | v1.0.0 |
|---|---|---|
| Minnow `nbody`, executable, first round | 179 ms | 170 ms |
| Minnow `nbody`, executable, second round | 125 ms | 132 ms |
| Tin `binarytrees`, executable, first round | 408 ms | 372 ms |
| Tin `binarytrees`, executable, second round | 378 ms | 480 ms |
| Large program, build | 1236 ms | 1421 ms |
| Large program, language server: open `main.mn` to diagnostics | 823 ms | 868 ms |
| Large program, language server: an edit to the next answer | 377 ms | 408 ms |

Yes. v1.0 changed APIs, the headers of text formats, documentation and tooling, not code
generation, the optimizer or the collector. The two releases differ by less than one release
differs from itself between rounds. The full run in the sections above has outliers of the same
size (`nbody` at 195 ms, Tin's `binarytrees` at 488).

**How have the headline numbers changed?** `bench/history.csv` keeps each release's numbers
(`bench/run.py --record`; see [releases](releases.md)), and `python3 bench/history.py` prints them
by machine. On this machine, in milliseconds:

| | 0.8.0 | 0.9.0 | 1.0.0 |
|---|---|---|---|
| kaleidoscope fib, executable | 20 | 16 | 19 |
| kaleidoscope mandel, executable | 954 | 945 | 916 |
| minnow fib, executable | 64 | 71 | 63 |
| minnow sieve, executable | 138 | 144 | 150 |
| minnow nbody, executable | 143 | 139 | 195 |
| minnow binarytrees, executable | 505 | 462 | 475 |
| minnow closures, executable | 823 | 141 | 144 |
| tin fib, executable | 77 | 78 | 68 |
| tin sieve, executable | 123 | 98 | 105 |
| tin nbody, executable | 87 | 153 | 152 |
| tin binarytrees, executable | 388 | 384 | 488 |
| parallel, 1 thread | 2432 | 2427 | 2406 |
| parallel, 2 threads | 1261 | 1246 | 1213 |
| parallel, 4 threads | 684 | 644 | 626 |
| parallel, 8 threads | 633 | 630 | 618 |
| lox fib, executable | 554 | 468 | 496 |
| clox fib | 1327 | 1165 | 1116 |
| lox binary_trees, executable | 1363 | 1191 | 994 |
| clox binary_trees | 3569 | 3169 | 2661 |
| lox method_call, executable | 195 | 172 | 214 |
| clox method_call | 245 | 220 | 242 |
| lox invocation, executable | 477 | 408 | 417 |
| clox invocation | 423 | 329 | 382 |
| lox equality, executable | 101 | 106 | 98 |
| clox equality | 4737 | 4844 | 4777 |
| lox zoo, executable | 260 | 231 | 218 |
| clox zoo | 431 | 364 | 409 |
| old large program, build | 4775 | 2723 | |
| old large program, language server: an edit to the next answer | 1973 | 370 | |
| large program, build | | 1236 | 1276 |
| large program, language server: an edit to the next answer | | 377 | 413 |

`minnow closures` dropped from 823 to 141 ms in 0.9.0, when A0's optimizer learned to inline
closures. The large program changed after 0.9.0: until then, every module's `check` called the
`check` of every module it imports, so the calls grew exponentially with the number of modules.
Its old numbers ("old large program") are kept apart, and 0.9.0 was measured with both.

## Tools

- Machine: x86_64, Linux 6.18 (a cloud virtual machine), 4 threads
- CPU: Intel(R) Xeon(R) Processor @ 2.10GHz
- A0: v1.0.0, rustc 1.97.0 (2d8144b78 2026-07-07), Cranelift 0.137, release build
- Ubuntu clang version 18.1.3 (1ubuntu1)
- `cc`, which A0 links with: cc (Ubuntu 13.3.0-6ubuntu2~24.04.1) 13.3.0
- LLVM 18.1.3: the Kaleidoscope tutorial, chapter 7, built with `clang++ -O2`, and the same with
  LLVM's -O2 pipeline on the whole program
- go version go1.24.7 linux/amd64
- clox at craftinginterpreters 4a840f70f69c, built with `clang -O2 -std=c99 -w`

### How each tool was run

| Tool | Compile | Run |
|---|---|---|
| A0's languages | `minnow build prog.mn -o prog` (likewise `tin`, `lox`, `kaleidoscope`); `A0_OPT=0` for no HIR optimizer | `./prog`. JIT: `minnow run prog.mn`, compiling included. Interpreted: the same with `A0_INTERP=1` |
| C | `clang -O0 prog.c -lm -o prog`, and the same with `-O2` | `./prog` |
| Go | `go build -o prog prog.go`, with the standard library cached | `./prog` |
| clox | | `clox prog.lox` |
| LLVM's Kaleidoscope | the program with the calls on its last line doing no work (`printd(fib(1))`), run through `toy` | `toy < prog.k`, less the compile time |

LLVM's JIT compiles each function when a call first needs it. Its compile time is therefore
measured by running the same program with the last line's calls doing no work, and the rest of the
full run counts as running.

## What these numbers do not say

- They come from one shared cloud machine and from small programs written the obvious way.
  Differences of 10% between runs, and in linking times, are noise.
- A0 is shown with its optimizer on and off. The other tools are run as they usually are: Go
  builds with its standard library cached, and LLVM's tutorial is a JIT.
- Every implementation of a program must print the same output, and the benchmark checks it before
  timing:
  - The times Lox's programs print are ignored.
  - Numbers are compared as clox prints them, with `%g`. A0's Lox prints `zoo`'s sum as 10000002,
    where clox prints `1e+07`.
- Native debug information (`-g`) and the debugger are not measured.

## Reproduce

On Linux or macOS, with clang, LLVM's development files and Go installed (`bench/README.md` lists
the packages):

```
python3 bench/run.py --runs 5
```

It takes about ten minutes and writes its measurements to `target/bench/results.md`, in wider
tables than this page's: one row per program, with every measurement in it.
