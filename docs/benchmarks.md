# Benchmarks

A small, honest picture of where A0 stands against established tools: how long its languages take
to compile, how fast what they compile runs, and how fast `lang run` runs a program at once. It is
not a CI check and nothing was tuned for it: the programs are the obvious ones, written the same
way in each language. `bench/run.py` reproduces everything here (`bench/README.md`); the tables
below are its output for v0.9.0. "From v0.8.0 to v0.9.0" compares with v0.8.0 run on the same
machine, a shared 4-processor cloud machine that is slower and noisier than the one earlier
releases were measured on (whose numbers are in this page's history), so compare within this page
only.

## In short

- **Closures are inlined (v0.9).** A closure passed to a function that is inlined, as in
  `fold(xs, 0, fn(a, b) { ... })`, is inlined too (D84): `closures` runs in 141 ms instead of 823,
  2.2 times slower than clang -O2 and Go instead of 13 times, and faster than clang -O0.
- **Large programs (v0.9).** A generated program of 100,000 lines in 175 modules builds in 2.7 s
  instead of 4.8 s, and the language server answers after an edit in 370 ms instead of 2.0 s
  (D82, below).
- **Everything else is v0.8's**, within the noise of a shared machine.
- **Threads run at the same time.** Counting the primes below 5,000,000 takes 2.4 s on one thread
  and 1.25 s and 0.64 s on 2 and 4 (3.8 times faster on this 4-processor machine).
- **Tin, the systems language, runs at C's speed.** Without a collector, its `binarytrees` takes
  384 ms against clang -O0's 341 (Minnow, with its collector: 462).
- **Typed code (Minnow) against C and Go.** On calls, arrays and floating point A0 runs at about
  clang -O0's speed and close to Go, about half as fast as clang -O2. On allocation it is 1.4 times
  slower than malloc and free and faster than Go.
- **Kaleidoscope against LLVM.** A0's Kaleidoscope is 502 lines against the tutorial's 901 and runs
  `mandel` slightly faster than the tutorial's passes (945 against 1090 ms) and 4 times slower than
  LLVM's whole -O2 pipeline.
- **Lox against clox.** Compiled with A0, Lox is faster than clox on five of the six benchmarks
  (2.5 times on `fib`, 2.7 times on `binary_trees`, 1.3 and 1.6 times on `method_call` and `zoo`)
  and slower on `invocation` (408 against 329 ms).
- **Compiling.** A0's own work, from source to object file, is 5–20 ms on these programs; the rest
  of a build is the linker.

## Tools

- Machine: x86_64, Linux 6.18 (a cloud virtual machine), 4 threads
- CPU: Intel(R) Xeon(R) Processor @ 2.10GHz
- A0: v0.9.0, rustc 1.97.0 (2d8144b78 2026-07-07), Cranelift 0.137, release build
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
- A0's Kaleidoscope: 502 (grammar, compiler, driver, runtime).
- LLVM's chapter 7: 901 (`toy.cpp` and `KaleidoscopeJIT.h`).

| Program | A0 build | no HIR optimizer | A0 native run | no HIR optimizer | A0 JIT (`run`) | A0 interpreted | LLVM JIT: startup and compile | LLVM JIT: run | LLVM -O2: startup and compile | LLVM -O2: run |
|---|---|---|---|---|---|---|---|---|---|---|
| fib | 40 ms | 41 ms | 16 ms | 17 ms | 18 ms | 544 ms | 25 ms | 12 ms | 25 ms | 9 ms |
| mandel | 36 ms | 46 ms | 945 ms | 1002 ms | 929 ms | 18.9 s | 41 ms | 1090 ms | 41 ms | 231 ms |

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
| fib | 41 ms | 36 ms | 90 ms | 110 ms | 195 ms | 71 ms | 68 ms | 72 ms | 66 ms | 39 ms | 63 ms |
| sieve | 48 ms | 48 ms | 127 ms | 125 ms | 212 ms | 144 ms | 143 ms | 114 ms | 163 ms | 87 ms | 92 ms |
| nbody | 69 ms | 70 ms | 108 ms | 159 ms | 195 ms | 139 ms | 154 ms | 143 ms | 167 ms | 66 ms | 107 ms |
| binarytrees | 36 ms | 44 ms | 83 ms | 104 ms | 189 ms | 462 ms | 484 ms | 512 ms | 341 ms | 274 ms | 617 ms |
| closures | 35 ms | 37 ms | 95 ms | 116 ms | 204 ms | 141 ms | 719 ms | 179 ms | 269 ms | 60 ms | 72 ms |

The programs:
- `fib` is calls (`fib(35)`).
- `sieve` is arrays: the primes below 2,000,000, ten times.
- `nbody` is floating point and structs: the Benchmarks Game's n-body, 1,000,000 steps.
- `binarytrees` is allocation: the Benchmarks Game's binary-trees at depth 16. The C version uses
  malloc and free.
- `closures` is a closure that captures a variable passed to a small higher-order function,
  `fold`, 100,000 times over 1,000 numbers; the C version passes a function pointer and its
  environment.

A0's collector in `binarytrees` (`A0_GC_STATS=1`): 56 collections, 136.4 ms (71.5 marking), peak
22.4 MB, 240329 objects live after the last.

## Tin against C

Tin (`examples/tin`) is A0's systems language: no collector, structs as values, raw pointers, C's
allocator. Its programs are written as the C ones are: `binarytrees` with `alloc` and `free`,
`sieve` in memory from `alloc` indexed without checks, `nbody` with the bodies in one block of
memory reached through pointers (`bench/tin`).

| Program | Tin build | Tin run | no HIR optimizer | Tin JIT (`run`) | clang -O0 | clang -O2 |
|---|---|---|---|---|---|---|
| fib | 35 ms | 78 ms | 78 ms | 62 ms | 64 ms | 35 ms |
| sieve | 36 ms | 98 ms | 115 ms | 108 ms | 177 ms | 93 ms |
| nbody | 47 ms | 153 ms | 96 ms | 101 ms | 177 ms | 64 ms |
| binarytrees | 38 ms | 384 ms | 391 ms | 910 ms | 328 ms | 390 ms |

## Threads

`parallel` (`bench/minnow/parallel.mn`) counts the primes below 5,000,000, each of `n` threads
testing every `n`-th odd number, on this machine's 4 processors:

| Threads | Minnow native | speedup | Minnow JIT (`run`) | speedup |
|---|---|---|---|---|
| 1 | 2427 ms | 1.0x | 2449 ms | 1.0x |
| 2 | 1246 ms | 1.9x | 1249 ms | 2.0x |
| 4 | 644 ms | 3.8x | 633 ms | 3.9x |
| 8 | 630 ms | 3.9x | 650 ms | 3.8x |

The threads share nothing but the counts they return, so the speedup is the processors'. Threads
that allocate share the heap lock and collections stop them all (D76): allocation-heavy work
speeds up less.

Where Minnow's build time goes (`cc` is x86_64-linux-gnu-gcc-13). Each of A0's steps includes the
ones before it; the object file is made from the IR text with `a0 obj`.

| Program | to HIR | to IR | object file | executable | executable, `CC=clang` | cc linking a C object | clang linking it |
|---|---|---|---|---|---|---|---|
| fib | 5 ms | 5 ms | 5 ms | 36 ms | 64 ms | 24 ms | 47 ms |
| nbody | 12 ms | 19 ms | 15 ms | 58 ms | 79 ms | 33 ms | 50 ms |

## Lox: A0 against clox

| Program | A0 build | no HIR optimizer | A0 native run | no HIR optimizer | A0 JIT (`lox file`) | A0 interpreted | clox |
|---|---|---|---|---|---|---|---|
| fib | 66 ms | 55 ms | 468 ms | 2591 ms | 480 ms | 32.2 s | 1165 ms |
| binary_trees | 110 ms | 74 ms | 1191 ms | 2970 ms | 1193 ms | 49.1 s | 3169 ms |
| method_call | 160 ms | 83 ms | 172 ms | 467 ms | 383 ms | 10.4 s | 220 ms |
| invocation | 183 ms | 76 ms | 408 ms | 823 ms | 584 ms | 17.3 s | 329 ms |
| equality | 73 ms | 66 ms | 106 ms | 1095 ms | 169 ms | 6120 ms | 4844 ms |
| zoo | 96 ms | 71 ms | 231 ms | 975 ms | 319 ms | 16.8 s | 364 ms |

The programs are the benchmarks of Crafting Interpreters' own test suite, with two left out:
- `string_equality`: clox rejects it, because it needs more than 256 constants in one chunk.
- `zoo_batch`: it runs for ten seconds rather than doing a fixed amount of work.

A0's collector in `binary_trees` (`A0_GC_STATS=1`): 144 collections, 315.7 ms (172.9 marking), peak
20.9 MB, 96759 objects live after the last.

## Large programs

`bench/large/gen.py` makes a Minnow package of 175 modules, 100,273 lines: each module imports up
to three earlier ones and calls into them. Its build (all of it compiled, D82):

| Build | `minnow run` (compiling included) | the executable |
|---|---|---|
| 2723 ms | 58.5 s | 56.8 s |

The program's run time measures the program, not A0: each module's `check` calls those of the
modules it imports, so the calls grow exponentially with the modules. Where the build's time goes
(`A0_TIME=1`):

| parse | analyze (HIR) | monomorphize | optimize (HIR) | lower to IR | verify IR | code generation | link |
|---|---|---|---|---|---|---|---|
| 168 ms | 307 ms | 0 ms | 516 ms | 894 ms | 96 ms | 657 ms | 91 ms |

`minnow lsp` on it: the time from opening a file to its diagnostics, from an edit to the answer to
the next request (which waits for the new analysis), and to answer requests:

| File open | its program | open to diagnostics | edit to an answer | hover | definition | completion |
|---|---|---|---|---|---|---|
| m87.mn | half the modules | 565 ms | 202 ms | 2 ms | 2 ms | 0 ms |
| main.mn | every module | 899 ms | 370 ms | 4 ms | 4 ms | 0 ms |

## From v0.8.0 to v0.9.0

Both on this machine, the same `bench/` run against each release:

| | v0.8.0 | v0.9.0 |
|---|---|---|
| Minnow `closures`, native | 823 ms | 141 ms |
| Minnow `closures`, JIT | 950 ms | 179 ms |
| Large program, build | 4775 ms | 2723 ms |
| Large program, language server: open `main.mn` to diagnostics | 2292 ms | 899 ms |
| Large program, language server: edit to an answer | 1973 ms | 370 ms |
| Minnow `nbody`, native | 143 ms | 139 ms |
| Minnow `binarytrees`, native | 505 ms | 462 ms |
| Kaleidoscope `mandel`, native | 954 ms | 945 ms |
| Lox `fib`, native (clox) | 554 ms (1327 ms) | 468 ms (1165 ms) |
| `parallel`, 4 threads | 684 ms | 644 ms |

The closures are D84's inlining. The large program's build is D82's linear optimizer, interned
location table and parallel code generation (on 4 processors here); the language server's answer
after an edit is its reuse of unchanged files' trees and indexes. The other programs' code did not
change, and their differences are the machine's noise (clox moved as much).

## Why

### Compile time is the linker's

A0 compiles these programs to an object file in 5–15 ms: parsing, building and checking the HIR,
optimizing, lowering and Cranelift together. The rest of a build is linking: A0 runs the platform's
C compiler driver (`cc`) on the object file and the runtime library, and the driver accounts for
most of it. In this run gcc's driver took 24–33 ms to link a plain C object and clang 47–50 ms;
A0's builds took about as long as compiling plus linking with each (`CC=clang`: 64–79 ms). lld,
run directly on a target pack, links in about 10 ms (`docs/cross.md`). clang -O0 takes 83–127 ms
to compile and link these programs, and `go build` 189–212 ms even with Go's standard library
cached.

For running a program at once, A0 does not link: `lang run` compiles into memory, so
Kaleidoscope's `fib` runs in 18 ms all told and Minnow's programs about as fast as their
executables. A Lox program first compiles the prelude functions it reaches, which costs 10–200 ms
over the executable.

### Minnow: Cranelift is clang -O0, not -O2

A0 hands per-function optimization to Cranelift (D27). Cranelift's code runs at about clang -O0's
speed: 71 ms against 66 ms on `fib`, 139 ms against 167 ms on `nbody`, and 144 ms against 163 ms
on `sieve`. clang -O2 is about twice as fast. -O2 runs LLVM's whole pipeline (inlining, loop and vector
optimizations), which Cranelift does not attempt; on `fib`, for example, it turns one of the two
recursive calls into a loop.

