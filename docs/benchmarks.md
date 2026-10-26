# Benchmarks

A small, honest picture of where A0 stands against established tools: how long its languages take
to compile, how fast what they compile runs, and how fast `lang run` runs a program at once. It is
not a CI check and nothing was tuned for it: the programs are the obvious ones, written the same
way in each language. `bench/run.py` reproduces everything here (`bench/README.md`); the tables
below are its output for v0.5.0, with v0.4.0's numbers beside them where they changed.

## In short

- **Running at once.** `lang run` compiles into memory with Cranelift's JIT (D63) instead of
  interpreting: Kaleidoscope's `mandel` takes 283 ms with `kaleidoscope run` (v0.4.0: 8.2 s
  interpreted), within 7% of the executable. Minnow's programs run in the JIT as fast as linked.
  Lox's JIT runs include compiling its prelude, 20–70 ms. The interpreter is still there
  (`A0_INTERP=1`), 14–50 times slower than clox, as the reference and the debugger's engine.
- **Compiling.** A0's own work, from source to object file, is 2–6 ms; the rest of a build is the
  system linker. This run's machine linked slowly (gcc's `cc` took 183 ms to link even a plain C
  object, against 105 ms in v0.4.0's run), so builds took 180–260 ms; with `CC=clang`, 113–119 ms.
- **Typed code (Minnow) against C and Go.** On calls and floating point A0 runs at clang -O0's
  speed and close to Go, half as fast as clang -O2. On arrays it now runs at clang -O0's speed too
  (`sieve` 84 → 46 ms): arrays are allocated at a size and booleans take a byte. On allocation it is
  1.6 times slower than malloc and free and 1.8 times faster than Go; its collector's share fell
  from 101 to 65 ms (D64).
- **Kaleidoscope against LLVM.**
  - Size: A0's Kaleidoscope is 502 lines against the tutorial's 901. Besides, A0's driver gives it
    a JIT, an interpreter, native executables, a language server, a debugger and a REPL.
  - Against the tutorial's passes, A0's code is as fast on `fib` and 1.6 times faster on `mandel`,
    thanks to the HIR optimizer's inlining.
  - Against LLVM's standard -O2 pipeline on the whole program, A0 is 3 times slower on `fib` and
    2.2 times slower on `mandel`: -O2 inlines, unrolls and vectorizes, which Cranelift does not.
- **Lox against clox.** Compiled with A0, Lox is now faster than clox on every benchmark: 2.7 times
  on `fib`, 3.5 times on `binary_trees`, 1.5 times on `method_call` and `zoo`, even on `invocation`.
  v0.4.0 was 1.3–1.7 times slower on method calls and property access; fields and methods are now
  found through shapes and caches, and a method call allocates nothing (D66).

## Tools

- Machine: x86_64, Linux-6.6.114.1-microsoft-standard-WSL2-x86_64-with-glibc2.39, 32 threads
  (WSL 2 on Windows 11)
- CPU: AMD Ryzen 9 9955HX3D 16-Core Processor
- A0: v0.5.0, rustc 1.99.1 (58cf8f9c3 2026-10-13), Cranelift 0.137, release build
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
| fib | 237 ms | 236 ms | 9 ms | 8 ms | 11 ms | 293 ms | 19 ms | 8 ms | 20 ms | 3 ms |
| mandel | 256 ms | 232 ms | 265 ms | 570 ms | 283 ms | 9709 ms | 22 ms | 415 ms | 21 ms | 118 ms |

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
| fib | 185 ms | 184 ms | 123 ms | 127 ms | 212 ms | 39 ms | 40 ms | 35 ms | 32 ms | 18 ms | 32 ms |
| sieve | 185 ms | 179 ms | 125 ms | 133 ms | 203 ms | 46 ms | 49 ms | 50 ms | 47 ms | 29 ms | 37 ms |
| nbody | 194 ms | 196 ms | 130 ms | 150 ms | 201 ms | 79 ms | 79 ms | 88 ms | 88 ms | 40 ms | 58 ms |
| binarytrees | 185 ms | 184 ms | 127 ms | 140 ms | 212 ms | 224 ms | 219 ms | 217 ms | 156 ms | 137 ms | 396 ms |

The programs:
- `fib` is calls (`fib(35)`).
- `sieve` is arrays: the primes below 2,000,000, ten times.
- `nbody` is floating point and structs: the Benchmarks Game's n-body, 1,000,000 steps.
- `binarytrees` is allocation: the Benchmarks Game's binary-trees at depth 16. The C version uses
  malloc and free.

