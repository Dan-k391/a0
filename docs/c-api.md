# The C API

`crates/a0-capi` makes A0's library crates usable from C (and anything that calls C): build core IR,
print and parse it, run it in the interpreter, compile it to object files for any supported target
or to an executable for the host, parse text with a grammar and walk the tree, and turn HIR text
into runnable core IR. The header is `crates/a0-capi/include/a0.h`; it is the reference.

`crates/a0-capi/tests/host.c` is a complete C program that does all of that, including a small
calculator language whose grammar is an A0 grammar and whose compiler is 40 lines of C. CI compiles
it with the system C compiler and runs it on every platform.

## Building and linking

```
cargo build --release -p a0-capi
```

produces the static library `target/release/a0_capi.lib` (Windows) or `liba0_capi.a` (Linux,
macOS). Compile with `-I crates/a0-capi/include` and link the library together with the system
libraries Rust's standard library needs, which `rustc --print native-static-libs` lists for a
target, and on Windows also `ole32.lib oleaut32.lib advapi32.lib`. For example:

```
# Linux
cc main.c -I crates/a0-capi/include target/release/liba0_capi.a \
   -lgcc_s -lutil -lrt -lpthread -lm -ldl -lc -o main
# macOS
cc main.c -I crates/a0-capi/include target/release/liba0_capi.a -lSystem -lc -lm -o main
# Windows (MSVC)
cl /MD main.c /I crates\a0-capi\include /link target\release\a0_capi.lib kernel32.lib ntdll.lib
   userenv.lib ws2_32.lib dbghelp.lib ole32.lib oleaut32.lib advapi32.lib
```

On Windows, compile with `/MD`: the library uses the dynamic C runtime. Building executables
(`a0_build_executable`) needs the same system linker the `a0` tool uses.

## Conventions

These follow decision D2 for every public API:

- **Objects** (`A0Builder`, `A0Module`, `A0Grammar`, `A0Tree`, `A0Hir`, `A0Diagnostics`) are opaque
  pointers created by A0 and released with their `a0_*_free`. `a0_builder_finish` consumes the
  builder.
- **Handles** (values, functions, variables, globals, tree nodes) are `uint32_t`; `A0_NONE` means
  none (the result of a void call, a missing child).
- **Strings** go in as pointer and length (`A0_STR("iadd")` expands to both) and come out as a
  pointer and a length borrowed from the object they belong to, valid until it changes or is freed.
  Neither is NUL-terminated.
- **Instructions and conditions** are named as in the text format (`"iadd"`, `"zext"`, `"slt"`),
  types with the `A0Type` enum.
- **Errors** are diagnostics: functions that can fail take `A0Diagnostics **diags` and set it to the
  errors, or to NULL on success. Misusing a builder (an unknown instruction, a value from another
  function) is recorded and reported by `a0_builder_finish`, never a crash.

```c
A0Builder *b = a0_builder_new(A0_STR("demo"));
A0Type i64[1] = {A0_I64};
A0Func print = a0_builder_declare_extern(b, A0_STR("a0_print_i64"), i64, 1, A0_VOID);
A0Func main_fn = a0_builder_declare_function(b, A0_STR("main"), NULL, 0, A0_I32);
a0_builder_begin_function(b, main_fn);
A0Value v = a0_builder_binary(b, A0_STR("imul"), a0_builder_iconst(b, A0_I64, 6),
                              a0_builder_iconst(b, A0_I64, 7));
a0_builder_call(b, print, &v, 1);
a0_builder_return(b, a0_builder_iconst(b, A0_I32, 0));
a0_builder_end_function(b);

A0Diagnostics *d = NULL;
A0Module *m = a0_builder_finish(b, &d);      /* or NULL, with d */
int32_t status;
a0_run(m, NULL, 0, &status, &d);             /* prints 42 */
a0_build_executable(m, A0_STR("demo"), &d);  /* ./demo prints 42 */
a0_module_free(m);
```

## What is bound

The core IR builder, modules and their text format, the interpreter (`a0_run`), the native backend
(`a0_write_object`, `a0_build_executable`), grammars and syntax trees, and HIR from its text format
(`a0_hir_parse`, `a0_hir_lower`). Not bound yet: the HIR builder, `a0-sema`, `SyntaxIndex` and the
driver (D20). The runtime and the collector keep global state, so use A0 from one thread at a time.
