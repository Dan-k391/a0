# Benchmarks

A small, honest picture of where A0 stands against established tools: how long its languages take
to compile, how fast what they compile runs, and how fast `lang run` runs a program at once. It is
not a CI check and nothing was tuned for it: the programs are the obvious ones, written the same
way in each language. `bench/run.py` reproduces everything here (`bench/README.md`); the tables
below are its output for v1.0.0. "From v0.9.0 to v1.0.0" compares with v0.9.0 run on the same
machine, a shared 4-processor cloud machine that is slower and noisier than the one earlier
releases were measured on (whose numbers are in this page's history), so compare within this page
only.

## In short

- **v1.0 runs as v0.9 does.** Nothing in v1.0 changes the code A0 generates or how fast it
  compiles; measured side by side, the two releases differ by this machine's noise (below).
- **Closures are inlined (v0.9).** A closure passed to a function that is inlined, as in
  `fold(xs, 0, fn(a, b) { ... })`, is inlined too (D84): `closures` runs in 144 ms instead of 782
  without the HIR optimizer, between clang -O0 (232 ms) and -O2 (64 ms).
- **Large programs.** A generated program of 100,000 lines in 175 modules builds in 1.3 s, and the
  language server answers after an edit in about 0.4 s (D82, below).
- **Threads run at the same time.** Counting the primes below 5,000,000 takes 2.4 s on one thread
  and 1.2 s and 0.63 s on 2 and 4 (3.8 times faster on this 4-processor machine).
- **Tin, the systems language, runs close to C.** Without a collector, its `binarytrees` takes
  372–488 ms across runs against clang -O0's 402 (Minnow, with its collector: 475).
- **Typed code (Minnow) against C and Go.** On calls, arrays and floating point A0 runs at about
  clang -O0's speed and close to Go, about half as fast as clang -O2. On allocation it is 1.3–1.5
  times slower than malloc and free and faster than Go.
- **Kaleidoscope against LLVM.** A0's Kaleidoscope is 498 lines against the tutorial's 901 and runs
  `mandel` slightly faster than the tutorial's passes (916 against 1015 ms) and 4 times slower than
  LLVM's whole -O2 pipeline.
- **Lox against clox.** Compiled with A0, Lox is faster than clox on five of the six benchmarks
  (2.2 times on `fib`, 2.7 times on `binary_trees`, 1.1 and 1.9 times on `method_call` and `zoo`)
  and slower on `invocation` (417 against 382 ms).
- **Compiling.** A0's own work, from source to object file, is 4–20 ms on these programs; the rest
  of a build is the linker.

## Tools

- Machine: x86_64, Linux 6.18 (a cloud virtual machine), 4 threads
- CPU: Intel(R) Xeon(R) Processor @ 2.10GHz
- A0: v1.0.0, rustc 1.97.0 (2d8144b78 2026-07-07), Cranelift 0.137, release build
- Ubuntu clang version 18.1.3 (1ubuntu1)
- `cc`, which A0 links with: cc (Ubuntu 13.3.0-6ubuntu2~24.04.1) 13.3.0
- LLVM 18.1.3: the Kaleidoscope tutorial, chapter 7, built with `clang++ -O2`; and the same with
  LLVM's -O2 pipeline on the whole program (below)
- go version go1.24.7 linux/amd64
- clox at craftinginterpreters 4a840f70f69c, built with `clang -O2 -std=c99 -w`
- Times are medians of 5 runs (wall clock, so each includes starting a process, about 1 ms);
  compile times include linking.

How each tool was run:

| | Compile | Run |
|---|---|---|
| A0's languages | `minnow build prog.mn -o prog` (likewise `lox`, `kaleidoscope`); `A0_OPT=0` for no HIR optimizer | `./prog`; JIT: `minnow run prog.mn` (compiling included); interpreted: the same with `A0_INTERP=1` |
| C | `clang -O0 prog.c -lm -o prog`, `clang -O2 ...` | `./prog` |
| Go | `go build -o prog prog.go`, the standard library cached | `./prog` |
| clox | | `clox prog.lox` |
| LLVM's Kaleidoscope | `toy < prog.k`, the same program with its calls doing no work (below) | `toy < prog.k`, less the compile time |

## Kaleidoscope: A0 against LLVM

Lines of code (without blank lines and comments):
- A0's Kaleidoscope: 498 (grammar, compiler, driver, runtime).
- LLVM's chapter 7: 901 (`toy.cpp` and `KaleidoscopeJIT.h`).

| Program | A0 build | no HIR optimizer | A0 native run | no HIR optimizer | A0 JIT (`run`) | A0 interpreted | LLVM JIT: startup and compile | LLVM JIT: run | LLVM -O2: startup and compile | LLVM -O2: run |
|---|---|---|---|---|---|---|---|---|---|---|
| fib | 33 ms | 31 ms | 19 ms | 15 ms | 21 ms | 453 ms | 25 ms | 9 ms | 24 ms | 9 ms |
| mandel | 41 ms | 44 ms | 916 ms | 980 ms | 926 ms | 17.8 s | 38 ms | 1015 ms | 48 ms | 217 ms |

`fib` is naive recursive Fibonacci (`fib(32)`). `mandel` is the tutorial's own Mandelbrot iteration
over a 1000 × 1000 grid, summed. It uses the operators `|` and `>` that the program defines itself,
and a recursive function that iterates.

"LLVM -O2" is the tutorial's code with its four function passes replaced by LLVM's standard -O2
pipeline (`PassBuilder::buildPerModuleDefaultPipeline`), run on one module that holds the whole
program. The tutorial compiles each definition in a module of its own, which would hide every
callee from the inliner; `bench/run.py` patches `toy.cpp` (`TOY_O2`).

## Minnow against C and Go

| Program | Minnow build | no HIR optimizer | clang -O0 | clang -O2 | go build | Minnow run | no HIR optimizer | Minnow JIT (`run`) | clang -O0 | clang -O2 | Go |
|---|---|---|---|---|---|---|---|---|---|---|---|
| fib | 37 ms | 34 ms | 93 ms | 92 ms | 198 ms | 63 ms | 68 ms | 62 ms | 76 ms | 41 ms | 57 ms |
| sieve | 40 ms | 43 ms | 101 ms | 106 ms | 187 ms | 150 ms | 143 ms | 109 ms | 165 ms | 72 ms | 103 ms |
| nbody | 61 ms | 55 ms | 97 ms | 147 ms | 180 ms | 195 ms | 134 ms | 168 ms | 185 ms | 60 ms | 129 ms |
| binarytrees | 38 ms | 43 ms | 93 ms | 122 ms | 197 ms | 475 ms | 581 ms | 526 ms | 363 ms | 316 ms | 730 ms |
| closures | 40 ms | 42 ms | 98 ms | 108 ms | 176 ms | 144 ms | 782 ms | 198 ms | 232 ms | 64 ms | 49 ms |

The programs:
- `fib` is calls (`fib(35)`).
- `sieve` is arrays: the primes below 2,000,000, ten times.
- `nbody` is floating point and structs: the Benchmarks Game's n-body, 1,000,000 steps.
- `binarytrees` is allocation: the Benchmarks Game's binary-trees at depth 16. The C version uses
  malloc and free.
- `closures` is a closure that captures a variable passed to a small higher-order function,
  `fold`, 100,000 times over 1,000 numbers; the C version passes a function pointer and its
  environment.

A0's collector in `binarytrees` (`A0_GC_STATS=1`): 56 collections, 152.5 ms (82.8 marking), peak
22.4 MB, 240329 objects live after the last.

## Tin against C

Tin (`examples/tin`) is A0's systems language: no collector, structs as values, raw pointers, C's
allocator. Its programs are written as the C ones are: `binarytrees` with `alloc` and `free`,
`sieve` in memory from `alloc` indexed without checks, `nbody` with the bodies in one block of
memory reached through pointers (`bench/tin`).

| Program | Tin build | Tin run | no HIR optimizer | Tin JIT (`run`) | clang -O0 | clang -O2 |
|---|---|---|---|---|---|---|
| fib | 42 ms | 68 ms | 72 ms | 64 ms | 62 ms | 37 ms |
| sieve | 45 ms | 105 ms | 106 ms | 119 ms | 166 ms | 84 ms |
| nbody | 55 ms | 152 ms | 122 ms | 100 ms | 184 ms | 63 ms |
| binarytrees | 39 ms | 488 ms | 387 ms | 809 ms | 402 ms | 325 ms |

## Threads

`parallel` (`bench/minnow/parallel.mn`) counts the primes below 5,000,000, each of `n` threads
testing every `n`-th odd number, on this machine's 4 processors:

| Threads | Minnow native | speedup | Minnow JIT (`run`) | speedup |
|---|---|---|---|---|
| 1 | 2406 ms | 1.0x | 2416 ms | 1.0x |
| 2 | 1213 ms | 2.0x | 1230 ms | 2.0x |
| 4 | 626 ms | 3.8x | 636 ms | 3.8x |
| 8 | 618 ms | 3.9x | 626 ms | 3.9x |

The threads share nothing but the counts they return, so the speedup is the processors'. Threads
that allocate share the heap lock and collections stop them all (D76): allocation-heavy work
speeds up less.

Where Minnow's build time goes (`cc` is x86_64-linux-gnu-gcc-13). Each of A0's steps includes the
ones before it; the object file is made from the IR text with `a0 obj`.

| Program | to HIR | to IR | object file | executable | executable, `CC=clang` | cc linking a C object | clang linking it |
|---|---|---|---|---|---|---|---|
| fib | 4 ms | 4 ms | 4 ms | 33 ms | 63 ms | 27 ms | 53 ms |
| nbody | 11 ms | 17 ms | 13 ms | 54 ms | 79 ms | 32 ms | 48 ms |

## Lox: A0 against clox

| Program | A0 build | no HIR optimizer | A0 native run | no HIR optimizer | A0 JIT (`lox file`) | A0 interpreted | clox |
|---|---|---|---|---|---|---|---|
| fib | 62 ms | 58 ms | 496 ms | 2483 ms | 487 ms | 30.7 s | 1116 ms |
| binary_trees | 105 ms | 69 ms | 994 ms | 3176 ms | 1310 ms | 49.4 s | 2661 ms |
| method_call | 184 ms | 86 ms | 214 ms | 546 ms | 416 ms | 9821 ms | 242 ms |
| invocation | 202 ms | 87 ms | 417 ms | 886 ms | 573 ms | 17.3 s | 382 ms |
| equality | 75 ms | 70 ms | 98 ms | 1076 ms | 153 ms | 5935 ms | 4777 ms |
| zoo | 115 ms | 73 ms | 218 ms | 1044 ms | 306 ms | 16.9 s | 409 ms |

The programs are the benchmarks of Crafting Interpreters' own test suite, with two left out:
- `string_equality`: clox rejects it, because it needs more than 256 constants in one chunk.
- `zoo_batch`: it runs for ten seconds rather than doing a fixed amount of work.

A0's collector in `binary_trees` (`A0_GC_STATS=1`): 144 collections, 261.2 ms (150.4 marking), peak
20.9 MB, 96759 objects live after the last.

## Large programs

`bench/large/gen.py` makes a Minnow package of 175 modules, 100,273 lines: each module imports up
to three earlier ones and calls into them. Its build (all of it compiled, D82):

| Build | `minnow run` (compiling included) | the executable |
|---|---|---|
| 1276 ms | 1302 ms | 2 ms |

Each module's `check` calls that of one module it imports and a function of the others, so the
program does little when it runs: its time is A0's. (Until v0.9.0 every import's `check` was
called, and the calls grew exponentially with the modules; the history below keeps those numbers
apart.) Where the build's time goes
(`A0_TIME=1`):

