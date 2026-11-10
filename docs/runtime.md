# The runtime

Every A0 program runs with A0's runtime, `a0rt`: a library of C code with the garbage collector,
strings, arrays, maps, threads, output, failures and stack traces. The interpreter and the JIT
call it, and executables link it. Its header is `a0rt.h`, in `crates/a0-runtime/include` (and in
`include/` of a release archive).

You need this page when your language has a runtime of its own in C: functions that its programs
call and that you write in C. Kaleidoscope has one for `putchard` and `printd`, and Tin has one
that makes the cJSON library available to its programs.

Most of `a0rt.h` is the contract between the code A0 generates and the runtime, and it may change
in any release. A language's C runtime may use only the small part listed in
[What a C runtime may use](#what-a-c-runtime-may-use). That part is covered by A0's
[compatibility promise](compatibility.md).

## An example: Kaleidoscope's `putchard`

This is Kaleidoscope's whole runtime, `examples/kaleidoscope/runtime/kaleidoscope.c`:

```c
/* Kaleidoscope's runtime library: the helpers the LLVM tutorial's programs call through `extern`.
 * Unlike the tutorial, output goes to stdout rather than stderr. It goes through a0_rt_write, as
 * the A0 runtime's own does, so that a debugger running the program can take it. */
#include "a0rt.h"

#include <stdio.h>

/* putchard - putchar that takes a double and returns 0. */
double putchard(double x) {
    uint8_t c = (uint8_t)(char)x;
    a0_rt_write(1, &c, 1);
    return 0;
}

/* printd - printf that takes a double, prints it as "%f\n", and returns 0. */
double printd(double x) {
    char text[512];
    int n = snprintf(text, sizeof text, "%f\n", x);
    if (n > (int)sizeof text - 1) n = (int)sizeof text - 1;
    if (n > 0) a0_rt_write(1, (const uint8_t *)text, (size_t)n);
    return 0;
}

const A0Symbol kaleidoscope_symbols[] = {
    {"putchard", (void *)&putchard},
    {"printd", (void *)&printd},
    {NULL, NULL},
};
```

It uses two things from `a0rt.h`:

- `a0_rt_write` writes the output. A debugger that runs the program takes its output, so a runtime
  writes through `a0_rt_write` rather than to `stdout`.
- `A0Symbol` is the type of `kaleidoscope_symbols`, a table of the functions by name. The
  interpreter and the JIT find the functions through it.

A Kaleidoscope program declares the functions with `extern` and calls them. `hi.k`:

```
extern putchard(c);
extern printd(x);

putchard(72);
putchard(105);
putchard(10);
printd(1.5);
```

```
$ kaleidoscope run hi.k
Hi
1.500000
```

`kaleidoscope build hi.k -o hi` makes an executable that prints the same, with the runtime linked
into it.

## Building a C runtime

The steps below are Kaleidoscope's. Tin's are the same, with three C files.

**1. Depend on `a0-runtime` and `a0-build`.** In the language's `Cargo.toml`, next to its other
A0 crates, pinned to the same release:

```toml
[dependencies]
# Only for its C header, which a0-build compiles the runtime against.
a0-runtime = { git = "https://github.com/Dan-k391/a0", tag = "v1.0.0" }

[build-dependencies]
a0-build = { git = "https://github.com/Dan-k391/a0", tag = "v1.0.0" }
```

The language depends on `a0-runtime` only for its header. The dependency must be direct, because
Cargo tells only a crate's direct dependents where its files are.

**2. Compile the C files in `build.rs`:**

```rust
fn main() {
    a0_build::runtime_library("kaleidoscope_rt", &["runtime/kaleidoscope.c"]);
}
```

`runtime_library` compiles the files against `a0rt.h` into a static library named
`kaleidoscope_rt`. It links the library into the language's own binary, so that the interpreter
and the JIT can call its functions. It also keeps a copy, `kaleidoscope_rt.embedded` in Cargo's
`OUT_DIR`, for linking executables.

**3. Give the library and the symbol table to the driver.** In `src/lib.rs`:

```rust
use a0_driver::backend::{A0Symbol, HostSymbols, Library};

/// The Kaleidoscope runtime library (`putchard`, `printd`), for linking native executables.
pub fn runtime_library() -> Library {
    Library::new(
        "kaleidoscope_rt",
        include_bytes!(concat!(env!("OUT_DIR"), "/kaleidoscope_rt.embedded")),
    )
}

unsafe extern "C" {
    static kaleidoscope_symbols: A0Symbol;
}

/// Host symbols for the interpreter: the A0 runtime, the C library and Kaleidoscope's runtime.
pub fn host_symbols() -> HostSymbols {
    let mut symbols = HostSymbols::new();
    // SAFETY: `kaleidoscope_symbols` is a NULL-terminated table defined in runtime/kaleidoscope.c.
    unsafe { symbols.add_table(&raw const kaleidoscope_symbols) };
    symbols
}
```

`Library` is the library's bytes, embedded in the language's binary, which the driver links into
every executable it builds. `HostSymbols::new()` already has A0's runtime and finds the C library's
functions; `add_table` adds the language's own.

In `src/main.rs`, `Language::runtime` hands both to the driver:

```rust
Language::new("kaleidoscope", /* the compiler */)
    .runtime(kaleidoscope::host_symbols, || {
        vec![kaleidoscope::runtime_library()]
    })
    .main()
```

**4. Declare the functions in the compiler.** Kaleidoscope's compiler declares each `extern` with
`HirBuilder::declare_extern`, as a plain C function. A plain C function takes and returns C
values: numbers, `bool`, pointers, strings as `const char *`, arrays as pointers to their
elements. [The high-level IR](hir.md#module) gives the C type for each HIR type.

A function declared `extern "a0"` instead takes and returns A0's own values; see
[A0's values](#a0s-values).

## What a C runtime may use

These parts of `a0rt.h` are covered by the [compatibility promise](compatibility.md):

| Part of `a0rt.h` | For |
|---|---|
| `A0Symbol` | the table of functions that the interpreter and the JIT call by name: `{name, address}` pairs, ending with `{NULL, NULL}` |
| `a0_rt_write(stream, bytes, len)` | output: stream 1 is standard output, 2 is standard error. A debugger running the program takes it, so a runtime writes through it rather than to `stdout` |
| `a0_print_i64`, `a0_print_f64`, `a0_print_bytes`, `a0_print_cstr`, `a0_print_newline`, `a0_eprint_bytes`, `a0_eprint_cstr`, `a0_putchar` | printing values as A0's `print` does |
| `a0_format_i64(value, buf)`, `a0_format_f64(value, buf)` | the same text written into `buf`, which must have at least 24 bytes for `a0_format_i64` and 32 for `a0_format_f64`; both return the length |
| `a0_rt_flush()`, `a0_rt_exit(code)` | flushing standard output and standard error; ending the program with `code`, its output flushed |
| `a0_rt_argc()`, `a0_rt_argv(i)` | the program's arguments; `a0_rt_argv` returns `NULL` past the last |
| `a0_panic(message)`, `A0_PANIC_EXIT_CODE` | failing: prints `panic: <message>` on standard error and exits with `A0_PANIC_EXIT_CODE`, 101, as a failing program does. It prints no stack trace |
| `a0_alloc(size)`, `a0_realloc(ptr, size)`, `a0_free(ptr)` | memory that the collector does not manage. `a0_alloc` fills it with zeros. Both panic when memory runs out |
| `a0_clock()` | the processor time the program has used, in seconds, as the `clock()` builtin gives programs |
| `A0_NORETURN` | marks a function that does not return |

The header declares each with its exact C types.

## A0's values

A function declared `extern "a0"` takes and returns A0's own values: an `A0String *` for a
`string`, an `A0Array *` for an array. For such functions, these are covered too:

- the layout of `A0String`: `int64_t len`, then `uint64_t hash` (0 until the runtime computes it),
  then the `len` bytes, `uint8_t bytes[]`;
- the layout of `A0Array`: `int64_t len`, `int64_t cap`, `data` (a pointer to the elements, held in
  a 64-bit word on every platform), `int64_t elem_kind`;
- `a0_string_new(bytes, len)`, `a0_string_len(s)` and `a0_array_new(elem_kind, capacity)`.

Nothing else about values is covered: not the collector's header, type descriptors, how `any` is
encoded, maps or objects.

**Allocating.** The collector may run whenever a function allocates. While an `extern "a0"`
function runs, its arguments stay rooted: the collector knows about them and keeps them. An object
the function allocates itself is not rooted, so a second allocation may free it. Only the
uncovered parts of `a0rt.h` can root an object. So a C runtime's function either allocates once,
or returns memory that C owns.
