# Benchmarks

A small, honest picture of where A0 stands against established tools: how long its languages take
to compile, how fast what they compile runs, and how fast `lang run` runs a program at once. It is
not a CI check and nothing was tuned for it: the programs are the obvious ones, written the same
way in each language. `bench/run.py` reproduces everything here (`bench/README.md`); the tables
below are its output for v0.6.0, with v0.5.0's beside them.

## In short

- **Garbage-collected languages are unaffected by v0.6.** Its systems features (sized numbers,
  inline structs, pointers, manual memory, C structs) change nothing for programs that do not use
  them: Minnow, Lox and Kaleidoscope run as before, and their collectors do the same work, to the
  collection. This run's machine was about 15% faster than v0.5.0's (clox's `fib` 554 → 478 ms,
  clang -O0's `fib` 32 → 27 ms), and A0's times moved with it.
- **Tin, the systems language, runs at C's speed.** Without a collector, with C's allocator and
  pointers, Tin's `binarytrees` takes 143 ms against clang -O0's 138 (Minnow, with its collector:
  204), and its `nbody`, structs in place reached through pointers, 48 ms against clang -O0's 75
  and Minnow's 70. clang -O2 is still 1.4–2.3 times faster (Cranelift is not -O2).
- **Running at once.** `lang run` compiles into memory (D63): Kaleidoscope's `mandel` takes 200 ms
  with `kaleidoscope run`, as fast as its executable. The interpreter (`A0_INTERP=1`) stays the
  reference and the debugger's engine, 14–50 times slower than clox.
- **Compiling.** A0's own work, from source to object file, is 2–6 ms; the rest of a build is the
  system linker: 145–170 ms with gcc's `cc`, which takes 153 ms to link even a plain C object, and
  95–102 ms with `CC=clang`.
- **Typed code (Minnow) against C and Go.** On calls, arrays and floating point A0 runs at clang
  -O0's speed and close to Go, half as fast as clang -O2. On allocation it is 1.5 times slower
  than malloc and free and 1.7 times faster than Go.
- **Kaleidoscope against LLVM.**
  - Size: A0's Kaleidoscope is 502 lines against the tutorial's 901. Besides, A0's driver gives it
    a JIT, an interpreter, native executables, a language server, a debugger and a REPL.
  - Against the tutorial's passes, A0's code is 1.6 times faster on `mandel`, thanks to the HIR
    optimizer's inlining; `fib` is a few milliseconds either way.
  - Against LLVM's standard -O2 pipeline on the whole program, A0 is 2.2 times slower on `mandel`:
    -O2 inlines, unrolls and vectorizes, which Cranelift does not.
- **Lox against clox.** Compiled with A0, Lox is faster than clox on five of the six benchmarks:
  2.6 times on `fib`, 4.3 times on `binary_trees`, 1.5 times on `method_call` and `zoo`, and even
  with it on `invocation` (157 against 148 ms).

## Tools

- Machine: x86_64, Linux-6.6.114.1-microsoft-standard-WSL2-x86_64-with-glibc2.39, 32 threads
  (WSL 2 on Windows 11)
- CPU: AMD Ryzen 9 9955HX3D 16-Core Processor
- A0: v0.6.0, rustc 1.99.1 (58cf8f9c3 2026-10-13), Cranelift 0.137, release build
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
| fib | 159 ms | 160 ms | 6 ms | 6 ms | 8 ms | 203 ms | 13 ms | 4 ms | 13 ms | 4 ms |
| mandel | 168 ms | 160 ms | 235 ms | 404 ms | 200 ms | 7908 ms | 22 ms | 380 ms | 21 ms | 107 ms |

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
| fib | 148 ms | 147 ms | 106 ms | 111 ms | 188 ms | 31 ms | 30 ms | 29 ms | 27 ms | 15 ms | 26 ms |
| sieve | 152 ms | 144 ms | 111 ms | 109 ms | 192 ms | 41 ms | 38 ms | 44 ms | 43 ms | 26 ms | 32 ms |
| nbody | 168 ms | 167 ms | 119 ms | 139 ms | 193 ms | 70 ms | 70 ms | 79 ms | 79 ms | 36 ms | 52 ms |
| binarytrees | 157 ms | 161 ms | 122 ms | 124 ms | 177 ms | 204 ms | 203 ms | 195 ms | 140 ms | 123 ms | 353 ms |

The programs:
- `fib` is calls (`fib(35)`).
- `sieve` is arrays: the primes below 2,000,000, ten times.
- `nbody` is floating point and structs: the Benchmarks Game's n-body, 1,000,000 steps.
- `binarytrees` is allocation: the Benchmarks Game's binary-trees at depth 16. The C version uses
  malloc and free.

A0's collector in `binarytrees` (`A0_GC_STATS=1`): 56 collections, 72.9 ms (53.8 marking), peak
22.4 MB, 240329 objects live after the last (v0.5.0: the same collections, 65.1 ms).

## Tin against C

Tin (`examples/tin`) is A0's systems language: no collector, structs as values, raw pointers, C's
allocator. Its programs are written as the C ones are: `binarytrees` with `alloc` and `free`,
`sieve` in memory from `alloc` indexed without checks, `nbody` with the bodies in one block of
memory reached through pointers (`bench/tin`).

| Program | Tin build | Tin run | no HIR optimizer | Tin JIT (`run`) | clang -O0 | clang -O2 |
|---|---|---|---|---|---|---|
| fib | 145 ms | 35 ms | 35 ms | 31 ms | 27 ms | 15 ms |
| sieve | 153 ms | 35 ms | 37 ms | 38 ms | 42 ms | 25 ms |
| nbody | 163 ms | 48 ms | 48 ms | 53 ms | 75 ms | 34 ms |
| binarytrees | 169 ms | 143 ms | 143 ms | 320 ms | 138 ms | 122 ms |

Where Minnow's build time goes (`cc` is x86_64-linux-gnu-gcc-13). Each of A0's steps includes the
ones before it; the object file is made from the IR text with `a0 obj`.

| Program | to HIR | to IR | object file | executable | executable, `CC=clang` | cc linking a C object | clang linking it |
|---|---|---|---|---|---|---|---|
| fib | 2 ms | 1 ms | 2 ms | 153 ms | 95 ms | 154 ms | 92 ms |
| nbody | 4 ms | 6 ms | 5 ms | 162 ms | 102 ms | 153 ms | 92 ms |

## Lox: A0 against clox

| Program | A0 build | no HIR optimizer | A0 native run | no HIR optimizer | A0 JIT (`lox file`) | A0 interpreted | clox |
|---|---|---|---|---|---|---|---|
| fib | 169 ms | 166 ms | 184 ms | 800 ms | 189 ms | 12.6 s | 478 ms |
| binary_trees | 202 ms | 174 ms | 439 ms | 1053 ms | 489 ms | 21.7 s | 1879 ms |
| method_call | 223 ms | 179 ms | 69 ms | 161 ms | 135 ms | 4270 ms | 107 ms |
| invocation | 216 ms | 171 ms | 157 ms | 307 ms | 225 ms | 6967 ms | 148 ms |
| equality | 182 ms | 177 ms | 61 ms | 318 ms | 83 ms | 2237 ms | 2346 ms |
| zoo | 183 ms | 170 ms | 99 ms | 294 ms | 136 ms | 6958 ms | 146 ms |

The programs are the benchmarks of Crafting Interpreters' own test suite, with two left out:
- `string_equality`: clox rejects it, because it needs more than 256 constants in one chunk.
- `zoo_batch`: it runs for ten seconds rather than doing a fixed amount of work.

A0's collector in `binary_trees` (`A0_GC_STATS=1`): 144 collections, 138.0 ms (97.2 marking), peak
20.9 MB, 96759 objects live after the last (v0.5.0: the same collections, 138.5 ms).

## From v0.5.0 to v0.6.0

v0.6 added what systems languages need and left the rest alone, which is what this table checks.
This run's machine was faster than v0.5.0's: the references (clox, C, Go) took 10–20% less time,
and so did A0. Native run times:

| Program | v0.5.0 | v0.6.0 | References, v0.5.0 → v0.6.0 |
|---|---|---|---|
| Minnow `fib` | 39 ms | 31 ms | clang -O0 32 → 27 ms |
| Minnow `nbody` | 79 ms | 70 ms | clang -O0 88 → 79 ms |
| Minnow `binarytrees` | 224 ms | 204 ms | malloc and free (clang -O0) 156 → 140 ms |
| Lox `fib` | 204 ms | 184 ms | clox 554 → 478 ms |
| Lox `binary_trees` | 480 ms | 439 ms | clox 1673 → 1879 ms |
| Lox `zoo` | 110 ms | 99 ms | clox 163 → 146 ms |
| Kaleidoscope `mandel` | 265 ms | 235 ms | LLVM's tutorial 415 → 380 ms |

The collectors did exactly the same work (the same collections, peak and live objects). New in
v0.6.0 is the Tin table above.

## Why

### Compile time is the linker's

A0 compiles these programs to an object file in 2–6 ms: parsing, building and checking the HIR,
optimizing, lowering and Cranelift together. The rest of a build is linking: A0 runs the platform's
C compiler driver (`cc`) on the object file and the runtime library, and the driver accounts for
most of it. In this run gcc's driver took 153 ms to link a plain C object and clang 92 ms; A0's
builds took about as long as linking with each (`CC=clang`: 95–102 ms). Against clang -O0,
which compiles C in about 20 ms and links with its own driver, an A0 build costs little more.
`go build` takes 177–193 ms even with Go's standard library cached.

For running a program at once, A0 no longer links: `lang run` compiles into memory, so
Kaleidoscope's `fib` runs in 8 ms all told and Minnow's programs as fast as their executables. A
Lox program first compiles the prelude functions it reaches, which costs 20–70 ms over the
executable.

### Minnow: Cranelift is clang -O0, not -O2

A0 hands per-function optimization to Cranelift (D27). Cranelift's code runs at about clang -O0's
speed: 31 ms against 27 ms on `fib`, 70 ms against 79 ms on `nbody`, and 41 ms against 43 ms on
`sieve`. clang -O2 is about twice as fast. -O2 runs LLVM's whole pipeline (inlining, loop and vector
optimizations), which Cranelift does not attempt; on `fib`, for example, it turns one of the two
recursive calls into a loop.

