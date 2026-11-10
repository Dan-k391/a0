# A0 from C

A0's C API lets a program written in C, or in any language that can call C, use A0. With it you
can:

- parse text with a [grammar](grammar.md) and walk the syntax tree;
- build a program in A0's [high-level IR](hir.md) (HIR, a typed form of a program that A0 checks,
  runs and compiles), or in its [core IR](ir.md), the low-level form under it;
- print both forms as text, and parse them back;
- run a module in the interpreter, or at native speed from memory with the JIT (code compiled to
  machine code in memory and run at once);
- compile a module to an object file for any supported platform, or to an executable for the
  machine you are on;
- give a language written in C the same command line a language written in Rust has, with the
  language server and the debugger.

The header is `crates/a0-capi/include/a0.h`. It is the reference for every function; this page
shows how to use them.

## A first program

This program builds `fn main() { let x = 6 * 7; print(str(x) + "\n"); }` in the HIR, runs it, and
builds it into an executable `demo`:

```c
#include <stdio.h>

#include "a0.h"

/* Prints the errors and frees them. */
static int fail(A0Diagnostics *d) {
    size_t len;
    const char *text = a0_diagnostics_text(d, &len);
    fprintf(stderr, "%.*s\n", (int)len, text);
    a0_diagnostics_free(d);
    return 1;
}

int main(void) {
    /* fn main() { let x = 6 * 7; print(str(x) + "\n"); } */
    A0HirBuilder *b = a0_hir_builder_new(A0_STR("demo"));
    A0HirFunc main_fn = a0_hir_declare_function(b, A0_STR("main"), NULL, 0);
    A0HirType unit = a0_hir_named_type(b, A0_STR("unit"), NULL, 0);
    a0_hir_set_signature(b, main_fn, NULL, NULL, 0, unit);
    A0HirBody body = a0_hir_function_body(b, main_fn);
    a0_hir_begin_body(b, body);
    A0HirExpr product = a0_hir_binary(b, A0_STR("*"), a0_hir_int(b, 6), a0_hir_int(b, 7));
    A0HirExpr items[2];
    items[0] = a0_hir_let_named(b, A0_STR("x"), a0_hir_fresh_type(b), product);
    A0HirExpr x = a0_hir_name(b, A0_STR("x"));
    A0HirExpr text = a0_hir_binary(b, A0_STR("+"), a0_hir_builtin(b, A0_STR("str"), &x, 1),
                                   a0_hir_string(b, A0_STR("\n")));
    items[1] = a0_hir_builtin(b, A0_STR("print"), &text, 1);
    a0_hir_set_root(b, body, a0_hir_block(b, items, 2, A0_NONE));
    a0_hir_end_body(b);

    A0Diagnostics *d = NULL;
    A0Hir *hir = a0_hir_finish(b, &d); /* frees the builder */
    if (hir == NULL) return fail(d);
    A0Module *m = a0_hir_lower(hir, &d);
    a0_hir_free(hir);
    if (m == NULL) return fail(d);

    int32_t status;
    if (a0_jit_run(m, NULL, 0, &status, &d) != 0) return fail(d);
    if (a0_build_executable(m, A0_STR("demo"), &d) != 0) return fail(d);
    a0_module_free(m);
    return status;
}
```

Save it as `main.c`. In a clone of A0, build the library, then compile and link the program
against it. On Linux:

```
cargo build --release -p a0-capi
cc main.c -I crates/a0-capi/include target/release/liba0_capi.a \
   -lgcc_s -lutil -lrt -lpthread -lm -ldl -lc -o main
./main
```

It prints:

```
42
```

and leaves the executable `demo`, which prints the same:

```
$ ./demo
42
```

What the program does:

1. `a0_hir_builder_new` starts a module. `a0_hir_declare_function`, `a0_hir_set_signature` and
   `a0_hir_function_body` make `main`.
2. Between `a0_hir_begin_body` and `a0_hir_end_body`, each call builds one expression and returns
   a handle to it, a number that later calls take as an argument. `a0_hir_name` finds `x` by its
   name, and the builder infers its type.
3. `a0_hir_finish` checks the module. `a0_hir_lower` turns it into a core IR module, which
   `a0_jit_run` runs and `a0_build_executable` compiles.

`crates/a0-capi/tests/host.c` is a longer program that uses most of the API. It has two small
languages: a calculator compiled to core IR, and a language with variables compiled to HIR, whose
unknown names the builder reports. CI compiles and runs it on every platform.

## Building and linking

```
cargo build --release -p a0-capi
```