| parse | analyze (HIR) | monomorphize | optimize (HIR) | lower to IR | verify IR | code generation | link |
|---|---|---|---|---|---|---|---|
| 218 ms | 391 ms | 0 ms | 481 ms | 135 ms | 9 ms | 89 ms | 38 ms |

`minnow lsp` on it: the time from opening a file to its diagnostics, from an edit to the answer to
the next request (which waits for the new analysis), and to answer requests:

| File open | its program | open to diagnostics | edit to an answer | hover | definition | completion |
|---|---|---|---|---|---|---|
| m87.mn | half the modules | 533 ms | 169 ms | 2 ms | 1 ms | 0 ms |
| main.mn | every module | 865 ms | 413 ms | 4 ms | 3 ms | 0 ms |

## From v0.9.0 to v1.0.0

Both on this machine, built side by side from their tags and run on the same `bench/`, alternating
(medians of 7 runs; the large program's of 5):

| | v0.9.0 | v1.0.0 |
|---|---|---|
| Minnow `nbody`, native, first round | 179 ms | 170 ms |
| Minnow `nbody`, native, second round | 125 ms | 132 ms |
| Tin `binarytrees`, native, first round | 408 ms | 372 ms |
| Tin `binarytrees`, native, second round | 378 ms | 480 ms |
| Large program, build | 1236 ms | 1421 ms |
| Large program, language server: open `main.mn` to diagnostics | 823 ms | 868 ms |
| Large program, language server: edit to an answer | 377 ms | 408 ms |

v1.0 changed APIs, formats' headers, documentation and tooling, not code generation, the optimizer
or the collector, and the two releases differ by less than one release differs from itself between
rounds. The full run above, like any single run on this shared machine, has outliers of the same
size (`nbody` at 195 ms, Tin's `binarytrees` at 488).

## From release to release

`bench/history.csv` keeps each release's headline numbers (`bench/run.py --record`,
`docs/releases.md`); `python3 bench/history.py` prints them by machine. On Intel(R) Xeon(R)
Processor @ 2.10GHz, 4 threads, in milliseconds (the large program changed after 0.9.0, as said
above, and 0.9.0 was measured on both):

| | 0.8.0 | 0.9.0 | 1.0.0 |
|---|---|---|---|
| kaleidoscope fib, native | 20 | 16 | 19 |
| kaleidoscope mandel, native | 954 | 945 | 916 |
| minnow fib, native | 64 | 71 | 63 |
| minnow sieve, native | 138 | 144 | 150 |
| minnow nbody, native | 143 | 139 | 195 |
| minnow binarytrees, native | 505 | 462 | 475 |
| minnow closures, native | 823 | 141 | 144 |
| tin fib, native | 77 | 78 | 68 |
| tin sieve, native | 123 | 98 | 105 |
| tin nbody, native | 87 | 153 | 152 |
| tin binarytrees, native | 388 | 384 | 488 |
| parallel, 1 threads | 2432 | 2427 | 2406 |
| parallel, 2 threads | 1261 | 1246 | 1213 |
| parallel, 4 threads | 684 | 644 | 626 |
| parallel, 8 threads | 633 | 630 | 618 |
| lox fib, native | 554 | 468 | 496 |
| clox fib | 1327 | 1165 | 1116 |
| lox binary_trees, native | 1363 | 1191 | 994 |
| clox binary_trees | 3569 | 3169 | 2661 |
| lox method_call, native | 195 | 172 | 214 |
| clox method_call | 245 | 220 | 242 |
| lox invocation, native | 477 | 408 | 417 |
| clox invocation | 423 | 329 | 382 |
| lox equality, native | 101 | 106 | 98 |
| clox equality | 4737 | 4844 | 4777 |
| lox zoo, native | 260 | 231 | 218 |
| clox zoo | 431 | 364 | 409 |
| large program until 0.9.0, build | 4775 | 2723 |  |
| large program until 0.9.0, language server edit to an answer | 1973 | 370 |  |
| large program, build |  | 1236 | 1276 |
| large program, language server edit to an answer |  | 377 | 413 |

## Why

### Compile time is the linker's

A0 compiles these programs to an object file in 4–13 ms: parsing, building and checking the HIR,
optimizing, lowering and Cranelift together. The rest of a build is linking: A0 runs the platform's
C compiler driver (`cc`) on the object file and the runtime library, and the driver accounts for
most of it. In this run gcc's driver took 27–32 ms to link a plain C object and clang 48–53 ms;
A0's builds took about as long as compiling plus linking with each (`CC=clang`: 63–79 ms). lld,
run directly on a target pack, links in about 10 ms (`docs/cross.md`). clang -O0 takes 93–101 ms
to compile and link these programs, and `go build` 176–198 ms even with Go's standard library
cached.

For running a program at once, A0 does not link: `lang run` compiles into memory, so
Kaleidoscope's `fib` runs in 21 ms all told and Minnow's programs about as fast as their
executables. A Lox program first compiles the prelude functions it reaches, which costs 10–200 ms
over the executable.

### Minnow: Cranelift is clang -O0, not -O2

A0 hands per-function optimization to Cranelift (D27). Cranelift's code runs at about clang -O0's
speed: 63 ms against 76 ms on `fib`, 125–195 ms across runs against 185 ms on `nbody`, and 150 ms
against 165 ms on `sieve`. clang -O2 is about twice as fast. -O2 runs LLVM's whole pipeline (inlining, loop and vector
optimizations), which Cranelift does not attempt; on `fib`, for example, it turns one of the two
recursive calls into a loop.

