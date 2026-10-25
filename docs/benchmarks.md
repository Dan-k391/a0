# Benchmarks

A small, honest picture of where A0 stands against established tools: how long its languages take
to compile and how fast what they compile runs. It is not a CI check and nothing was tuned for it:
the programs are the obvious ones, written the same way in each language. `bench/run.py`
reproduces everything here (`bench/README.md`); the tables below are its output for v0.4.0.

## In short

- **Compiling.** A0's languages build a native executable in 110–160 ms: 20–35 ms more than clang
  -O0 takes for the same program in C, and less than `go build`. Almost all of it is the system
  linker. A0's own work, from source to object file, takes 2–6 ms on these programs. Linking
  through `cc` (gcc's driver on Ubuntu) takes about 105 ms even for a plain C object, and through
  clang about 72 ms. With `CC=clang`, Minnow builds in 76–85 ms.
- **Typed code (Minnow) against C and Go.** On calls and floating point A0 runs at the speed of
  clang -O0 and close to Go, and half as fast as clang -O2. On arrays it is 3 times slower than C,
  because of A0's array representation. On allocation it is half as fast as malloc and free, and
  1.6 times faster than Go.
- **Kaleidoscope against LLVM's tutorial.**
  - Size: A0's Kaleidoscope is 502 lines against the tutorial's 901. Besides, A0's driver gives it
    an interpreter, native executables, a language server, a debugger and a REPL.
  - Speed: its native code is as fast as LLVM's JIT on `fib`, and 1.6 times faster on `mandel`.
    The gain on `mandel` is A0's HIR optimizer inlining the operators the program defines; without
    the optimizer, A0 is 8% slower than LLVM.
  - Startup: LLVM's JIT starts running within 13–23 ms, while an A0 executable takes 110 ms to
    build.
- **Lox against clox.** Compiled with A0, Lox is 2.9 times faster than clox on `fib` and even on
  `binary_trees`. It is 1.3–1.7 times slower on method calls, function calls through values and
  property access, because of how A0's Lox looks up names. Interpreted, A0 is 14–48 times slower
  than clox.

## Tools

- Machine: x86_64, Linux-6.6.114.1-microsoft-standard-WSL2-x86_64-with-glibc2.39, 32 threads
  (WSL 2 on Windows 11)
- CPU: AMD Ryzen 9 9955HX3D 16-Core Processor
- A0: v0.4.0, rustc 1.99.1 (58cf8f9c3 2026-10-13), Cranelift 0.137, release build
- Ubuntu clang version 18.1.3 (1ubuntu1)
- `cc`, which A0 links with: cc (Ubuntu 13.3.0-6ubuntu2~24.04.1) 13.3.0
- LLVM 18.1.3 (the Kaleidoscope tutorial, chapter 7, built with `clang++ -O2`)
- go version go1.22.2 linux/amd64
- clox at craftinginterpreters 4a840f70f69c, built with `clang -O2 -std=c99 -w`
- Times are medians of 5 runs (wall clock, so each includes starting a process, about 1 ms);
  compile times include linking. A0's interpreter ran 2 times per program.

How each tool was run:

| | Compile | Run |
|---|---|---|
| A0's languages | `minnow build prog.mn -o prog` (likewise `lox`, `kaleidoscope`); `A0_OPT=0` for no HIR optimizer | `./prog`; interpreted: `minnow run prog.mn` |
| C | `clang -O0 prog.c -lm -o prog`, `clang -O2 ...` | `./prog` |
| Go | `go build -o prog prog.go`, the standard library cached | `./prog` |
| clox | | `clox prog.lox` |
| LLVM's Kaleidoscope | `toy < prog.k`, the same program with its calls doing no work (below) | `toy < prog.k`, less the compile time |

## Kaleidoscope: A0 against LLVM's tutorial

Lines of code (without blank lines and comments):
- A0's Kaleidoscope: 502 (grammar, compiler, driver, runtime).
- LLVM's chapter 7: 901 (`toy.cpp` and `KaleidoscopeJIT.h`).

| Program | A0 build | no HIR optimizer | A0 native run | no HIR optimizer | A0 interpreted | LLVM JIT: startup and compile | LLVM JIT: run |
|---|---|---|---|---|---|---|---|
| fib | 116 ms | 112 ms | 7 ms | 7 ms | 201 ms | 13 ms | 6 ms |
| mandel | 110 ms | 118 ms | 250 ms | 435 ms | 8221 ms | 23 ms | 404 ms |

`fib` is naive recursive Fibonacci (`fib(32)`). `mandel` is the tutorial's own Mandelbrot iteration
over a 1000 × 1000 grid, summed. It uses the operators `|` and `>` that the program defines itself,
and a recursive function that iterates.

## Minnow against C and Go

| Program | Minnow build | no HIR optimizer | clang -O0 | clang -O2 | go build | Minnow run | no HIR optimizer | clang -O0 | clang -O2 | Go |
|---|---|---|---|---|---|---|---|---|---|---|
| fib | 126 ms | 110 ms | 88 ms | 92 ms | 180 ms | 35 ms | 34 ms | 31 ms | 17 ms | 31 ms |
| sieve | 114 ms | 113 ms | 91 ms | 97 ms | 161 ms | 84 ms | 80 ms | 46 ms | 28 ms | 34 ms |
| nbody | 121 ms | 119 ms | 94 ms | 113 ms | 171 ms | 76 ms | 76 ms | 84 ms | 38 ms | 56 ms |
| binarytrees | 114 ms | 111 ms | 92 ms | 102 ms | 160 ms | 245 ms | 249 ms | 146 ms | 126 ms | 402 ms |

The programs:
- `fib` is calls (`fib(35)`).
- `sieve` is arrays: the primes below 2,000,000, ten times.
- `nbody` is floating point and structs: the Benchmarks Game's n-body, 1,000,000 steps.
- `binarytrees` is allocation: the Benchmarks Game's binary-trees at depth 16. The C version uses
  malloc and free.

A0's collector in `binarytrees` (`A0_GC_STATS=1`): 110 collections, 101.1 ms (71.9 marking), peak
19.6 MB, 230019 objects live after the last.

Where Minnow's build time goes (`cc` is x86_64-linux-gnu-gcc-13). Each of A0's steps includes the
ones before it; the object file is made from the IR text with `a0 obj`.

| Program | to HIR | to IR | object file | executable | executable, `CC=clang` | cc linking a C object | clang linking it |
|---|---|---|---|---|---|---|---|
| fib | 1 ms | 1 ms | 2 ms | 112 ms | 76 ms | 105 ms | 72 ms |
| nbody | 5 ms | 6 ms | 6 ms | 135 ms | 85 ms | 109 ms | 73 ms |

## Lox: A0 against clox

| Program | A0 build | no HIR optimizer | A0 native run | no HIR optimizer | A0 interpreted | clox |
|---|---|---|---|---|---|---|
| fib | 124 ms | 124 ms | 183 ms | 790 ms | 12.8 s | 529 ms |
| binary_trees | 144 ms | 128 ms | 1546 ms | 1914 ms | 23.0 s | 1614 ms |
| method_call | 153 ms | 127 ms | 142 ms | 184 ms | 4606 ms | 113 ms |
| invocation | 158 ms | 132 ms | 284 ms | 360 ms | 7939 ms | 164 ms |
| equality | 128 ms | 124 ms | 69 ms | 365 ms | 2136 ms | 2609 ms |
| zoo | 139 ms | 128 ms | 274 ms | 364 ms | 7809 ms | 172 ms |

The programs are the benchmarks of Crafting Interpreters' own test suite, with two left out:
- `string_equality`: clox rejects it, because it needs more than 256 constants in one chunk.
- `zoo_batch`: it runs for ten seconds rather than doing a fixed amount of work.

A0's collector in `binary_trees` (`A0_GC_STATS=1`): 227 collections, 779.9 ms (587.4 marking), peak
68.0 MB, 122773 objects live after the last.

## Why

### Compile time is the linker's

A0 compiles these programs to an object file in 2–6 ms: parsing, building and checking the HIR,
optimizing, lowering and Cranelift together.

The rest of a build, over 100 ms, is linking. A0 runs the platform's C compiler driver (`cc`) on
the object file and the runtime library. The driver accounts for most of the difference:

| Linking with | A C object | A Minnow build |
|---|---|---|
| `cc` (gcc 13's driver on Ubuntu) | 105–109 ms | 112–135 ms |
| clang | 72–73 ms | 76–85 ms (`CC=clang`) |

The runtime library adds little. So against clang -O0, which compiles C in about 20 ms and links
with its own driver, an A0 build costs about 20–35 ms more. The HIR optimizer does not change
build times measurably.

`go build` takes 160–180 ms even with Go's standard library cached. LLVM's JIT is not comparable
here: it compiles in memory and never links. A0's way to run a program at once is its interpreter.

### Minnow: Cranelift is clang -O0, not -O2

A0 hands per-function optimization to Cranelift (D27). Cranelift's code runs at about clang -O0's
speed: 35 ms against 31 ms on `fib`, and 76 ms against 84 ms on `nbody`. clang -O2 is twice as
fast on both. -O2 runs LLVM's whole pipeline (inlining, loop and vector optimizations), which
Cranelift does not attempt; on `fib`, for example, it turns one of the two recursive calls into a
loop.

The HIR optimizer changes nothing on these programs (the "no HIR optimizer" column). They have no
small functions to inline and no dynamic types to fold.

Where A0 loses more, it is A0's own design rather than its code generator.

- **sieve** (84 ms against 28 ms for C at -O2 and 34 ms for Go). Three things add up:
  - A0's arrays hold 64-bit words, so the 2,000,001 booleans take 16 MB against C's 2 MB.
  - Minnow has no way to allocate an array of n elements, so the program grows it with `push`, one
    runtime call per element.
  - Every index is bounds-checked. Go checks bounds too, but its array of bytes is allocated once.
- **nbody** (76 ms against 38 ms at -O2 and 56 ms for Go).
  - The bodies are structs on the heap, reached through a bounds-checked array of references.
  - `math.sqrt` is a call to C's `sqrt` instead of an instruction.
- **binarytrees** (245 ms against 126 ms for malloc and free). 41% of A0's time is collection: 101 ms
  over 110 collections, 72 ms of it marking. The collector is a simple non-moving mark and sweep,
  so it marks every live object at every collection, the long-lived tree included. Making that
  cheaper would need a generational collector, which in turn needs precise knowledge of every
  reference (M19). Go is slower still on this program, at 402 ms. Its collector is built for short
  pauses beside a running program, not for the throughput of a small single-threaded one.

### Kaleidoscope: A0's optimizer against the tutorial's passes

The tutorial's chapter 7, as of LLVM 18, runs four function passes on each function: instcombine,
reassociate, GVN and simplifycfg. It has no inliner and no tail-call elimination. Although the
chapter is about mem2reg, its code does not run it either, so every argument and variable stays in
a stack slot. As a result, `mandel`'s inner loop calls the user-defined `|` and `>` operators on
every iteration and recurses for every step.

A0's HIR optimizer inlines small functions like those operators. That makes `mandel` 1.6 times
faster than LLVM's code; without it, A0 is 8% slower (435 ms against 404 ms). On `fib` there is
nothing to inline, and both run in 6–7 ms.

How LLVM's times were split: its JIT compiles each function when a call first needs it. Its compile
time is therefore measured as the same program with the last line's calls doing no work
(`printd(fib(1))`), and the rest of the full run counts as running.

### Lox: compiled through the HIR, but names are looked up by string

A0's Lox compiles every Lox value to `any`. Lox's semantics (calls, classes, properties) are HIR
functions in its prelude, which the optimizer inlines and specializes. The optimizer matters more
here than anywhere else: without it, `fib` takes 790 ms and is slower than clox.

- **Faster than clox** on `fib` (183 ms against 529 ms). Once the prelude's call functions are
  inlined, arithmetic and comparisons are native code, and a call is a few type tests and an
  indirect call; clox dispatches bytecode for every operation. `binary_trees` is even (1546 ms
  against 1614 ms), and half of A0's time there is its collector (780 ms over 227 collections).
- **Slower than clox** on method calls (1.26 times), calls through variables (`invocation`, 1.73
  times) and property access (`zoo`, 1.59 times). These are choices of A0's Lox, not limits of A0,
  and the obvious next optimizations for it:
  - A property access is runtime calls into maps: is the name one of the instance's fields
    (`has`, then the value), and if not, one of its class's methods. Strings keep their hash and
    the map answers the second call from a one-entry cache, but every lookup is a call that
    compares keys as strings.
  - A method call `o.m()` first makes a bound method, which allocates, and then calls it through
    the generic call path: a type test, an arity check, and the call depth counted in a global.
  - clox interns strings, so its tables compare pointers. It calls methods with one instruction
    (`OP_INVOKE`) and allocates nothing.
- **`equality`** compares constants and throws the results away. A0's optimizer removes the
  comparisons, so its 69 ms is the loop alone. Even without the optimizer (365 ms), the comparisons
  are inline native code, while clox evaluates every one.
- **Interpreted**, A0 is 14–48 times slower than clox. A0's interpreter runs the core IR and is
  not tuned for speed. It is there to run a program at once and to check the
  native backend against: every test runs both, and they must agree. clox is a tuned bytecode
  virtual machine. (On `equality` the interpreter is faster than clox, because the optimizer has
  already removed the work.)

## What these numbers do not say

- They come from one machine (WSL 2 on a laptop) and from small programs written the obvious way.
  Differences of a few milliseconds are within run-to-run noise.
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