makes two libraries in `target/release`:

| | Linux | macOS | Windows |
|---|---|---|---|
| static | `liba0_capi.a` | `liba0_capi.a` | `a0_capi.lib` |
| shared | `liba0_capi.so` | `liba0_capi.dylib` | `a0_capi.dll` |

The shared library needs nothing else at run time: the runtime that native executables link is
inside it. The [Python package](python.md) uses it.

To use the static library, compile with `-I crates/a0-capi/include`, and link the library together
with the system libraries that Rust's standard library needs. `rustc --print native-static-libs`
lists them for a platform. On Windows, also link `ole32.lib oleaut32.lib advapi32.lib`, which A0
uses to find the MSVC linker. For example:

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

On Windows, compile with `/MD`, because the library uses the dynamic C runtime.

To use the shared library instead, link it by name. The program then finds the library when it
starts, for example through `LD_LIBRARY_PATH` on Linux:

```
cc main.c -I crates/a0-capi/include -L target/release -la0_capi -o main
LD_LIBRARY_PATH=target/release ./main
```

`a0_build_executable` links with the system linker, the same one the `a0` tool uses: `cc` on Linux
and macOS, MSVC's linker on Windows.

## Concepts

### Objects

Objects are opaque pointers that A0 creates: `A0HirBuilder`, `A0Hir`, `A0Builder`, `A0Module`,
`A0Grammar`, `A0Tree`, `A0Language` and `A0Diagnostics`. Each has a function that frees it:
`a0_module_free`, `a0_tree_free` and so on.

Two functions consume their object, so you do not free it afterwards:

- `a0_hir_finish` and `a0_builder_finish` free the builder, whether they succeed or not;
- `a0_hir_builder_from` takes over the `A0Hir` it continues.

`a0_hir_builder_free` frees a HIR builder you will not finish.

### Handles

Things inside an object are `uint32_t` handles: values, functions, variables and globals in a
builder, HIR types and expressions, tree nodes. The header gives them names (`A0Value`,
`A0HirExpr`, `A0Node`, ...), but they are all `uint32_t`. `A0_NONE` stands for none: the result
of a call that returns nothing, a missing child, an `if` without `else`.

A HIR item (struct, enum, function, global, extern, interface) is a kind and a handle. The kinds
are `A0_HIR_STRUCT`, `A0_HIR_ENUM`, `A0_HIR_FUNC`, `A0_HIR_GLOBAL`, `A0_HIR_EXTERN` and
`A0_HIR_INTERFACE`.

### Strings

A string goes in as a pointer and a length. `A0_STR("iadd")` expands to both, for a literal. Lists
of names are arrays of `A0Str`: `A0Str params[] = {A0_STR("x"), A0_STR("y")};`.

A string A0 returns comes out as a pointer, with its length in a `size_t *`. It belongs to the
object it came from, and stays valid until that object changes or is freed.

Neither is NUL-terminated. A0 reads exactly the length you pass, and a string it returns may have
no NUL after it. The one exception is `a0_version()`, which returns a NUL-terminated string.

### Names

Instructions, conditions, operators and builtins are named as in the text formats: `"iadd"`,
`"slt"`, `"+"`, `"print"`. Core IR types are the `A0Type` enum (`A0_I64`, `A0_F64`, `A0_PTR`, ...).
HIR types are handles: `a0_hir_named_type(b, A0_STR("int"), NULL, 0)`.

### Errors

A function that can fail takes `A0Diagnostics **diags` as its last argument. On failure it sets
`*diags` to the errors; on success, to `NULL`. You may pass `NULL` if you do not want them. Free
the errors with `a0_diagnostics_free`.

This program reports an unknown name, `y`, at bytes 6 to 7 of the source `print(y)`:

```c
#include <stdio.h>

#include "a0.h"

int main(void) {
    const char source[] = "print(y)";
    A0HirBuilder *b = a0_hir_builder_new(A0_STR("broken"));
    A0HirFunc main_fn = a0_hir_declare_function(b, A0_STR("main"), NULL, 0);
    a0_hir_set_signature(b, main_fn, NULL, NULL, 0, a0_hir_named_type(b, A0_STR("unit"), NULL, 0));
    A0HirBody body = a0_hir_function_body(b, main_fn);
    a0_hir_begin_body(b, body);
    a0_hir_set_span(b, 6, 7); /* `y` in the source */
    a0_hir_set_root(b, body, a0_hir_name(b, A0_STR("y")));
    a0_hir_end_body(b);

    A0Diagnostics *d = NULL;
    A0Hir *hir = a0_hir_finish(b, &d);
    if (hir != NULL) return 0;
    size_t len;
    for (size_t i = 0; i < a0_diagnostics_count(d); i++) {
        const char *code = a0_diagnostics_code(d, i, &len);
        printf("code: %.*s\n", (int)len, code);
        uint32_t start, end;
        if (a0_diagnostics_span(d, i, &start, &end)) printf("span: %u..%u\n", start, end);
    }
    const char *text = a0_diagnostics_render(d, A0_STR("broken.x"), A0_STR(source), &len);
    printf("%.*s", (int)len, text);
    a0_diagnostics_free(d);
    return 1;
}
```

