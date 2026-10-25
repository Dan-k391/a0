# The C API

`crates/a0-capi` makes A0's library crates usable from C (and anything that calls C): parse text with
a grammar and walk the tree, build HIR (by name, with the builder resolving names and checking
types, or by handle), build core IR, print and parse both, run modules in the interpreter, and
compile them to object files for any supported target or to an executable for the host. The header
is `crates/a0-capi/include/a0.h`; it is the reference.

`crates/a0-capi/tests/host.c` is a complete C program that does all of that, including two small
languages: a calculator compiled to core IR, and a language with variables compiled to HIR by name,
whose unknown names the builder reports. CI compiles it with the system C compiler and runs it on
every platform, and a test keeps the header and the library's exports identical.

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
- **Instructions, conditions, operators and builtins** are named as in the text formats
  (`"iadd"`, `"slt"`, `"+"`, `"print"`); core IR types use the `A0Type` enum, HIR types are handles
  (`a0_hir_named_type(b, A0_STR("int"), NULL, 0)`). Lists of names are arrays of `A0Str`
  (`{A0_STR("x")}`), and a HIR item (struct, enum, function, ...) is a kind (`A0_HIR_STRUCT`) and
  a handle.
- **Errors** are diagnostics: functions that can fail take `A0Diagnostics **diags` and set it to the
  errors, or to NULL on success. Misusing a builder (an unknown instruction, a value from another
  function, a handle that does not exist, a local declared outside any body) is recorded and
  reported by `a0_builder_finish` or `a0_hir_finish`, never a crash.

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

A frontend in C builds HIR the way Minnow does in Rust: open a function's body, then build its
code by name, with spans so that errors point at the source.

```c
A0HirBuilder *b = a0_hir_builder_new(A0_STR("demo"));
A0HirType int_t = a0_hir_named_type(b, A0_STR("int"), NULL, 0);
A0HirFunc main_fn = a0_hir_declare_function(b, A0_STR("main"), NULL, 0);
a0_hir_set_signature(b, main_fn, NULL, NULL, 0, int_t);
A0HirBody body = a0_hir_function_body(b, main_fn);
a0_hir_begin_body(b, body);
A0HirExpr items[2];
items[0] = a0_hir_let_named(b, A0_STR("x"), a0_hir_fresh_type(b), a0_hir_int(b, 42));
A0HirExpr x = a0_hir_name(b, A0_STR("x"));              /* resolved by the builder */
A0HirExpr text = a0_hir_builtin(b, A0_STR("str"), &x, 1);
items[1] = a0_hir_builtin(b, A0_STR("print"), &text, 1);
a0_hir_set_root(b, body, a0_hir_block(b, items, 2, a0_hir_int(b, 0)));
a0_hir_end_body(b);

A0Diagnostics *d = NULL;
A0Hir *h = a0_hir_finish(b, &d);                        /* or NULL, with d */
A0Module *m = a0_hir_lower(h, &d);                      /* then a0_run, a0_build_executable */
```

## What is bound

Grammars and syntax trees, the HIR builder (every function of `HirBuilder`, by handle and by name,
including what makes good diagnostics: `a0_hir_set_name_span`, `a0_hir_set_param_span`,
`a0_hir_set_term`, `a0_hir_report_code` and the functions that add labels, notes and fixes; see
`docs/diagnostics.md`), the codes of diagnostics (`a0_diagnostics_code`),
HIR text, the core IR builder and text format, the interpreter (`a0_run`) and the native backend
(`a0_write_object`, `a0_build_executable`). Not bound: `a0-sema`, `SyntaxIndex` and the driver
(D20). The runtime and the collector keep global state, so use A0 from one thread at a time.