A0's collector in `binarytrees` (`A0_GC_STATS=1`): 56 collections, 65.1 ms (49.5 marking), peak
22.4 MB, 240329 objects live after the last (v0.4.0: 110 collections, 101.1 ms, peak 19.6 MB).

Where Minnow's build time goes (`cc` is x86_64-linux-gnu-gcc-13). Each of A0's steps includes the
ones before it; the object file is made from the IR text with `a0 obj`.

| Program | to HIR | to IR | object file | executable | executable, `CC=clang` | cc linking a C object | clang linking it |
|---|---|---|---|---|---|---|---|
| fib | 1 ms | 1 ms | 2 ms | 188 ms | 113 ms | 183 ms | 111 ms |
| nbody | 5 ms | 6 ms | 5 ms | 191 ms | 119 ms | 180 ms | 107 ms |

## Lox: A0 against clox

| Program | A0 build | no HIR optimizer | A0 native run | no HIR optimizer | A0 JIT (`lox file`) | A0 interpreted | clox |
|---|---|---|---|---|---|---|---|
| fib | 194 ms | 196 ms | 204 ms | 932 ms | 222 ms | 13.6 s | 554 ms |
| binary_trees | 223 ms | 197 ms | 480 ms | 1161 ms | 514 ms | 23.8 s | 1673 ms |
| method_call | 251 ms | 202 ms | 81 ms | 177 ms | 153 ms | 4855 ms | 118 ms |
| invocation | 263 ms | 202 ms | 173 ms | 319 ms | 245 ms | 7737 ms | 171 ms |
| equality | 184 ms | 181 ms | 69 ms | 358 ms | 87 ms | 2142 ms | 2635 ms |
| zoo | 200 ms | 175 ms | 110 ms | 323 ms | 143 ms | 8077 ms | 163 ms |

The programs are the benchmarks of Crafting Interpreters' own test suite, with two left out:
- `string_equality`: clox rejects it, because it needs more than 256 constants in one chunk.
- `zoo_batch`: it runs for ten seconds rather than doing a fixed amount of work.

A0's collector in `binary_trees` (`A0_GC_STATS=1`): 144 collections, 138.5 ms (98.3 marking), peak
20.9 MB, 96759 objects live after the last (v0.4.0: 227 collections, 779.9 ms, peak 68.0 MB).

## From v0.4.0 to v0.5.0

Native run times, v0.4.0 → v0.5.0. This run's machine was a little slower overall (clox 529 → 554
ms on `fib`), so differences of 10% are noise.

| Program | v0.4.0 | v0.5.0 | Why |
|---|---|---|---|
| Minnow `sieve` | 84 ms | 46 ms | `filled` allocates the array at once; `array<bool>` takes a byte per element (D65) |
| Minnow `binarytrees` | 245 ms | 224 ms | 16-byte headers, prefetched marking, a heap three times what survives (D64) |
| Lox `binary_trees` | 1546 ms | 480 ms | an instance is two objects instead of four; collection 780 → 139 ms (D64, D66) |
| Lox `zoo` | 274 ms | 110 ms | fields found through shapes and per-site caches (D66) |
| Lox `method_call` | 142 ms | 81 ms | no bound method allocated per call (D66) |
| Lox `invocation` | 284 ms | 173 ms | the same |
| Kaleidoscope `mandel`, `run` | 8221 ms (interpreted) | 283 ms (JIT) | `run` compiles into memory (D63) |

Unchanged within noise: Minnow `fib` and `nbody`, Lox `fib` and `equality`, Kaleidoscope native.

## Why

### Compile time is the linker's

A0 compiles these programs to an object file in 2–6 ms: parsing, building and checking the HIR,
optimizing, lowering and Cranelift together. The rest of a build is linking: A0 runs the platform's
C compiler driver (`cc`) on the object file and the runtime library, and the driver accounts for
most of it. In this run gcc's driver took 180–183 ms to link a plain C object and clang 107–111 ms;
A0's builds took about as long as linking with each (`CC=clang`: 113–119 ms). Against clang -O0,
which compiles C in about 20 ms and links with its own driver, an A0 build costs little more.
`go build` takes 200–212 ms even with Go's standard library cached.

For running a program at once, A0 no longer links: `lang run` compiles into memory, so
Kaleidoscope's `fib` runs in 11 ms all told and Minnow's programs as fast as their executables. A
Lox program first compiles the prelude functions it reaches, which costs 20–70 ms over the
executable.

