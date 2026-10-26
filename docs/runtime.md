# The runtime, for languages' C runtimes

Programs run with A0's runtime, `a0rt` (`crates/a0-runtime`): the collector, strings, arrays, maps,
threads, output, failures and stack traces. Most of its header, `a0rt.h`, is the contract between
the code A0 generates and the runtime, and changes with code generation. A language that ships a
runtime of its own in C (Kaleidoscope's `putchard`, Tin's bindings to cJSON) uses a small part of
it, listed here; that part is covered by A0's compatibility promise (`docs/compatibility.md`).
Everything else in `a0rt.h` may change in any release.

## Building a C runtime

A language's `build.rs` compiles its C files against `a0rt.h` with `a0-build`:

```rust
fn main() {
    a0_build::runtime_library("kaleidoscope_rt", &["runtime/kaleidoscope.c"]);
}
```

The crate depends on `a0-runtime` (for its header: Cargo passes its location only to direct
dependents) and on `a0-build` as a build dependency. The library is embedded in the language's
binary and given to the driver with `Language::runtime`, together with a table of the functions
the interpreter may call:

```rust
use a0_driver::backend::{A0Symbol, HostSymbols, Library};

pub fn runtime_library() -> Library {
    Library::new("kaleidoscope_rt", include_bytes!(concat!(env!("OUT_DIR"), "/kaleidoscope_rt.embedded")))
}

unsafe extern "C" {
    static kaleidoscope_symbols: A0Symbol;
}

pub fn host_symbols() -> HostSymbols {
    let mut symbols = HostSymbols::new();
    // SAFETY: a NULL-terminated table defined in the C runtime.
    unsafe { symbols.add_table(&raw const kaleidoscope_symbols) };
    symbols
}
```

```c
const A0Symbol kaleidoscope_symbols[] = {
    {"putchard", (void *)putchard},
    {"printd", (void *)printd},
    {NULL, NULL},
};
```

## What a C runtime may use

| Part of `a0rt.h` | For |
|---|---|
| `A0Symbol` | the table of functions the interpreter calls by name |
| `a0_rt_write(stream, bytes, len)` | output (1 standard output, 2 standard error); a debugger running the program takes it, so a runtime writes through it rather than `stdout` |
| `a0_print_i64`, `a0_print_f64`, `a0_print_bytes`, `a0_print_cstr`, `a0_print_newline`, `a0_eprint_bytes`, `a0_eprint_cstr`, `a0_putchar` | printing values as A0's `print` does |
| `a0_format_i64`, `a0_format_f64` | the same text into a buffer |
| `a0_rt_flush`, `a0_rt_exit(code)` | ending the program from C with its output flushed |
| `a0_rt_argc`, `a0_rt_argv(i)` | the program's arguments |
| `a0_panic(message)`, `A0_PANIC_EXIT_CODE` | failing with a message and the stack trace, as a panic in the program does (exit 101) |
| `a0_alloc`, `a0_realloc`, `a0_free` | memory outside the collected heap, as `alloc` and `free` give programs |
| `a0_clock()` | seconds since an arbitrary start, as `clock()` |
| `A0_NORETURN` | marking functions that do not return |

Functions declared `extern "a0"` take A0's own values. For those, the layouts of `A0String`
(`len`, then the bytes) and `A0Array` (`len`, `cap`, `data`, `elem_kind`) and the functions
`a0_string_new`, `a0_string_len` and `a0_array_new` are covered too; anything else about values (the
collector's header, type descriptors, `any`'s encoding, maps and objects) is not. A function that
allocates must not keep pointers to objects across a second allocation without rooting them, which
only `a0rt.h`'s uncovered functions can do: allocate once, or return what C owns.