The HIR optimizer changes nothing on the first four programs (the "no HIR optimizer" column).
They have no small functions to inline and no dynamic types to fold. On `closures` it inlines
`fold` and then the closure (D84): 141 ms against 719 ms without it, between clang -O0 (269 ms)
and -O2 (60 ms), which also inlines the function pointer once it inlines `fold`. Go's 72 ms is the
same inlining.

- **sieve** (144 ms against 87 ms for C at -O2 and 92 ms for Go). The array is allocated at its
  size and holds a byte per boolean, as in C; what remains is Cranelift's code and the bounds
  check on every index, which Go has too (Tin, without the check, takes 98 ms).
- **nbody** (139 ms against 66 ms at -O2 and 107 ms for Go).
  - The bodies are structs on the heap, reached through a bounds-checked array of references.
  - `math.sqrt` is a call to C's `sqrt` instead of an instruction.
- **binarytrees** (462 ms against 274–341 ms for malloc and free). Close to a third of A0's time
  is collection: 136 ms over 56 collections, 72 ms of it marking. The collector is a non-moving
  mark and sweep, so it marks every live object at every collection, the long-lived tree included;
  v0.5.0 made each collection rarer and cheaper (D64), and a generational collector would avoid
  most of what is left (D84 measured why it is not needed yet). Go is slower still on this
  program, at 617 ms. Its collector is built for short pauses beside a running program, not for
  the throughput of a small single-threaded one.