### Minnow: Cranelift is clang -O0, not -O2

A0 hands per-function optimization to Cranelift (D27). Cranelift's code runs at about clang -O0's
speed: 39 ms against 32 ms on `fib`, 79 ms against 88 ms on `nbody`, and now 46 ms against 47 ms on
`sieve`. clang -O2 is about twice as fast. -O2 runs LLVM's whole pipeline (inlining, loop and vector
optimizations), which Cranelift does not attempt; on `fib`, for example, it turns one of the two
recursive calls into a loop.

The HIR optimizer changes nothing on these programs (the "no HIR optimizer" column). They have no
small functions to inline and no dynamic types to fold.

- **sieve** (46 ms against 29 ms for C at -O2 and 37 ms for Go). The array is now allocated at its
  size and holds a byte per boolean, as in C; what remains is Cranelift's code and the bounds
  check on every index, which Go has too.
- **nbody** (79 ms against 40 ms at -O2 and 58 ms for Go).
  - The bodies are structs on the heap, reached through a bounds-checked array of references.
  - `math.sqrt` is a call to C's `sqrt` instead of an instruction.
- **binarytrees** (224 ms against 137 ms for malloc and free). 29% of A0's time is collection: 65 ms
  over 56 collections, 50 ms of it marking. The collector is a non-moving mark and sweep, so it
  marks every live object at every collection, the long-lived tree included; v0.5.0 made each
  collection rarer and cheaper (D64), and a generational collector would avoid most of what is
  left. Go is slower still on this program, at 396 ms. Its collector is built for short pauses
  beside a running program, not for the throughput of a small single-threaded one.

### Kaleidoscope: A0's optimizer against LLVM's passes

The tutorial's chapter 7, as of LLVM 18, runs four function passes on each function: instcombine,
reassociate, GVN and simplifycfg. It has no inliner and no tail-call elimination. Although the
chapter is about mem2reg, its code does not run it either, so every argument and variable stays in
a stack slot. As a result, `mandel`'s inner loop calls the user-defined `|` and `>` operators on
every iteration and recurses for every step.

A0's HIR optimizer inlines small functions like those operators. That makes `mandel` 1.6 times
faster than the tutorial's code; without it, A0 is slower (570 ms against 415 ms). On `fib` there
is nothing to inline, and both run in 8–9 ms.

LLVM's standard -O2 pipeline, given the whole program, does everything: it inlines the operators
and `mandelconverge`, turns the recursion into loops, promotes every variable to a register and
vectorizes. It runs `mandel` in 118 ms and `fib` in 3 ms, 2.2 and 3 times faster than A0, which
shows what a full optimizing pipeline buys over Cranelift's per-function optimization and A0's
inlining. Its compile time is the same as the tutorial's on these small programs, about 20 ms.

How LLVM's times were split: its JIT compiles each function when a call first needs it. Its compile
time is therefore measured as the same program with the last line's calls doing no work
(`printd(fib(1))`), and the rest of the full run counts as running.

### Lox: dynamic objects with shapes and caches

A0's Lox compiles every Lox value to `any`. Lox's semantics (calls, classes, properties) are HIR
functions in its prelude, which the optimizer inlines and specializes. The optimizer matters more
here than anywhere else: without it, `fib` takes 932 ms and is slower than clox.

- **`fib`** (204 ms against 554 ms). Once the prelude's call functions are inlined, arithmetic and
  comparisons are native code, and a call is a few type tests and an indirect call; clox dispatches
  bytecode for every operation.
- **Fields and methods** are HIR dynamic objects (D66): an instance's fields and a class's methods
  are slots found through the object's shape, and every place in the program that names a field
  keeps a cache of the shape it saw last, so a property access is a comparison and a load. clox
  hashes the name, which it interns, into each instance's table. `zoo` (property access) runs in
  110 ms against 163 ms, `method_call` in 81 against 118.
- **Method calls** look the method up first and call it on the object, allocating no bound method,
  as clox's `OP_INVOKE` does: `invocation` is even with clox (173 against 171 ms).
- **`binary_trees`** (480 ms against 1673 ms). An instance is now its struct and one object of
  slots, where it was a struct, a map and the map's two tables; the collector's share fell from
  780 to 139 ms.
- **`equality`** compares constants and throws the results away. A0's optimizer removes the
  comparisons, so its 69 ms is the loop alone.
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
