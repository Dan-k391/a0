# Languages in Python

`bindings/python` is a Python package, `a0`, over the C API (`docs/c-api.md`) with ctypes: no
compiler, no dependencies, only the shared library that `cargo build -p a0-capi` builds. A language
written in Python gets the same as one written in Rust: grammars and syntax trees, the HIR builder
that resolves names, infers and checks types and reports errors, the JIT, the interpreter, native
executables and the collector, and the driver's commands: the language server, the debugger and
the REPL. [Pebble](../bindings/python/examples/pebble) is an example: a small
typed language with functions, arrays and `for` loops, whose compiler is about 230 lines of Python.

```
cargo build --release -p a0-capi
python bindings/python/examples/pebble/pebble.py run bindings/python/examples/pebble/programs/arrays.pb
python bindings/python/examples/pebble/pebble.py build bindings/python/examples/pebble/programs/arrays.pb -o arrays
```

The package finds the library inside itself (`a0/lib/`, as a wheel or a release archive's
`python/` directory has it: `pip install ./python`), in `target/release` or `target/debug` of the
checkout it is in, or wherever `A0_LIBRARY` says. `a0.__version__` is the package's version; it
refuses a library of another major version or an older minor one. To use it from elsewhere, put `bindings/python` on `PYTHONPATH` (or copy
the `a0` directory) and set `A0_LIBRARY`.

## The API

```python
import a0

grammar = a0.Grammar(open("calc.a0g").read())   # a0.Error if the grammar is invalid
tree = grammar.parse(text)                      # a0.Error with the syntax errors
node = tree.root
node.kind, node.text, node.span                 # "let_stmt", "let x = 1;", (0, 10)
node.field("value"), node.fields("arg")         # labeled children: a Node (or None), a list
node.children, node.nodes, node.is_token        # all children, those that are nodes

b = a0.HirBuilder("calc")
f = b.declare_function("main", [])               # no type parameters
b.set_signature(f, ([], []), b.named_type("int", []))
body = b.function_body(f)
b.begin_body(body)
b.at(node)                                      # what follows, and its errors, is at `node`
x = b.let_named("x", b.fresh_type(), b.int(41))
b.set_root(body, b.block([x], b.binary("+", b.name("x"), b.int(1))))
b.end_body()

hir = b.finish()                                # a0.Error with every problem found
print(hir)                                      # the HIR text format
module = hir.lower()                            # core IR, optimized unless A0_OPT=0
status = module.run_jit(["calc"])               # at native speed, from memory: 42
status = module.run(["calc"])                   # interpreted: 42
module.build_executable("calc")                 # native, with the system linker
```

The builders' methods are the C functions without their prefix: `a0_hir_let_named` is
`HirBuilder.let_named`, `a0_builder_iconst` is `Builder.iconst`, and `if`, `while`, `return`,
`break`, `continue`, `is` and `else` take a trailing underscore. Arguments follow the C signature:
a Python string for a pointer and length, an int for a handle (`None` for `A0_NONE`), a list for
an array and its count, and a pair of lists for names with their types or values
(`set_signature(f, (names, types), ret)`). `lookup_type` and `lookup_value` return the handle and
the kind. The table of signatures (`a0.HIR_FUNCTIONS`, `a0.IR_FUNCTIONS`) is checked against
`a0.h` by the tests, so a function added to the C API is added there too.

Errors are `a0.Error`: `diagnostics` lists them (code, message, span), `str(error)` has one per
line, and `error.render(name, source)` shows them against the source with the lines they are
about, as the `a0` tool does.

`a0.driver.main(name, compile, grammar=..., repl_main=...)` is the command line of a language: the
Rust driver itself, reached through the C API (`a0_language_main`, D67), so a Python language has
every command a Rust one has: `run <file> [args...]` (at native speed; `A0_INTERP=1` interprets),
`build <file> [-o out]`, `hir`, `ir`, `lsp` (the language server, `docs/editors.md`), `dap` (the
debugger, `docs/debugging.md`), `repl` and `explain <code>`. `compile(text)` returns an `a0.Hir` or
raises `a0.Error`; the driver calls it whenever it needs the program, the language server on every
pause in editing. `grammar` (an `a0.Grammar`) gives the language server its outline, definitions
and completion. A panic prints its message and stack trace on standard error and gives 101, as a
native executable does, so a language behaves the same in every mode. In VS Code, the entry for a
Python language runs Python:

```json
{ "extensions": [".pb"], "command": "python", "args": ["${workspaceFolder}/pebble.py", "lsp"] }
```

`Module.run` and `Module.run_jit` run a module from Python directly, interpreted and at native
speed.

## Testing

`bindings/python/tests/test_a0.py` tests the bindings and runs Pebble's programs interpreted, in the JIT
(also under GC stress and without the optimizer) and native. `cargo test -p a0-capi --test python` runs
it against the library cargo just built, on every CI platform, and tests Pebble's language server
and debugger with the driver's own clients:

```
cargo test -p a0-capi --test python
A0_LIBRARY=target/debug/a0_capi.dll python -m unittest discover -s bindings/python/tests
```

## Not yet

- Programs of several files: the driver binding compiles one file (`modules` is Rust only).
- Packaging: the package is used from the checkout; nothing is published to PyPI.
- One thread at a time, as for the C API: the runtime and the collector keep global state.