The HIR optimizer changes nothing on the first four programs (the "no HIR optimizer" column).
They have no small functions to inline and no dynamic types to fold. On `closures` it inlines
`fold` and then the closure (D84): 144 ms against 782 ms without it, between clang -O0 (232 ms)
and -O2 (64 ms), which also inlines the function pointer once it inlines `fold`. Go's 49 ms is the
same inlining.

- **sieve** (150 ms against 72 ms for C at -O2 and 103 ms for Go). The array is allocated at its
  size and holds a byte per boolean, as in C; what remains is Cranelift's code and the bounds
  check on every index, which Go has too (Tin, without the check, takes 105 ms).
- **nbody** (125–195 ms against 60 ms at -O2 and 129 ms for Go).
  - The bodies are structs on the heap, reached through a bounds-checked array of references.
  - `math.sqrt` is a call to C's `sqrt` instead of an instruction.
- **binarytrees** (475 ms against 316–363 ms for malloc and free). Close to a third of A0's time
  is collection: 153 ms over 56 collections, 83 ms of it marking. The collector is a non-moving
  mark and sweep, so it marks every live object at every collection, the long-lived tree included;
  v0.5.0 made each collection rarer and cheaper (D64), and a generational collector would avoid
  most of what is left (D84 measured why it is not needed yet). Go is slower still on this
  program, at 730 ms. Its collector is built for short pauses beside a running program, not for
  the throughput of a small single-threaded one.