It prints:

```
code: E1001
span: 6..7
error[E1001]: unknown name `y`
 --> broken.x:1:7
  |
1 | print(y)
  |       ^ not found in this scope
```

`a0_hir_set_span` says where in the source the next nodes come from, so that errors point there.
`a0_diagnostics_render` shows the errors against the source, the way A0's tools do.

**Misusing a builder does not crash.** An unknown instruction, a value from another function, a
handle that does not exist, or a local declared outside any body is recorded, and
`a0_builder_finish` or `a0_hir_finish` reports it. Passing an invalid pointer is still undefined
behavior.

### Versions

`A0_VERSION_MAJOR`, `A0_VERSION_MINOR` and `A0_VERSION_PATCH` are the header's version, and
`a0_version()` returns the library's, such as `"1.0.0"`. Within a major version, nothing in the
header changes or goes away ([compatibility](compatibility.md)). So a program built with one
header works with a library of the same major version that is at least as new. Compare the two at
run time to check.

A declaration that is to be retired is marked `A0_DEPRECATED("use ...")`. GCC, Clang and MSVC warn
where it is used, and it keeps working until the next major version.

### Threads

The runtime and the collector keep global state, so use A0 from one thread at a time.

## Reference

The header declares each function with a comment. This section says what each group is for.

### Diagnostics

| Function | What it does |
|---|---|
| `a0_diagnostics_count` | how many diagnostics there are |
| `a0_diagnostics_message(d, i, &len)` | diagnostic `i`'s message |
| `a0_diagnostics_code(d, i, &len)` | its code, such as `E2001` (`a0 explain` explains it), or empty |
| `a0_diagnostics_span(d, i, &start, &end)` | where it is, as byte offsets into its source; returns 0 if it has no place |
| `a0_diagnostics_text(d, &len)` | all of them, one per line |
| `a0_diagnostics_render(d, name, len, source, len, &len)` | all of them shown against `source`, as the file `name` |
| `a0_diagnostics_free` | frees them |

### Building core IR

The `a0_builder_*` functions build a [core IR](ir.md) module one instruction at a time. This one
calls the runtime's `a0_print_i64` with 6 × 7:

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

