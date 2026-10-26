# The standard library

Every language built on A0 can offer its programs the same standard library: files, standard
input and output, arguments and the environment, time, mathematics and string operations. It is a
set of HIR modules (`crates/a0-hir/std/*.hir`) over C functions of the runtime
(`crates/a0-runtime/c/std.c`), added to a program when the program imports one (decision D48), so
programs pay only for what they use, and it behaves the same interpreted and native.

## Using it from a language

A module is imported like any other (`docs/hir.md`, "Modules"):

```rust
b.import_module("std.fs", "fs");            // then fs.read(path), through call_member
b.import_item("std.str", "split");          // then split(s, ","), through call_named
```

A language with modules gets it with no code of its own: the driver treats imports of `std` and
`std.*` as the library's, never as files. In Minnow:

```
import std.fs;
import std.str as s;

fn main() {
  match fs.read("notes.txt") {
    Ok(text) => print(s.join(s.lines(text), " / ")),
    Err(e) => print("error: " + e),
  }
}
```

A language without modules imports what it needs itself (once, outside function bodies) and
calls the functions by member, or by their full names from HIR text added with `add_text`
(`` `std.fs.read`(path) ``), wrapping them in its own conventions. Lox's `natives.hir` does that: an
`Err` becomes a Lox exception.

## Errors

Operations that can fail return `std.Result<T, E>` (`enum Result<T, E> { Ok(T), Err(E) }` in
module `std`), with a message as the error: `cannot read notes.txt: No such file or directory`.
A program that wants to name the type imports `std` (`std.Result<string, string>`). Misusing a
function (an empty separator for `split`) panics instead.

## Modules

**`std`**: `Result<T, E>`.

**`std.io`**: standard input and output.

| Function | |
|---|---|
| `read_line() -> string?` | the next line without its line ending (`\n` or `\r\n`), null at the end of the input |
| `read_all() -> string` | the rest of the input |
| `write(text)`, `write_line(text)` | to standard output (without, with a newline) |
| `write_error(text)` | to standard error, after what was written to standard output |
| `flush()` | flushes standard output (reading input does that too) |

**`std.fs`**: files and directories; paths are UTF-8 (UTF-16 for Windows' own calls).

| Function | |
|---|---|
| `read(path) -> Result<string, string>` | a file's contents |
| `write(path, text) -> Result<bool, string>` | replaces a file's contents, creating it if needed |
| `append(path, text) -> Result<bool, string>` | adds to the end of a file |
| `exists(path) -> bool`, `is_dir(path) -> bool` | |
| `remove(path) -> Result<bool, string>` | a file or an empty directory |
| `list(path) -> Result<array<string>, string>` | the names in a directory, sorted, without `.` and `..` |

**`std.env`**: `args() -> array<string>` (the program's arguments, without the program),
`get(name) -> string?` (an environment variable).

**`std.time`**: `now()` (seconds since 1970 UTC), `monotonic()` (seconds since a fixed moment, for
measuring), `sleep(seconds)`, `format(seconds) -> string` (`1970-01-02T00:00:00Z`, UTC). The
builtin `clock()` is processor time.

**`std.math`**: the C library's `sqrt`, `cbrt`, `exp`, `log`, `log2`, `log10`, `pow`, `sin`,
`cos`, `tan`, `asin`, `acos`, `atan`, `atan2`, `sinh`, `cosh`, `tanh`, `hypot`, `floor`, `ceil`,
`round`, `trunc`, `fmod`, `fabs`; `pi()`, `e()`, `inf()`, `nan()`, `is_nan`, `is_finite`, `abs`,
`min`, `max`, `clamp` on floats; `abs_int`, `min_int`, `max_int`, `clamp_int`; random numbers:
`random()` in [0, 1), `random_int(low, high)` in [low, high], `seed(n)` (xoshiro256\*\*; unseeded
programs get the same numbers every run).

**`std.str`**: strings are byte strings, usually UTF-8; positions and lengths are in bytes.

| Function | |
|---|---|
| `find(s, part)`, `find_from(s, part, start)`, `find_last(s, part)` | a position, or -1 |
| `contains`, `starts_with`, `ends_with` | |
| `slice(s, start, end)` | the bytes from `start` up to `end` |
| `replace(s, from, to)`, `repeat(s, n)` | |
| `split(s, separator)`, `words(s)`, `lines(s)` | pieces between separators, between runs of whitespace, lines without endings |
| `join(parts, separator)` | |
| `trim`, `trim_start`, `trim_end`, `upper`, `lower` | whitespace and case are ASCII's |
| `parse_int(s) -> Result<int, string>`, `parse_float(s) -> Result<float, string>` | decimal, surrounding whitespace allowed |
| `fixed(x, decimals)` | `fixed(3.14159, 2)` is `3.14` |
| `from_byte(b)`, `from_code_point(c)`, `bytes(s)`, `from_bytes(bs)` | bytes and UTF-8 |
| `is_digit`, `is_alpha`, `is_alnum`, `is_space` | classes of ASCII bytes, for scanners |

The HIR builtins stay what every language has without importing anything: `len`, `substr`,
`byte_at`, `str`, `print`, `eprint`, `exit`, `panic`, `clock` and the collection operations.

## How it is made

Each module is HIR text, parsed into the program's builder by `HirBuilder::load_std` the first
time it is imported, after the modules it uses. Its C functions are declared `extern "a0"`: they
take and return A0's own values (an `A0String *`, an `A0Array *`), may allocate, and report
failures by returning NULL or 0 and leaving a message that `a0_std_error()` returns (the runtime
runs one thread, and C has no other way to return a value and a reason). Functions the C library
already has (`getenv`, `sqrt`) are declared as plain C functions, which take strings and arrays
directly (`docs/hir.md`, "Module").

In a module's text, a C function whose name starts with `a0_` is private to the module and
everything else is public; other modules' items are written by their full names
(`` `std.Result` ``). The library's modules are built-in sources (`a0_diag::register_builtin_source`)
named `<std>/str.hir` and so on, which every `SourceMap` knows, so the library's code has spans like
a program's: a diagnostic points at the parameter a call does not fit, a stack trace shows
`at std.str.split (<std>/str.hir:65:5)`, the language server's go-to-definition and the debugger
open a copy of the module written to the system's temporary directory, and warnings are left out
of it.

To add a function: write it in the module's `.hir` file, in HIR when it can be (`is_digit`), or
over a new C function in `std.c` (declared in `a0rt.h`, added to `symbols.c`), then use it from a
test program (`examples/minnow/tests/programs/std.mn`) and list it here.