### Tin: what a language without a collector gets

Tin's programs are Minnow's written as C: the same compiler, optimizer and code generator, without
the collector, bounds checks and references.

- **binarytrees** (372–488 ms across runs, clang -O0's 402, Minnow's 475). Tin allocates with C's `calloc` and
  frees each tree as C does, so it does what the C program does; the collector's time is gone. In
  the JIT it takes 809 ms: `lang run` runs `main` on a thread of its own, and glibc's allocator is
  slower on a thread's arena than on the main one for this pattern of allocating and freeing.
- **nbody** (152 ms against 184 at clang -O0, and 122 ms without the HIR optimizer: this run's
  noise is that large). The bodies are five structs in one block of memory, reached through
  pointers; Minnow reaches objects through a checked array of references.
- **sieve** (105 ms): bytes from `alloc` indexed without checks, between clang -O0 and -O2.
- **fib** is calls and the same as Minnow's.

### Kaleidoscope: A0's optimizer against LLVM's passes

The tutorial's chapter 7, as of LLVM 18, runs four function passes on each function: instcombine,
reassociate, GVN and simplifycfg. It has no inliner and no tail-call elimination. Although the
chapter is about mem2reg, its code does not run it either, so every argument and variable stays in
a stack slot. As a result, `mandel`'s inner loop calls the user-defined `|` and `>` operators on
every iteration and recurses for every step.

