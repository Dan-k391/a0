# Benchmarks

A small, honest picture of where A0 stands against established tools: how long its languages take
to compile, how fast what they compile runs, and how fast `lang run` runs a program at once. It is
not a CI check and nothing was tuned for it: the programs are the obvious ones, written the same
way in each language. `bench/run.py` reproduces everything here (`bench/README.md`); the tables
below are its output for v0.7.0, with v0.6.0's beside them.

## In short

- **Threads run at the same time (v0.7).** Counting the primes below 5,000,000 takes 856 ms on one
  thread and 433, 236 and 123 ms on 2, 4 and 8 (2.0, 3.6 and 7.0 times faster), natively and in the
  JIT alike.
- **Programs without threads are unaffected.** Threads changed the collector (a shadow stack per
  thread), the runtime's tables (locks) and lowering (polls), but a program with one thread takes
  none of the new paths: v0.6.0's and v0.7.0's executables of the same Lox programs, run in
  alternation on one machine, take the same time (below). This run's machine was noisier than
  v0.6.0's (clox's `binary_trees` 1879 → 2676 ms), so its own columns moved more than A0 did.
- **Tin, the systems language, runs at C's speed.** Without a collector, with C's allocator and
  pointers, Tin's `binarytrees` takes 145 ms against clang -O0's 138 (Minnow, with its collector:
  224), and its `nbody` 47 ms against clang -O0's 78. clang -O2 is still 1.3–2 times faster.
- **Running at once.** `lang run` compiles into memory (D63): Kaleidoscope's `mandel` takes 198 ms
  with `kaleidoscope run`. The interpreter (`A0_INTERP=1`) stays the reference and the debugger's
  engine, 14–50 times slower than clox.
- **Compiling.** A0's own work, from source to object file, is 1–5 ms; the rest of a build is the
  system linker: 160–190 ms with gcc's `cc`, which takes 157 ms to link even a plain C object, and
  95–102 ms with `CC=clang`.
- **Typed code (Minnow) against C and Go.** On calls, arrays and floating point A0 runs at clang
  -O0's speed and close to Go, half as fast as clang -O2. On allocation it is 1.5 times slower
  than malloc and free and 1.6 times faster than Go.
- **Kaleidoscope against LLVM.** A0's Kaleidoscope is 502 lines against the tutorial's 901, and runs
  `mandel` 2.2 times faster than the tutorial's passes (the HIR optimizer's inlining) and 1.6 times
  slower than LLVM's whole -O2 pipeline.
- **Lox against clox.** Compiled with A0, Lox is faster than clox on five of the six benchmarks:
  2.7 times on `fib`, 5 times on `binary_trees`, 1.5 times on `method_call` and `zoo`, and even
  with it on `invocation` (159 against 156 ms).

## Tools

- Machine: x86_64, Linux-6.6.114.1-microsoft-standard-WSL2-x86_64-with-glibc2.39, 32 threads
  (WSL 2 on Windows 11)
- CPU: AMD Ryzen 9 9955HX3D 16-Core Processor
- A0: v0.7.0, rustc 1.99.1 (58cf8f9c3 2026-10-13), Cranelift 0.137, release build
- Ubuntu clang version 18.1.3 (1ubuntu1)
- `cc`, which A0 links with: cc (Ubuntu 13.3.0-6ubuntu2~24.04.1) 13.3.0
- LLVM 18.1.3: the Kaleidoscope tutorial, chapter 7, built with `clang++ -O2`; and the same with
  LLVM's -O2 pipeline on the whole program (below)
- go version go1.22.2 linux/amd64
- clox at craftinginterpreters 4a840f70f69c, built with `clang -O2 -std=c99 -w`
- Times are medians of 5 runs (wall clock, so each includes starting a process, about 1 ms);
  compile times include linking. A0's interpreter ran 2 times per program.

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
- A0's Kaleidoscope: 502 (grammar, compiler, driver, runtime).
- LLVM's chapter 7: 901 (`toy.cpp` and `KaleidoscopeJIT.h`).