### Tin: what a language without a collector gets

Tin's programs are Minnow's written as C: the same compiler, optimizer and code generator, without
the collector, bounds checks and references.

- **binarytrees** (384 ms, clang -O0's 341, Minnow's 462). Tin allocates with C's `calloc` and
  frees each tree as C does, so it does what the C program does; the collector's time is gone. In
  the JIT it takes 910 ms: `lang run` runs `main` on a thread of its own, and glibc's allocator is
  slower on a thread's arena than on the main one for this pattern of allocating and freeing.
- **nbody** (153 ms against 177 at clang -O0, and 96 ms without the HIR optimizer: this run's
  noise is that large). The bodies are five structs in one block of memory, reached through
  pointers; Minnow reaches objects through a checked array of references.
- **sieve** (98 ms): bytes from `alloc` indexed without checks, between clang -O0 and -O2.
- **fib** is calls and the same as Minnow's.

### Kaleidoscope: A0's optimizer against LLVM's passes

The tutorial's chapter 7, as of LLVM 18, runs four function passes on each function: instcombine,
reassociate, GVN and simplifycfg. It has no inliner and no tail-call elimination. Although the
chapter is about mem2reg, its code does not run it either, so every argument and variable stays in
a stack slot. As a result, `mandel`'s inner loop calls the user-defined `|` and `>` operators on
every iteration and recurses for every step.