The HIR optimizer changes nothing on these programs (the "no HIR optimizer" column). They have no
small functions to inline and no dynamic types to fold.

- **sieve** (41 ms against 26 ms for C at -O2 and 32 ms for Go). The array is allocated at its
  size and holds a byte per boolean, as in C; what remains is Cranelift's code and the bounds
  check on every index, which Go has too (Tin, without the check, takes 35 ms).
- **nbody** (70 ms against 36 ms at -O2 and 52 ms for Go).
  - The bodies are structs on the heap, reached through a bounds-checked array of references.
  - `math.sqrt` is a call to C's `sqrt` instead of an instruction.
- **binarytrees** (204 ms against 123–140 ms for malloc and free). A third of A0's time is
  collection: 73 ms over 56 collections, 54 ms of it marking. The collector is a non-moving mark and sweep, so it
  marks every live object at every collection, the long-lived tree included; v0.5.0 made each
  collection rarer and cheaper (D64), and a generational collector would avoid most of what is
  left. Go is slower still on this program, at 353 ms. Its collector is built for short pauses
  beside a running program, not for the throughput of a small single-threaded one.

### Tin: what a language without a collector gets

Tin's programs are Minnow's written as C: the same compiler, optimizer and code generator, without
the collector, bounds checks and references.

- **binarytrees** (143 ms, clang -O0's 138, Minnow's 204). Tin allocates with C's `calloc` and
  frees each tree as C does, so it does what the C program does; the collector's 73 ms is gone. In
  the JIT it takes 320 ms: `lang run` runs `main` on a thread of its own, and glibc's allocator is
  slower on a thread's arena than on the main one for this pattern of allocating and freeing.