| Program | A0 build | no HIR optimizer | A0 native run | no HIR optimizer | A0 JIT (`run`) | A0 interpreted | LLVM JIT: startup and compile | LLVM JIT: run | LLVM -O2: startup and compile | LLVM -O2: run |
|---|---|---|---|---|---|---|---|---|---|---|
| fib | 180 ms | 168 ms | 6 ms | 6 ms | 8 ms | 208 ms | 12 ms | 5 ms | 12 ms | 4 ms |
| mandel | 169 ms | 183 ms | 173 ms | 408 ms | 198 ms | 7777 ms | 22 ms | 378 ms | 20 ms | 107 ms |

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
| fib | 172 ms | 163 ms | 127 ms | 145 ms | 230 ms | 35 ms | 34 ms | 34 ms | 31 ms | 17 ms | 29 ms |
| sieve | 190 ms | 176 ms | 125 ms | 136 ms | 202 ms | 44 ms | 40 ms | 44 ms | 43 ms | 26 ms | 33 ms |
| nbody | 179 ms | 177 ms | 119 ms | 141 ms | 176 ms | 72 ms | 69 ms | 76 ms | 79 ms | 37 ms | 51 ms |
| binarytrees | 190 ms | 171 ms | 115 ms | 134 ms | 198 ms | 224 ms | 224 ms | 203 ms | 146 ms | 125 ms | 360 ms |

The programs:
- `fib` is calls (`fib(35)`).
- `sieve` is arrays: the primes below 2,000,000, ten times.
- `nbody` is floating point and structs: the Benchmarks Game's n-body, 1,000,000 steps.
- `binarytrees` is allocation: the Benchmarks Game's binary-trees at depth 16. The C version uses
  malloc and free.

A0's collector in `binarytrees` (`A0_GC_STATS=1`): 56 collections, 55.2 ms (40.7 marking), peak
22.4 MB, 240329 objects live after the last (v0.6.0: the same collections).

## Tin against C

Tin (`examples/tin`) is A0's systems language: no collector, structs as values, raw pointers, C's
allocator. Its programs are written as the C ones are: `binarytrees` with `alloc` and `free`,
`sieve` in memory from `alloc` indexed without checks, `nbody` with the bodies in one block of
memory reached through pointers (`bench/tin`).

| Program | Tin build | Tin run | no HIR optimizer | Tin JIT (`run`) | clang -O0 | clang -O2 |
|---|---|---|---|---|---|---|
| fib | 159 ms | 29 ms | 30 ms | 31 ms | 28 ms | 15 ms |
| sieve | 161 ms | 36 ms | 40 ms | 38 ms | 42 ms | 25 ms |
| nbody | 169 ms | 47 ms | 47 ms | 51 ms | 78 ms | 35 ms |
| binarytrees | 166 ms | 145 ms | 146 ms | 321 ms | 138 ms | 119 ms |

## Threads

`parallel` (`bench/minnow/parallel.mn`) counts the primes below 5,000,000, each of `n` threads
testing every `n`-th odd number, on a machine with 32 processors (16 cores):

| Threads | Minnow native | speedup | Minnow JIT (`run`) | speedup |
|---|---|---|---|---|
| 1 | 856 ms | 1.0x | 863 ms | 1.0x |
| 2 | 433 ms | 2.0x | 443 ms | 1.9x |
| 4 | 236 ms | 3.6x | 227 ms | 3.8x |
| 8 | 123 ms | 7.0x | 122 ms | 7.1x |

The threads share nothing but the counts they return, so the speedup is the processors'. Threads
that allocate share the heap lock and collections stop them all (D76): allocation-heavy work
speeds up less.

Where Minnow's build time goes (`cc` is x86_64-linux-gnu-gcc-13). Each of A0's steps includes the
ones before it; the object file is made from the IR text with `a0 obj`.

| Program | to HIR | to IR | object file | executable | executable, `CC=clang` | cc linking a C object | clang linking it |
|---|---|---|---|---|---|---|---|
| fib | 1 ms | 1 ms | 1 ms | 158 ms | 95 ms | 157 ms | 96 ms |
| nbody | 4 ms | 5 ms | 5 ms | 165 ms | 102 ms | 160 ms | 96 ms |

## Lox: A0 against clox