A0's HIR optimizer inlines small functions like those operators. That makes `mandel` faster than
the tutorial's code (916 against 1015 ms in this run); without it, A0 is about as fast (980 ms).
On `fib` there is nothing to inline, and both run in 9–19 ms.

LLVM's standard -O2 pipeline, given the whole program, does everything: it inlines the operators
and `mandelconverge`, turns the recursion into loops, promotes every variable to a register and
vectorizes. It runs `mandel` in 217 ms, 4 times faster than A0, which shows what a full
optimizing pipeline buys over Cranelift's per-function optimization and A0's inlining. Its compile
time is about the tutorial's on these small programs, 24–48 ms.

How LLVM's times were split: its JIT compiles each function when a call first needs it. Its compile
time is therefore measured as the same program with the last line's calls doing no work
(`printd(fib(1))`), and the rest of the full run counts as running.

### Lox: dynamic objects with shapes and caches

A0's Lox compiles every Lox value to `any`. Lox's semantics (calls, classes, properties) are HIR
functions in its prelude, which the optimizer inlines and specializes. The optimizer matters more
here than anywhere else: without it, `fib` takes 2483 ms and is slower than clox.

- **`fib`** (496 ms against 1116 ms). Once the prelude's call functions are inlined, arithmetic and
  comparisons are native code, and a call is a few type tests and an indirect call; clox dispatches
  bytecode for every operation.
