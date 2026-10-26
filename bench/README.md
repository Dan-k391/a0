# Benchmarks

The programs of A0's benchmark and the script that runs it; `docs/benchmarks.md` has the results
and what they mean.

| Directory | What |
|---|---|
| `kaleidoscope/` | Kaleidoscope programs that run on A0's Kaleidoscope and on LLVM's tutorial (chapter 7) |
| `minnow/`, `c/`, `go/` | The same programs in Minnow, C and Go: calls (`fib`), arrays (`sieve`), floating point and structs (`nbody`), allocation (`binarytrees`) |
| `minnow/parallel.mn` | Threads: the primes below a limit counted on 1, 2, 4 and 8 threads |
| `tin/` | The same programs in Tin, A0's systems language, written as the C ones are (pointers, C's allocator) |
| `run.py` | Builds everything, checks that the implementations print the same, and times compiling and running |

Lox is measured on the benchmark programs of Crafting Interpreters' own test suite
(`examples/lox/tests/craftinginterpreters/test/benchmark`), against clox.

```
sudo apt install clang llvm-dev zlib1g-dev libzstd-dev golang-go   # Ubuntu; or the same from Homebrew
python3 bench/run.py --runs 5
```

The script builds A0 and its languages in release mode, fetches clox (a pinned commit of Crafting
Interpreters) and chapter 7 of LLVM's Kaleidoscope tutorial (for the installed LLVM's version) into
`target/bench`, checks that every implementation prints the same, and writes the tables of
`docs/benchmarks.md` to `target/bench/results.md`. Run it on a quiet machine; it takes about ten
minutes, most of them in A0's interpreter.
