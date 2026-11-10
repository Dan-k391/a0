# A0 from Python

The Python package `a0` lets a Python program use A0. With it, Python code can parse text with a
[grammar](grammar.md), build a program in A0's [high-level IR](hir.md) (HIR, a typed form of a
program that A0 can check, run and compile), and run that program or compile it to an executable.

So you can write a whole language in Python. It gets what a language written in Rust gets:

- error messages with codes, labels and fixes;
- a JIT (code compiled to machine code in memory and run at once), an interpreter, and native
  executables;
- the garbage collector;
- the language server, the debugger and the REPL.

The package calls [A0's C API](c-api.md) through ctypes, Python's built-in way to call C. It needs
no C compiler and no other Python package, only A0's shared library.

## A first script

This script parses `1 + 2 + 39` with a three-line grammar, builds a program that adds the numbers
and prints the sum, runs it, and builds it into an executable:

```python
import a0

# A grammar: numbers separated by "+".
grammar = a0.Grammar('skip / +/; token NUM /[0-9]+/; program = term ("+" term)*; term = NUM;')
tree = grammar.parse("1 + 2 + 39")

# A HIR function `main` that adds the numbers and prints the sum.
b = a0.HirBuilder("sum")
main = b.declare_function("main", [])
b.set_signature(main, ([], []), b.named_type("unit", []))
body = b.function_body(main)
b.begin_body(body)
total = b.int(0)
for term in tree.root.nodes:
    b.at(term)
    total = b.binary("+", total, b.int(int(term.text)))
b.set_root(body, b.block([b.builtin("print", [b.builtin("str", [total])])],
                         b.builtin("print", [b.string("\n")])))
b.end_body()

hir = b.finish()
module = hir.lower()
module.run_jit(["sum"])
module.build_executable("sum")
```

Save it as `sum.py`. In a clone of A0, build the shared library, then run the script with the
package on Python's path (on Windows, `set PYTHONPATH=bindings\python`, then `python sum.py`):

```
cargo build -p a0-capi
PYTHONPATH=bindings/python python3 sum.py
```

It prints:

```
42
```

and leaves an executable `sum` (`sum.exe` on Windows), which prints the same:

```
$ ./sum
42
```

What the script does:

1. `a0.Grammar` reads the grammar, and `parse` turns the text into a syntax tree.
2. `a0.HirBuilder` builds the program. `declare_function`, `set_signature` and `function_body`
   make `main`. Between `begin_body` and `end_body`, each call builds one expression and returns a
   handle to it, which later calls take as arguments.
3. `b.at(term)` says which part of the source the next expressions come from, so that an error
   points there.
4. `finish` checks the program and returns it as an `a0.Hir`. `lower` turns it into a
   [core IR](ir.md) module, the low-level form that A0 runs and compiles.
5. `run_jit` runs the module, and `build_executable` writes an executable with the system linker.

## Installing

The package needs Python 3.9 or later and A0's shared library: `a0_capi.dll` on Windows,
`liba0_capi.so` on Linux and `liba0_capi.dylib` on macOS.

**From a clone of A0**, build the library with `cargo build -p a0-capi` (or with `--release`), and
put `bindings/python` on `PYTHONPATH`, or copy its `a0` directory next to your code.

**From a release**, the archive has the package in `python/`, with the library inside it:

```
pip install ./python
```

Each release also has a wheel for each platform, `a0_lang-<version>-py3-none-<platform>.whl`,
with the library inside.

When it is imported, the package looks for the library in these places, in order:

1. the file named by the environment variable `A0_LIBRARY`;
2. `a0/lib/` inside the package, where a wheel or a release archive puts it;
3. `target/release`, then `target/debug`, of the A0 clone the package is in.

If it finds none, `import a0` fails:

```
ImportError: cannot find liba0_capi.so: build it with `cargo build -p a0-capi` or set A0_LIBRARY
```

`a0.__version__` is the package's version, and `a0.version()` the library's. The package refuses,
with an `ImportError`, a library of another major version or of an older minor version.

## Pebble, a language written in Python

[Pebble](../bindings/python/examples/pebble) is a small typed language with functions, arrays and
`for` loops. Its compiler, `pebble.py`, is about 230 lines of Python. From a clone of A0, after
`cargo build -p a0-capi`:

```
python3 bindings/python/examples/pebble/pebble.py run bindings/python/examples/pebble/programs/arrays.pb
```

```
14
15
5
square 1
square 4
square 9
square 16
square 25
```

`build` makes an executable that prints the same:

```
python3 bindings/python/examples/pebble/pebble.py build bindings/python/examples/pebble/programs/arrays.pb -o arrays
```

Errors are reported the way A0's own tools report them. For a file `broken.pb` with these lines:

```
fn main() {
  let s = 1 + "no";
}
```

`check` prints:

```
$ python3 bindings/python/examples/pebble/pebble.py check broken.pb
error[E2001]: mismatched types
 --> broken.pb:2:15
  |
2 |   let s = 1 + "no";
  |           -   ^^^^ expected `int`, found `string`
  |           |
  |           expected because this is `int`
```

Pebble's compiler is one function, `compile(text)`, that parses the text and walks the tree with a
`HirBuilder`. Its last line hands that function to A0's driver, which provides every command.

## A language's command line

`a0.driver.main` is a language's whole command line. Pebble ends with:

```python
from a0.driver import main

if __name__ == "__main__":
    sys.exit(main("pebble", compile, grammar=GRAMMAR, repl_main="fn main() {\n{}}\n"))
```

`main(name, compile, argv=None, grammar=None, repl_main=None)` runs the command line `argv` (by
default the process's, without the program's name) and returns the exit status. It is A0's driver
itself, the same one a language written in Rust uses, reached through the C API. So a Python
language has the same commands:

| Command | What it does |
|---|---|
| `run <file> [args...]` | runs the program in the JIT; `A0_INTERP=1` interprets it instead; `-` as the file reads the program from standard input |
| `build <file> [-o out] [-g] [--target <target>]` | compiles it to an executable; `-g` adds debug information; another platform needs its [target pack](cross.md) |
| `check <file>` | reports errors without running anything |
| `hir <file>`, `ir <file>` | prints the HIR or the core IR the compiler made |
| `lsp` | the language server ([editors](editors.md)) |
| `dap` | the debugger ([debugging](debugging.md)) |
| `repl` | an interactive prompt ([the REPL](repl.md)) |
| `explain <code>` | explains an error code |

The arguments:

- **`name`** is the command's name, used in messages and the usage text.
- **`compile(text)`** returns the program as an `a0.Hir`, or raises `a0.Error` with spans into
  `text`. The driver calls it whenever it needs the program; the language server calls it on
  every pause in editing. If it raises any other exception, the driver prints the traceback and
  treats the program as failed.
- **`grammar`**, an `a0.Grammar`, gives the language server its outline, go to definition and
  completion. It also lets the REPL tell declarations from statements, and see when a line is
  unfinished.
- **`repl_main`** is for a language whose statements must be inside a function. It is the program
  the REPL runs, with `{}` where the statements typed so far go.

The program's own exit status is passed through. A program that panics prints the message and
its stack trace on standard error and exits with 101, as a native executable does. So a Python
language behaves the same in every mode.

To use the language server in VS Code, give A0's extension a command that runs Python, such as
`python pebble.py lsp`. [Editor support](editors.md) shows the entry.

## Reference

### Grammars and syntax trees

```python
grammar = a0.Grammar('skip / +/; token NUM /[0-9]+/; token NAME /[a-z]+/; '
                     'program = stmt*; stmt = "let" name:NAME "=" value:NUM ";";')
tree = grammar.parse("let x = 1;")
node = tree.root.nodes[0]
node.kind, node.text, node.span    # 'stmt', 'let x = 1;', (0, 10)
node.field("value")                # <NUM '1'>
node.fields("name")                # [<NAME 'x'>]
```

| Name | What it is |
|---|---|
| `Grammar(text)` | a grammar in A0's notation ([grammars](grammar.md)); raises `a0.Error` if it is invalid |
| `grammar.text` | the grammar's text |
| `grammar.parse(text)` | the syntax tree of `text`, a `Tree`; raises `a0.Error` with the syntax errors |
| `tree.root` | the root node; `tree.text` is the text that was parsed |
| `node.kind` | the rule or token kind: `stmt`, `NAME`, `"+"` |
| `node.text` | the source text the node covers |
| `node.span` | where that text is, as byte offsets `(start, end)` |
| `node.is_token` | whether it is a token rather than a node |
| `node.children` | every child, tokens included |
| `node.nodes` | the children that are nodes |
| `node.field(name)` | the first child labeled `name` in the grammar (`value:NUM`), or `None` |
| `node.fields(name)` | every child labeled `name`, in order |

Two `Node`s are equal when they are the same node of the same tree.

### Building HIR: `HirBuilder`

```python
b = a0.HirBuilder("calc")
f = b.declare_function("main", [])               # no type parameters
b.set_signature(f, ([], []), b.named_type("int", []))
body = b.function_body(f)
b.begin_body(body)
x = b.let_named("x", b.fresh_type(), b.int(41))
b.set_root(body, b.block([x], b.binary("+", b.name("x"), b.int(1))))
b.end_body()
hir = b.finish()
print(hir)
```

prints the module in the [HIR text format](hir.md#text-format), with the type of `x` inferred:

```
module "calc" version 1;

fn main() -> int {
  let x: int = 41;
  x + 1
}
```

The builder's methods are the C API's `a0_hir_*` functions without their prefix:
`a0_hir_let_named` is `HirBuilder.let_named`. The ones whose names are Python keywords take a
trailing underscore: `if_`, `while_`, `return_`, `break_`, `continue_`, `is_`, `lambda_` and
`global_`. The C API's
[list of what is bound](c-api.md#what-is-bound) and `a0.h` describe each function.

Arguments follow the C signature, converted like this:

| In C | In Python |
|---|---|
| a string: a pointer and a length | a `str` |
| a handle | an `int`; `None` for `A0_NONE` |
| an array and its count | a list |
| names with their types or values (two arrays and a count) | a pair of lists: `set_signature(f, (names, types), ret)` |
| bytes and their count | a list of ints |
| `A0Diagnostics **` | nothing: an error is raised as `a0.Error` |

`lookup_type` and `lookup_value` return a pair: the handle and its kind. `add_text` raises
`a0.Error` if the text has errors.

Methods of the builder itself:

| Name | What it does |
|---|---|
| `HirBuilder(module_name)` | a new, empty builder |
| `HirBuilder.from_hir(hir)` | a builder that continues `hir`, for example a prelude parsed from HIR text; it uses `hir` up |
| `b.at(node)` | says that what is built next, and its errors, come from `node`: a `Node`, or a span `(start, end)` |
| `b.finish()` | checks the module and returns it as an `a0.Hir`; raises `a0.Error` with every problem found |

After `finish`, the builder is used up: calling `finish` again raises `ValueError`.

`a0.HIR_FUNCTIONS` is the table of the methods and their argument types. The tests check it
against `a0.h`, so a function added to the C API gets a method too.

### Building core IR: `Builder`

```python
b = a0.Builder("ir")
main = b.declare_function("main", [], a0.I32)
b.begin_function(main)
b.return_(b.binary("iadd", b.iconst(a0.I32, 2), b.iconst(a0.I32, 3)))
b.end_function()
module = b.finish()
module.run([])    # 5
```

`Builder(module_name)` builds a [core IR](ir.md) module instruction by instruction. Its methods are
the C API's `a0_builder_*` functions without their prefix, converted as for `HirBuilder`; `if_`,
`else_`, `break_`, `continue_` and `return_` take a trailing underscore. Core IR types are the
constants `a0.VOID`, `a0.I1`, `a0.I8`, `a0.I16`, `a0.I32`, `a0.I64`, `a0.F32`, `a0.F64` and
`a0.PTR`. `finish()` verifies the module and returns it as an `a0.Module`, or raises `a0.Error`.
`a0.IR_FUNCTIONS` is its table of methods, checked against `a0.h` like `HIR_FUNCTIONS`.

### `Hir`: a HIR module

| Name | What it does |
|---|---|
| `Hir.parse(text)` | parses and checks HIR text; raises `a0.Error` |
| `str(hir)` | the module as HIR text |
| `hir.warnings` | the warnings found while building (turned on with `enable_warning`), as an `a0.Error`, or `None` |
| `hir.lower()` | the core IR module, an `a0.Module`, optimized unless the environment variable `A0_OPT` is `0` |

### `Module`: a core IR module

| Name | What it does |
|---|---|
| `Module.parse(text)` | parses and verifies core IR text; raises `a0.Error` |
| `str(module)` | the module as core IR text |
| `module.run(args)` | interprets `main` and returns its exit status; `args` is the program's arguments, its name first |
| `module.run_jit(args)` | compiles `main` into memory, runs it at native speed, and returns its exit status |
| `module.write_object(path, target="")` | writes an object file for `target`, a name like `x86_64-linux`; the host by default |
| `module.build_executable(path)` | compiles and links an executable for the host; needs the system linker |

When the program panics, both print the message and the stack trace on standard error. `run` then
returns `a0.PANIC_EXIT_CODE` (101). `run_jit` ends the whole Python process with that status, as a
native executable would end.

### Errors

A0's errors are raised as `a0.Error`:

```python
try:
    b.finish()
except a0.Error as error:
    print(error.diagnostics)
    print(error.render("broken.x", "print(y)"))
```

For a builder whose `main` uses an unknown name `y` at bytes 6 to 7 of `print(y)`, this prints:

```
[Diagnostic('E1001', 'unknown name `y`', (6, 7))]
error[E1001]: unknown name `y`
 --> broken.x:1:7
  |
1 | print(y)
  |       ^ not found in this scope
```

- `error.diagnostics` is a list of `a0.Diagnostic`, each with a `code` (`E1001`; `a0 explain`
  explains it), a `message` and a `span`: byte offsets `(start, end)`, or `None`.
- `str(error)` has one line per diagnostic: ``error: unknown name `y` ``.
- `error.render(name, source)` shows the diagnostics against `source`, as the file `name`, the way
  A0's tools do.

### Other names

| Name | What it is |
|---|---|
| `a0.NONE` | the handle that stands for none (`A0_NONE`, `0xFFFFFFFF`) |
| `a0.PANIC_EXIT_CODE` | 101, the exit status of a program that panics |
| `a0.version()` | the library's version, `"1.0.0"` |
| `a0.__version__` | the package's version |

## Testing

`bindings/python/tests/test_a0.py` tests the package, and runs Pebble's programs in every mode:
interpreted, in the JIT (also under GC stress and without the optimizer), and as executables.
Cargo runs it against the library it has just built, and also tests Pebble's language server and
debugger:

```
cargo test -p a0-capi --test python
```

To run the Python tests alone, point `A0_LIBRARY` at a library you built (on Windows,
`target/debug/a0_capi.dll`):

```
A0_LIBRARY=target/debug/liba0_capi.so python3 -m unittest discover -s bindings/python/tests
```

## Limits

- **One file per program.** The driver's Python binding compiles a single file. Programs of
  several files, with imports, need a language written in Rust.
- **One thread at a time.** As for the C API, the runtime and the collector keep global state, so
  call the package from one thread only.