- **Fields and methods** are HIR dynamic objects (D66): an instance's fields and a class's methods
  are slots found through the object's shape, and every place in the program that names a field
  keeps a cache of the shape it saw last, so a property access is a comparison and a load. clox
  hashes the name, which it interns, into each instance's table. `zoo` (property access) runs in
  218 ms against 409 ms, `method_call` in 214 against 242.
- **Method calls** look the method up first and call it on the object, allocating no bound method,
  as clox's `OP_INVOKE` does; `invocation`, which is mostly calls of methods with arguments, is
  slower than clox (417 against 382 ms).
- **`binary_trees`** (994 ms against 2661 ms). An instance is its struct and one object of
  slots; the collector takes 261 ms of it.
- **`equality`** compares constants and throws the results away. A0's optimizer removes the
  comparisons, so its 98 ms is the loop alone.
- **Interpreted**, A0 is 1.2–45 times slower than clox. A0's interpreter runs the core IR and is
  not tuned for speed; it is the reference every test checks the native code against and the
  debugger's engine. `lox file` now uses the JIT.

## What these numbers do not say

- They come from one machine (a shared cloud machine) and from small programs written the obvious
  way. Differences of 10% between runs, and in linking times, are noise.
- A0 is compared with its optimizer on and off; the references are compared as they are usually
  used. Go builds with its standard library cached, and LLVM's tutorial is a JIT.
- Output is checked, not just timed. Every implementation of a program must print the same:
  - Lox ignores the times its programs print.
  - Numbers are compared as clox prints them, with `%g`: A0's Lox prints `zoo`'s sum as 10000002
    where clox prints `1e+07`.
- Native debug information (`-g`) and the debugger are not measured.

To reproduce, on Linux or macOS with clang, LLVM's development files and Go installed:

```
python3 bench/run.py --runs 5
```