- **nbody** (48 ms against 75 at clang -O0 and Minnow's 70). The bodies are five structs in one
  block of memory, reached through pointers; Minnow reaches objects through a checked array of
  references. Cranelift keeps the fields in registers where clang -O0 reloads them from memory.
- **sieve** (35 ms): bytes from `alloc` indexed without checks, between clang -O0 and -O2.
- **fib** is calls and the same as Minnow's.

### Kaleidoscope: A0's optimizer against LLVM's passes

The tutorial's chapter 7, as of LLVM 18, runs four function passes on each function: instcombine,
reassociate, GVN and simplifycfg. It has no inliner and no tail-call elimination. Although the
chapter is about mem2reg, its code does not run it either, so every argument and variable stays in
a stack slot. As a result, `mandel`'s inner loop calls the user-defined `|` and `>` operators on
every iteration and recurses for every step.

A0's HIR optimizer inlines small functions like those operators. That makes `mandel` 1.6 times
faster than the tutorial's code (235 against 380 ms); without it, A0 is slower (404 ms). On `fib`
there is nothing to inline, and both run in 4–6 ms.

LLVM's standard -O2 pipeline, given the whole program, does everything: it inlines the operators
and `mandelconverge`, turns the recursion into loops, promotes every variable to a register and
vectorizes. It runs `mandel` in 107 ms, 2.2 times faster than A0, which
shows what a full optimizing pipeline buys over Cranelift's per-function optimization and A0's
inlining. Its compile time is the same as the tutorial's on these small programs, about 20 ms.

How LLVM's times were split: its JIT compiles each function when a call first needs it. Its compile
time is therefore measured as the same program with the last line's calls doing no work
(`printd(fib(1))`), and the rest of the full run counts as running.

### Lox: dynamic objects with shapes and caches

A0's Lox compiles every Lox value to `any`. Lox's semantics (calls, classes, properties) are HIR
functions in its prelude, which the optimizer inlines and specializes. The optimizer matters more
here than anywhere else: without it, `fib` takes 800 ms and is slower than clox.

- **`fib`** (184 ms against 478 ms). Once the prelude's call functions are inlined, arithmetic and
  comparisons are native code, and a call is a few type tests and an indirect call; clox dispatches
  bytecode for every operation.
- **Fields and methods** are HIR dynamic objects (D66): an instance's fields and a class's methods
  are slots found through the object's shape, and every place in the program that names a field
  keeps a cache of the shape it saw last, so a property access is a comparison and a load. clox
  hashes the name, which it interns, into each instance's table. `zoo` (property access) runs in
  99 ms against 146 ms, `method_call` in 69 against 107.
- **Method calls** look the method up first and call it on the object, allocating no bound method,
  as clox's `OP_INVOKE` does: `invocation` is about even with clox (157 against 148 ms).
- **`binary_trees`** (439 ms against 1879 ms). An instance is its struct and one object of slots;
  the collector takes 138 ms of it.
- **`equality`** compares constants and throws the results away. A0's optimizer removes the
  comparisons, so its 61 ms is the loop alone.
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