| Program | A0 build | no HIR optimizer | A0 native run | no HIR optimizer | A0 JIT (`lox file`) | A0 interpreted | clox |
|---|---|---|---|---|---|---|---|
| fib | 170 ms | 177 ms | 182 ms | 842 ms | 207 ms | 12.7 s | 490 ms |
| binary_trees | 206 ms | 184 ms | 509 ms | 1225 ms | 552 ms | 22.8 s | 2676 ms |
| method_call | 243 ms | 205 ms | 73 ms | 170 ms | 145 ms | 4472 ms | 109 ms |
| invocation | 249 ms | 200 ms | 159 ms | 306 ms | 237 ms | 7495 ms | 156 ms |
| equality | 188 ms | 179 ms | 64 ms | 344 ms | 80 ms | 2215 ms | 2403 ms |
| zoo | 209 ms | 188 ms | 99 ms | 323 ms | 138 ms | 7446 ms | 155 ms |

The programs are the benchmarks of Crafting Interpreters' own test suite, with two left out:
- `string_equality`: clox rejects it, because it needs more than 256 constants in one chunk.
- `zoo_batch`: it runs for ten seconds rather than doing a fixed amount of work.

A0's collector in `binary_trees` (`A0_GC_STATS=1`): 144 collections, 174.2 ms (122.2 marking), peak
20.9 MB, 96759 objects live after the last (v0.6.0: the same collections, 138.0 ms on its quieter
run).

## From v0.6.0 to v0.7.0

Threads touched what every program runs: the collector's frames, the runtime's tables, the
allocator. A first run of this benchmark found Lox's method calls 6–15% slower than v0.6.0's
(frames through a thread-local, field lookups through lock checks that were calls, an atomic call
per allocation), and the single-threaded paths were made v0.6.0's again (a program's main thread
keeps its frames in a static struct; locks are taken only once a second thread exists). To compare
without the noise of two runs, v0.6.0's and v0.7.0's executables of the same programs ran in
alternation, best of seven, three rounds, on one machine:

| Program (native) | v0.6.0 | v0.7.0 |
|---|---|---|
| Lox `fib` | 180 ms | 184 ms |
| Lox `method_call` | 70 ms | 70 ms |
| Lox `invocation` | 153 ms | 154 ms |
| Lox `zoo` | 102 ms | 100 ms |

The collectors did the same work in both (the same collections, peak and live objects).

## Why

### Compile time is the linker's

A0 compiles these programs to an object file in 1–5 ms: parsing, building and checking the HIR,
optimizing, lowering and Cranelift together. The rest of a build is linking: A0 runs the platform's
C compiler driver (`cc`) on the object file and the runtime library, and the driver accounts for
most of it. In this run gcc's driver took 157–160 ms to link a plain C object and clang 96 ms;
A0's builds took about as long as linking with each (`CC=clang`: 95–102 ms). Against clang -O0,
which compiles C in about 20 ms and links with its own driver, an A0 build costs little more.
`go build` takes 176–230 ms even with Go's standard library cached.

For running a program at once, A0 no longer links: `lang run` compiles into memory, so
Kaleidoscope's `fib` runs in 8 ms all told and Minnow's programs as fast as their executables. A
Lox program first compiles the prelude functions it reaches, which costs 20–70 ms over the
executable.

### Minnow: Cranelift is clang -O0, not -O2

A0 hands per-function optimization to Cranelift (D27). Cranelift's code runs at about clang -O0's
speed: 35 ms against 31 ms on `fib`, 72 ms against 79 ms on `nbody`, and 44 ms against 43 ms on
`sieve`. clang -O2 is about twice as fast. -O2 runs LLVM's whole pipeline (inlining, loop and vector
optimizations), which Cranelift does not attempt; on `fib`, for example, it turns one of the two
recursive calls into a loop.

The HIR optimizer changes nothing on these programs (the "no HIR optimizer" column). They have no
small functions to inline and no dynamic types to fold.

- **sieve** (44 ms against 26 ms for C at -O2 and 33 ms for Go). The array is allocated at its
  size and holds a byte per boolean, as in C; what remains is Cranelift's code and the bounds
  check on every index, which Go has too (Tin, without the check, takes 36 ms).
- **nbody** (72 ms against 37 ms at -O2 and 51 ms for Go).
  - The bodies are structs on the heap, reached through a bounds-checked array of references.
  - `math.sqrt` is a call to C's `sqrt` instead of an instruction.