A0's HIR optimizer inlines small functions like those operators. That makes `mandel` faster than
the tutorial's code (945 against 1090 ms in this run); without it, A0 is about as fast (1002 ms).
On `fib` there is nothing to inline, and both run in 12–17 ms.

LLVM's standard -O2 pipeline, given the whole program, does everything: it inlines the operators
and `mandelconverge`, turns the recursion into loops, promotes every variable to a register and
vectorizes. It runs `mandel` in 231 ms, 4 times faster than A0, which shows what a full
optimizing pipeline buys over Cranelift's per-function optimization and A0's inlining. Its compile
time is the same as the tutorial's on these small programs, 25–41 ms.

How LLVM's times were split: its JIT compiles each function when a call first needs it. Its compile
time is therefore measured as the same program with the last line's calls doing no work
(`printd(fib(1))`), and the rest of the full run counts as running.

### Lox: dynamic objects with shapes and caches

A0's Lox compiles every Lox value to `any`. Lox's semantics (calls, classes, properties) are HIR
functions in its prelude, which the optimizer inlines and specializes. The optimizer matters more
here than anywhere else: without it, `fib` takes 2591 ms and is slower than clox.

- **`fib`** (468 ms against 1165 ms). Once the prelude's call functions are inlined, arithmetic and
  comparisons are native code, and a call is a few type tests and an indirect call; clox dispatches
  bytecode for every operation.
- **Fields and methods** are HIR dynamic objects (D66): an instance's fields and a class's methods
  are slots found through the object's shape, and every place in the program that names a field
  keeps a cache of the shape it saw last, so a property access is a comparison and a load. clox
  hashes the name, which it interns, into each instance's table. `zoo` (property access) runs in
  231 ms against 364 ms, `method_call` in 172 against 220.
- **Method calls** look the method up first and call it on the object, allocating no bound method,
  as clox's `OP_INVOKE` does; `invocation`, which is mostly calls of methods with arguments, is
  slower than clox (408 against 329 ms).
- **`binary_trees`** (1191 ms against 3169 ms). An instance is its struct and one object of
  slots; the collector takes 316 ms of it.
- **`equality`** compares constants and throws the results away. A0's optimizer removes the
  comparisons, so its 106 ms is the loop alone.
- **Interpreted**, A0 is 1.3–50 times slower than clox. A0's interpreter runs the core IR and is
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
