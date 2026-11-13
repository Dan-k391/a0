# Benchmarks

This page answers three questions:

1. **How fast does code compiled by A0 run**, compared with established tools: C (clang), Go,
   LLVM and clox?
2. **How long does A0 take to compile** a program?
3. **What do A0's own options cost**: running with the JIT, running in the interpreter, and turning
   A0's optimizer off?

The programs are small, written the obvious way, and written the same way in each language.
Nothing was tuned for this page, and it is not a CI check. The numbers are for A0's unreleased
code after v1.0.0 (commit 5a98dad), which adds a pass of optimizations
([the changelog](../CHANGELOG.md) lists them).

## How to read the tables

- **Times are in milliseconds** (`ms`), or seconds (`s`) where marked. **Lower is better.**
- Each time is the median of 5 runs. It is wall-clock time, so it includes starting the process
  (about 1 ms). Compile times include linking.
- **Each table answers one question**, written above it. A0's columns come first; the columns
  after them are what A0 is compared with. The sentence under the table says what it shows.
- The machine is a laptop with a 16-core processor (32 hardware threads), running Linux under WSL2
  on Windows. Differences under about 10% are noise.
- [From release to release](#from-release-to-release) was measured on another machine. Compare
  its numbers only with each other, not with the tables above it.

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
| Minnow (typed, with a collector) | C and Go | Faster than clang -O0 on three of five programs, 1.2–2.3 times slower than clang -O2, and as fast as Go or faster on three of five |
| Tin (no collector) | C | Faster than clang -O0 on `sieve` and `nbody`, as fast as clang -O2 on `nbody`, 1.2 times slower than -O0 on `fib` and within noise of it on `binarytrees` |
| Kaleidoscope | LLVM's tutorial | 1.6 times faster than the tutorial's code on `mandel` (241 against 391 ms), 2.2 times slower than LLVM's full -O2 |
| Lox (dynamic) | clox | Faster on the five programs that compare, from 1.1 times (`invocation`, close to noise) to 4.1 times (`binary_trees`); `equality` does not compare |
| Minnow with threads | itself on one thread | 4.0 times faster on 4 threads, 7.3 times on 8 |
| A large program | | 100,000 lines in 175 modules build in 0.4 s; the language server answers after an edit in about 0.15 s |
| Compiling | clang and Go | A0's own work takes under 10 ms on these programs; most of a build is the linker |

Every table above [From release to release](#from-release-to-release) measures the code after
v1.0.0 on this laptop. That section compares releases 0.8 to 1.0, which were measured earlier on
another machine, so its numbers cannot be compared with these. [The changelog](../CHANGELOG.md)
gives the before and after of this pass of optimizations, measured on Windows.

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
| fib | 36 ms | 28 ms | 16 ms | 28 ms |
| sieve | 32 ms | 43 ms | 26 ms | 32 ms |
| nbody | 45 ms | 79 ms | 36 ms | 53 ms |
| binarytrees | 156 ms | 136 ms | 116 ms | 353 ms |
| closures | 40 ms | 115 ms | 21 ms | 27 ms |

Minnow is faster than clang -O0 on `sieve`, `nbody` and `closures`, and slower on `fib` and
`binarytrees`. clang -O2 is 1.2–1.3 times faster than Minnow on `sieve`, `nbody` and
`binarytrees`, and about twice as fast on `fib` and `closures`. On allocation (`binarytrees`),
Minnow is 1.1–1.3 times slower than `malloc` and `free`, and 2.3 times faster than Go.

**How long does building an executable take?**

| Program | Minnow | clang -O0 | clang -O2 | go build |
|---|---|---|---|---|
| fib | 131 ms | 98 ms | 104 ms | 183 ms |
| sieve | 140 ms | 101 ms | 104 ms | 176 ms |
| nbody | 145 ms | 101 ms | 118 ms | 149 ms |
| binarytrees | 144 ms | 107 ms | 111 ms | 192 ms |
| closures | 141 ms | 97 ms | 101 ms | 158 ms |

Minnow builds faster than Go and slower than clang. Almost all of a Minnow build is linking with
`cc`, which on this machine takes about 130 ms even for a plain C object; clang links in about
80 ms ([Compiling](#compiling)). Go's time is with its standard library already cached.

**What do A0's options cost?**

| Program | executable | executable, no HIR optimizer | JIT (`minnow run`) | build, no HIR optimizer |
|---|---|---|---|---|
| fib | 36 ms | 36 ms | 32 ms | 133 ms |
| sieve | 32 ms | 31 ms | 34 ms | 134 ms |
| nbody | 45 ms | 44 ms | 50 ms | 142 ms |
| binarytrees | 156 ms | 160 ms | 168 ms | 154 ms |
| closures | 40 ms | 108 ms | 35 ms | 140 ms |

The JIT's times include compiling, and are still close to the executable's: from 12% faster
(`closures`) to 11% slower (`nbody`). The HIR optimizer matters only on `closures`. The other four
programs have no small functions for it to inline, so their differences are noise.

### Why

- **On loops, Cranelift's code lies between clang -O0's and -O2's.** A0 leaves optimizing each
  function to Cranelift, its code generator. clang -O0 keeps every variable in memory on the
  stack, where Cranelift keeps them in registers, so Minnow is faster than -O0 where a loop does
  the work. On `fib`, which is only calls, it is 1.3 times slower than -O0; we have not found why.
  clang -O2 runs LLVM's whole pipeline: inlining, loop and vector optimizations, which Cranelift
  does not attempt. On `fib`, for example, -O2 turns one of the two recursive calls into a loop.
- **`closures`:** A0's optimizer inlines `fold`, and then the closure passed to it. That takes the
  program from 108 ms to 40 ms, between clang -O0 (115 ms) and -O2 (21 ms). clang -O2 also inlines
  the function pointer once it has inlined `fold`, and Go's 27 ms comes from the same inlining.
- **`sieve`:** the array holds one byte per boolean, as in C. Every index is bounds-checked, as in
  Go. A failed check ends the program, so the check is a compare and a branch out of the loop that
  is never taken. Minnow runs as fast as Go, and within noise of Tin, which has no check (36 ms).
- **`nbody`:** the bodies are structs on the heap, reached through a bounds-checked array of
  references. `math.sqrt` is an instruction, not a call to C's `sqrt`. The bodies' float fields
  are loaded and stored as floats, without a move through an integer register. Minnow runs it 1.8
  times faster than clang -O0 and 1.25 times slower than -O2.
- **`binarytrees`:** over a quarter of A0's time is the collector. With `A0_GC_STATS=1` it reports
  56 collections taking 42.4 ms, 29.2 ms of it marking, with a peak of 22.4 MB and 240,329 objects
  live after the last collection. A0's collector is a non-moving mark and sweep, so every
  collection marks every live object, including the long-lived tree. A generational collector
  would avoid most of that work. Go is slower (353 ms): its collector is built for short pauses
  beside a running program, not for the throughput of a small single-threaded one.

## Tin against C

Tin is A0's systems language: no collector, structs as values, raw pointers and C's allocator. Its
programs are written as the C ones are: `binarytrees` allocates with `alloc` and `free`, `sieve`
uses memory from `alloc` indexed without checks, and `nbody` keeps the bodies in one block of
memory reached through pointers (`bench/tin`). The Minnow column is the same program with a
collector, from the section above.

**How fast does the compiled code run?**

| Program | Tin (executable) | Minnow (executable) | clang -O0 | clang -O2 |
|---|---|---|---|---|
| fib | 36 ms | 36 ms | 29 ms | 16 ms |
| sieve | 36 ms | 32 ms | 43 ms | 25 ms |
| nbody | 37 ms | 45 ms | 78 ms | 36 ms |
| binarytrees | 151 ms | 156 ms | 138 ms | 122 ms |

Tin is faster than clang -O0 on `sieve` and `nbody`, and as fast as -O2 on `nbody`. It is 1.2
times slower than -O0 on `fib`, and within noise of it on `binarytrees`.

**What do A0's options cost?**

| Program | executable | executable, no HIR optimizer | JIT (`tin run`) | build |
|---|---|---|---|---|
| fib | 36 ms | 32 ms | 33 ms | 135 ms |
| sieve | 36 ms | 39 ms | 36 ms | 130 ms |
| nbody | 37 ms | 36 ms | 43 ms | 136 ms |
| binarytrees | 151 ms | 153 ms | 371 ms | 144 ms |

### Why

Tin's programs use the same compiler, optimizer and code generator as Minnow's, without the
collector, the bounds checks and the references.

- **`binarytrees`:** Tin allocates with C's `calloc` and frees each tree as the C program does, so
  the collector's time is gone. In the JIT it takes 371 ms because `tin run` runs `main` on a
  thread of its own, and glibc's allocator is slower on a thread's memory arena than on the main
  one for this pattern of allocating and freeing.
- **`nbody`:** the five bodies are structs in one block of memory, reached through pointers, where
  Minnow goes through a checked array of references. Their float fields are loaded and stored as
  floats, as in Minnow, and Tin runs as fast as clang -O2.
- **`sieve`:** bytes from `alloc`, indexed without checks, land between clang -O0 and -O2. Minnow's
  checked array is as fast (32 ms).
- **`fib`** is only calls, and runs as Minnow's does: 1.2 times slower than clang -O0.

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
| fib | 6 ms | 4 ms | 4 ms |
| mandel | 241 ms | 391 ms | 108 ms |

A0 is 1.6 times faster than the tutorial's code on `mandel` and 2.2 times slower than LLVM -O2.
LLVM's times leave out its compile time (see [How each tool was run](#how-each-tool-was-run)); A0's
include starting the process.

**How long does compiling take?**

| Program | A0 build (an executable, linking included) | A0 build, no HIR optimizer | LLVM tutorial (start and compile) | LLVM -O2 (start and compile) |
|---|---|---|---|---|
| fib | 134 ms | 134 ms | 10 ms | 9 ms |
| mandel | 133 ms | 135 ms | 16 ms | 18 ms |

A0's build is almost all linking ([Compiling](#compiling)). LLVM's JIT does not link.

**What do A0's options cost?**

| Program | executable | executable, no HIR optimizer | JIT (`kaleidoscope run`) | interpreted |
|---|---|---|---|---|
| fib | 6 ms | 6 ms | 8 ms | 271 ms |
| mandel | 241 ms | 383 ms | 173 ms | 10.4 s |

On `mandel`, A0's JIT (173 ms, compiling included) is faster than its executable (241 ms); the
next section says what we know of why.

### Why

The tutorial's chapter 7 (as of LLVM 18) runs four passes on each function: instcombine,
reassociate, GVN and simplifycfg. It has no inliner and no tail-call elimination. The chapter is
about mem2reg, but its code does not run it, so every argument and variable stays in memory on the
stack. So `mandel`'s inner loop calls the program's `|` and `>` operators on every iteration, and
recurses for every step.

A0's HIR optimizer inlines small functions like those operators. That makes `mandel` faster than
the tutorial's code; without the optimizer, A0 is about as fast (383 ms). `fib` has nothing to
inline.

LLVM's -O2 pipeline, given the whole program, does everything: it inlines the operators and
`mandelconverge`, turns the recursion into loops, keeps every variable in a register and
vectorizes. Its 108 ms shows what a full optimizing pipeline buys over Cranelift's per-function
optimization plus A0's inlining. On these small programs it compiles as fast as the tutorial's
code.

A0's JIT runs `mandel` faster than its executable, and that holds up: five more runs of each gave
239–244 ms for the executable and 176–200 ms for the JIT. Both get their code from Cranelift and
the same optimizer; the one difference the changelog names is that executables call the runtime,
the C library and other codegen units directly, where the JIT loads each address first. We have
not found why the executable is slower.

## Lox against clox

Lox is the dynamically typed language of the book Crafting Interpreters, and clox is the book's
bytecode interpreter in C. The programs are the benchmarks from the book's own test suite, except
two:

- `string_equality`: clox rejects it, because it needs more than 256 constants in one chunk.
- `zoo_batch`: it runs for ten seconds rather than doing a fixed amount of work.

**How fast does a program run?**

| Program | A0's Lox (executable) | clox | A0 against clox |
|---|---|---|---|
| fib | 143 ms | 492 ms | 3.4 times faster |
| binary_trees | 305 ms | 1249 ms | 4.1 times faster |
| method_call | 65 ms | 109 ms | 1.7 times faster |
| invocation | 134 ms | 151 ms | 1.1 times faster |
| equality | 65 ms | 2436 ms | (see below) |
| zoo | 95 ms | 160 ms | 1.7 times faster |

`equality` compares constants and throws the results away. A0's optimizer removes the comparisons,
so its 65 ms is the loop alone, and the comparison with clox means nothing.

**What do A0's options cost?**

| Program | executable | executable, no HIR optimizer | JIT (`lox file`) | interpreted |
|---|---|---|---|---|
| fib | 143 ms | 762 ms | 164 ms | 10.8 s |
| binary_trees | 305 ms | 853 ms | 356 ms | 18.9 s |
| method_call | 65 ms | 170 ms | 116 ms | 4383 ms |
| invocation | 134 ms | 269 ms | 176 ms | 7376 ms |
| equality | 65 ms | 288 ms | 81 ms | 1787 ms |
| zoo | 95 ms | 302 ms | 125 ms | 7625 ms |

The JIT time includes compiling, so it is the time to run a file as clox does (`lox file` against
`clox file`). It is 16–51 ms slower than the executable, because a Lox program first compiles the
functions of Lox's prelude that it reaches. Without the HIR optimizer, Lox is 2–5 times slower.

**How long does building an executable take?**

| Program | A0's Lox | A0's Lox, no HIR optimizer |
|---|---|---|
| fib | 148 ms | 147 ms |
| binary_trees | 170 ms | 167 ms |
| method_call | 184 ms | 160 ms |
| invocation | 171 ms | 146 ms |
| equality | 141 ms | 139 ms |
| zoo | 160 ms | 147 ms |

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
  allocating a bound method, as clox's `OP_INVOKE` does. This is the program where clox comes
  closest: A0 is 1.1 times faster.
- **`binary_trees`:** an instance is a struct plus one object of slots. Calling a class whose
  `init` takes up to three parameters builds the instance without first putting the arguments in
  an array. The collector takes 63.6 ms of the time: 92 collections, 44.6 ms of it marking, with a
  peak of 19.8 MB and 93,328 objects live after the last collection.
- **Interpreted**, A0 is 15–49 times slower than clox. On `equality` it is faster, because the
  optimizer has removed the comparisons. A0's interpreter runs the core IR and is not built for
  speed: it is the reference that every test checks native code against, and the debugger's
  engine.

## Threads

`parallel` (`bench/minnow/parallel.mn`) counts the primes below 5,000,000. With `n` threads, each
thread tests every `n`-th odd number. The machine has 32 hardware threads.

**How much faster do more threads make it?**

| Threads | executable | speedup | JIT (`minnow run`) | speedup |
|---|---|---|---|---|
| 1 | 903 ms | 1.0x | 867 ms | 1.0x |
| 2 | 432 ms | 2.1x | 502 ms | 1.7x |
| 4 | 226 ms | 4.0x | 247 ms | 3.5x |
| 8 | 124 ms | 7.3x | 145 ms | 6.0x |

The threads share nothing but the counts they return, so the executable speeds up almost as much
as it has threads: 7.3 times on 8. The JIT speeds up less, 6.0 times on 8: it is faster than the
executable on one thread and slower on two or more, and we have not found why. The benchmark goes
up to 8 threads, a quarter of the machine's. Work that allocates
speeds up less, because threads that allocate share one lock on the heap, and each collection
stops them all.

## Compiling

**Where does the time of a build go?** Each of A0's steps below includes the ones before it. The
object file is made from the IR text with `a0 obj`.

| Program | A0: source to HIR | A0: to IR | A0: to an object file | whole build (linked by `cc`) | whole build (linked by clang) |
|---|---|---|---|---|---|
| fib | 2 ms | 2 ms | 2 ms | 134 ms | 97 ms |
| nbody | 4 ms | 5 ms | 6 ms | 141 ms | 91 ms |

**How long does linking alone take?** The same linkers, linking a plain C object file:

| Program | `cc` (gcc 13) | clang |
|---|---|---|
| fib | 132 ms | 81 ms |
| nbody | 129 ms | 81 ms |

A0's own work, from source to an object file, takes a few milliseconds: parsing, building and
checking the HIR, optimizing, lowering and Cranelift together. Most of a build is linking. A0 runs
the platform's C compiler driver (`cc`) on the object file and the runtime library, and the
driver takes most of that time. A whole build takes about as long as A0's work plus linking with
each linker. lld, run directly on a target pack, links in about 10 ms
([other platforms](cross.md)).

To run a program at once, A0 does not link at all: `run` compiles into memory in a few
milliseconds. That is why the JIT's times, compiling included, are near the executables' run times
rather than their build times. Where they differ by more, the sections above say why.

## Large programs

`bench/large/gen.py` makes a Minnow package of 175 modules and 100,273 lines. Each module imports
up to three earlier ones and calls into them. When the program runs, each module's `check` calls
the `check` of one module it imports and a function of the others, so it does little work: the
times below are A0's.

**How long does it take to build and run?**

| Build | `minnow run` (compiling included) | the executable |
|---|---|---|
| 404 ms | 304 ms | 1 ms |

**Where does the build's time go?** (`A0_TIME=1`)

| Step | Time |
|---|---|
| parse | 59 ms |
| analyze (build and check the HIR) | 114 ms |
| monomorphize | 0 ms |
| optimize (HIR) | 18 ms |
| lower to IR | 22 ms |
| verify IR | 3 ms |
| code generation | 29 ms |
| link | 148 ms |

The optimizer works only on the functions `main` can reach, so it takes little time here. Linking
takes the longest, as in the small programs. `minnow run` does not link, so it is faster than the
build.

**How fast does the language server answer?** `minnow lsp` on the same program:

| File opened | Program it belongs to | open to diagnostics | an edit to the next answer | hover | go to definition | completion |
|---|---|---|---|---|---|---|
| m87.mn | half the modules | 324 ms | 74 ms | 0 ms | 0 ms | 0 ms |
| main.mn | every module | 455 ms | 153 ms | 1 ms | 1 ms | 0 ms |

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
differs from itself between rounds. The full v1.0.0 run on that machine had outliers of the same
size (`nbody` at 195 ms, Tin's `binarytrees` at 488). The tables above are newer than this
comparison: they measure the code after v1.0.0 on a different machine. That code is faster than
v1.0.0; [the changelog](../CHANGELOG.md) has before-and-after times measured on Windows.

**How have the headline numbers changed?** `bench/history.csv` keeps each release's numbers
(`bench/run.py --record`; see [releases](releases.md)), and `python3 bench/history.py` prints them
by machine. On the machine the releases were measured on, in milliseconds:

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

- Machine: x86_64, Linux 6.6 (WSL2 on Windows), 32 threads
- CPU: AMD Ryzen 9 9955HX3D 16-Core Processor
- A0: unreleased, commit 5a98dad (after v1.0.0), rustc 1.99.1 (58cf8f9c3 2026-10-13), Cranelift 0.137, release build
- Ubuntu clang version 18.1.3 (1ubuntu1)
- `cc`, which A0 links with: cc (Ubuntu 13.3.0-6ubuntu2~24.04.1) 13.3.0
- LLVM 18.1.3: the Kaleidoscope tutorial, chapter 7, built with `clang++ -O2`, and the same with
  LLVM's -O2 pipeline on the whole program
- go version go1.22.2 linux/amd64
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

- They come from one laptop running Linux under WSL2, and from small programs written the obvious
  way. Differences of 10% between runs are noise.
- Linking is slow on this machine: `cc` takes about 130 ms even for a plain C object, so it is
  most of every build time here. lld, run directly on a target pack, links in about 10 ms
  ([Compiling](#compiling)).
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