- **binarytrees** (224 ms against 125–146 ms for malloc and free). A quarter of A0's time is
  collection: 55 ms over 56 collections, 41 ms of it marking. The collector is a non-moving mark and sweep, so it
  marks every live object at every collection, the long-lived tree included; v0.5.0 made each
  collection rarer and cheaper (D64), and a generational collector would avoid most of what is
  left. Go is slower still on this program, at 360 ms. Its collector is built for short pauses
  beside a running program, not for the throughput of a small single-threaded one.

### Tin: what a language without a collector gets

Tin's programs are Minnow's written as C: the same compiler, optimizer and code generator, without
the collector, bounds checks and references.

- **binarytrees** (145 ms, clang -O0's 138, Minnow's 224). Tin allocates with C's `calloc` and
  frees each tree as C does, so it does what the C program does; the collector's 55 ms is gone. In
  the JIT it takes 321 ms: `lang run` runs `main` on a thread of its own, and glibc's allocator is
  slower on a thread's arena than on the main one for this pattern of allocating and freeing.
- **nbody** (47 ms against 78 at clang -O0 and Minnow's 72). The bodies are five structs in one
  block of memory, reached through pointers; Minnow reaches objects through a checked array of
  references. Cranelift keeps the fields in registers where clang -O0 reloads them from memory.
- **sieve** (36 ms): bytes from `alloc` indexed without checks, between clang -O0 and -O2.
- **fib** is calls and the same as Minnow's.

### Kaleidoscope: A0's optimizer against LLVM's passes

The tutorial's chapter 7, as of LLVM 18, runs four function passes on each function: instcombine,
reassociate, GVN and simplifycfg. It has no inliner and no tail-call elimination. Although the
chapter is about mem2reg, its code does not run it either, so every argument and variable stays in
a stack slot. As a result, `mandel`'s inner loop calls the user-defined `|` and `>` operators on
every iteration and recurses for every step.

A0's HIR optimizer inlines small functions like those operators. That makes `mandel` 1.6 times
faster than the tutorial's code (173 against 378 ms in this run); without it, A0 is slower (408 ms). On `fib`
there is nothing to inline, and both run in 4–6 ms.

LLVM's standard -O2 pipeline, given the whole program, does everything: it inlines the operators
and `mandelconverge`, turns the recursion into loops, promotes every variable to a register and
vectorizes. It runs `mandel` in 107 ms, 1.6 times faster than A0, which
shows what a full optimizing pipeline buys over Cranelift's per-function optimization and A0's
inlining. Its compile time is the same as the tutorial's on these small programs, about 20 ms.

How LLVM's times were split: its JIT compiles each function when a call first needs it. Its compile
time is therefore measured as the same program with the last line's calls doing no work
(`printd(fib(1))`), and the rest of the full run counts as running.

### Lox: dynamic objects with shapes and caches

A0's Lox compiles every Lox value to `any`. Lox's semantics (calls, classes, properties) are HIR
functions in its prelude, which the optimizer inlines and specializes. The optimizer matters more
here than anywhere else: without it, `fib` takes 842 ms and is slower than clox.

- **`fib`** (182 ms against 490 ms). Once the prelude's call functions are inlined, arithmetic and
  comparisons are native code, and a call is a few type tests and an indirect call; clox dispatches
  bytecode for every operation.
- **Fields and methods** are HIR dynamic objects (D66): an instance's fields and a class's methods
  are slots found through the object's shape, and every place in the program that names a field
  keeps a cache of the shape it saw last, so a property access is a comparison and a load. clox
  hashes the name, which it interns, into each instance's table. `zoo` (property access) runs in
  99 ms against 155 ms, `method_call` in 73 against 109.
- **Method calls** look the method up first and call it on the object, allocating no bound method,
  as clox's `OP_INVOKE` does: `invocation` is about even with clox (159 against 156 ms).
- **`binary_trees`** (509 ms against 2676 ms on this noisy run). An instance is its struct and one
  object of slots; the collector takes 174 ms of it.
- **`equality`** compares constants and throws the results away. A0's optimizer removes the
  comparisons, so its 64 ms is the loop alone.
- **Interpreted**, A0 is 14–50 times slower than clox. A0's interpreter runs the core IR and is
  not tuned for speed; it is the reference every test checks the native code against and the
  debugger's engine. `lox file` now uses the JIT.

## What these numbers do not say

- They come from one machine (WSL 2 on a laptop) and from small programs written the obvious way.
  Differences of a few milliseconds, and in linking times between runs, are noise.
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