Functions and externs can be declared in any order. Control flow is structured: `a0_builder_if`,
`a0_builder_else` and `a0_builder_end_if`, and `a0_builder_loop` and `a0_builder_end_loop`, with
`break` and `continue`. Variables (`a0_builder_declare_var`, `a0_builder_get`, `a0_builder_set`)
are mutable, so a frontend needs no phi nodes. `a0_builder_set_struct_param` passes C structs by
value to externs ([C structs](ir.md#c-structs)).

### Modules

| Function | What it does |
|---|---|
| `a0_module_parse` | parses and verifies core IR text |
| `a0_module_text` | the module as core IR text |
| `a0_run` | interprets `main` |
| `a0_jit_run` | compiles `main` into memory and runs it at native speed |
| `a0_write_object` | writes an object file for a platform: `"x86_64-linux"`, `"aarch64-macos"`, ...; an empty name means this machine |
| `a0_build_executable` | compiles and links an executable for this machine |
| `a0_module_free` | frees the module |

`a0_run` and `a0_jit_run` take the program's arguments, its name first, and flush the program's
output before they return. They differ when the program fails:

| Result | `a0_run` | `a0_jit_run` |
|---|---|---|
| the program ended | returns 0, exit status in `*status` | returns 0, exit status in `*status` |
| it trapped or panicked | returns 1, `*status` is 101 and `*diags` says why | prints why and ends the whole process with status 101, as an executable would |
| it could not run | returns -1, with `*diags` | returns -1, with `*diags` |

### Grammars and syntax trees

`a0_grammar_new` reads a [grammar](grammar.md), and `a0_grammar_parse` parses text with it. Parsing
always returns a tree: syntax errors appear in it as `error` nodes, and in `*diags`.

A tree's nodes are handles: `a0_tree_root` is the root, `a0_tree_child_count` and `a0_tree_child`
walk the children. For each node, `a0_tree_kind` gives its rule or token kind (`"binary"`,
`"NUMBER"`, `"\"+\""`), `a0_tree_text` its source text, `a0_tree_span` its byte range, and
`a0_tree_is_token` whether it is a token. `a0_tree_field` finds the first child with a label from
the grammar, and `a0_tree_child_field` gives a child's label.

### The HIR

| Function | What it does |
|---|---|
| `a0_hir_parse` | parses and checks [HIR text](hir.md#text-format) |
| `a0_hir_text` | the module as HIR text |
| `a0_hir_lower` | lowers a module with a `main` to core IR, optimized unless the environment variable `A0_OPT` is `0` |
| `a0_hir_warnings` | the warnings found when the module was built (turned on with `a0_hir_enable_warning`) |
| `a0_hir_free` | frees the module |

### Building HIR

The `a0_hir_*` builder functions build a HIR module the way a frontend in Rust does
([building HIR from a frontend](hir.md#building-hir-from-a-frontend)). The builder infers and
checks types. Built by name, it also resolves names, so a frontend needs no checker of its own.
The first program on this page builds HIR this way: open a function's body, then build its code
by name, with spans so that errors point at the source.

### What is bound

The C API covers:

- **Grammars and syntax trees.**
- **The HIR builder**, every function of the Rust `HirBuilder`, by handle and by name. That
  includes:
  - what makes good diagnostics: `a0_hir_set_name_span`, `a0_hir_set_param_span`,
    `a0_hir_set_term`, `a0_hir_report_code`, and the functions that add labels, notes and fixes
    ([diagnostics](diagnostics.md));
  - modules: `a0_hir_begin_module`, `a0_hir_import_module`, `a0_hir_call_member`;
  - methods and interfaces: `a0_hir_declare_method`, `a0_hir_implement`, `a0_hir_method_call`;
  - exceptions: `a0_hir_throw`, `a0_hir_try_catch`.
- **Diagnostics**, with their codes (`a0_diagnostics_code`), rendered against their source the way
  the `a0` tool shows them (`a0_diagnostics_render`).
- **HIR text**, the **core IR builder** and **core IR text**.
- **The interpreter** (`a0_run`), **the JIT** (`a0_jit_run`), and **the native backend**
  (`a0_write_object`, `a0_build_executable`).
- **The driver** (`a0_language_*`), below.

Not bound: `a0-sema` and `SyntaxIndex`.

A test checks that the header declares exactly the functions the library exports.

### The driver: a language's command line

A language whose compiler is a C function gets every command of a language written in Rust:
`run`, `build`, `check`, `hir`, `ir`, `lsp` (the [language server](editors.md)), `dap` (the
[debugger](debugging.md)), `repl` and `explain`. This program is a language whose source is HIR
text, so its compiler only parses:

```c
#include "a0.h"

/* The language's compiler. This one takes HIR text, so it only parses it. */
static A0Hir *compile(void *context, const char *text, size_t len, A0Diagnostics **diags) {
    (void)context;
    return a0_hir_parse(text, len, diags);
}

int main(int argc, char **argv) {
    A0Language *lang = a0_language_new(A0_STR("lang"));
    a0_language_set_compiler(lang, compile, NULL);
    int status = a0_language_main(lang, (const char *const *)argv + 1, (size_t)argc - 1);
    a0_language_free(lang);
    return status;
}
```

With `hello.hir`:

```
module "hello" version 1;

fn main() {
  print("hello from C\n");
}
```

```
$ ./lang run hello.hir
hello from C
```

The compiler turns the text of one source file into HIR whose spans are in that text, or returns
`NULL` and sets `*diags`. The driver calls it whenever it needs the program.

| Function | What it does |
|---|---|
| `a0_language_new(name, len)` | a language; `name` is its command, used in messages and the usage text |
| `a0_language_set_compiler(l, compile, context)` | the compiler, called with `context` |
| `a0_language_set_grammar(l, text, len)` | the text of the language's `.a0g` grammar, for the language server and the REPL |
| `a0_language_set_repl_main(l, program, len)` | for a language whose statements must be inside a function: the program the REPL runs, with `{}` where the statements typed so far go |
| `a0_language_main(l, args, count)` | runs the command line `args`, without the program's name, and returns the exit status; a program that runs ends the process with its own status |
| `a0_language_free` | frees the language |

There can be one language per process.
