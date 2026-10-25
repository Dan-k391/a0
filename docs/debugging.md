# Debugging

Every language built on A0 gets a debugger with no code of its own: `lang dap` serves the Debug
Adapter Protocol, so VS Code (through `editors/vscode`), Neovim (nvim-dap) and any other DAP client
can set breakpoints, step, and show the stack and its locals. Native executables can also carry
debug information for gdb and lldb (`lang build -g`).

## The debugger: `lang dap`

```
minnow dap                       # what a client starts
a0 lang calc.a0g dap             # a language defined by its grammar alone
```

The program runs in the interpreter, in the debugger's own process, compiled without the HIR
optimizer so that every function keeps its frame. It supports:

- **Breakpoints** by line, set before or while the program runs, and **pause**.
- **Stepping**: over (`next`), in and out, by source lines.
- **The stack**: each call's function and line. Inlined code does not happen (no optimizer), and
  the entry code A0 generates is hidden.
- **Locals** of every frame, by the source names and types the frontend gave: numbers, strings,
  arrays, maps, structs and enum variants, nullable references, `any` values, captured variables,
  and interface values by their dynamic type. Structured values expand. Locals declared after the
  current position are left out; of two with the same name, the inner one is shown.
- **Hovers and watches** of local names (`evaluate`).
- **Failures**: a panic stops the program where it happened (reason `exception`, with the
  message) before its calls unwind, so its stack and locals can be inspected; continuing ends it
  with status 101, as it would end anyway.

The program's output becomes the client's debug console output; its standard input is empty,
since the protocol uses it. Globals are not shown.

### VS Code

The extension (`docs/editors.md`) adds a debugger of type `a0`. It runs `<command> dap` for the
language of the program, the entry of `a0.languages` whose extensions match it (with `lsp` in its
`args` replaced by `dap`):

```json
{ "type": "a0", "request": "launch", "name": "Debug the current file", "program": "${file}" }
```

`args` and `stopOnEntry` are optional. VS Code takes breakpoints only in files of languages it
knows; the extension declares Lox, Minnow and Kaleidoscope, and for other languages
`"debug.allowBreakpointsEverywhere": true` does it (projects made with `a0 new` set both).

### Neovim

```lua
local dap = require('dap')
dap.adapters.minnow = { type = 'executable', command = 'minnow', args = { 'dap' } }
dap.configurations.minnow = {
  { type = 'minnow', request = 'launch', name = 'Debug', program = '${file}' },
}
```

## Native code: `lang build -g`

`-g` describes an executable for the platform's debugger: DWARF line tables and the program's
functions, so that gdb and lldb stop at `file:line`, step by lines and print backtraces with source
functions and lines. It also turns off the optimizers, so that the code follows the source. On
macOS the information is gathered into `<program>.dSYM` with `dsymutil`.

```
minnow build prog.mn -g -o prog
gdb -ex 'break prog.mn:4' -ex run -ex bt ./prog
```

Not described, and why:

- **Variables.** Their places in native code (registers, stack slots, at each instruction) would
  come from Cranelift's value locations, which A0 does not ask for; `lang dap` shows locals.
- **Windows.** Its debuggers read PDB (CodeView), which A0 does not write; `lang dap` works there.
- **Types**, beyond the functions: the DWARF has no type entries.

## What a language needs

Nothing, if its HIR has spans: lines come from the spans of HIR expressions (`HirBuilder::set_span`),
locals from the names and types the builder knows. Three things help:

- Set spans on what is built, including literals: an expression built with a stale span puts
  its code on another line.
- A language whose functions are closures names them (`HirBuilder::set_closure_name`, as Lox
  does), or the stack shows `main::{closure}`.
- A language's own C runtime writes output with `a0_rt_write` (`a0rt.h`, "Debuggers"), as the A0
  runtime does, so that the debugger can take it.

## How it works

- Lowering records how each local is shown (`a0_ir::DebugKind`: the language's type name, and
  how to read the value's words: a struct's fields, an enum's variants, an array's elements) and
  where it is declared (`docs/ir.md`, "Debug information").
- `Interpreter::new_debug` marks every new source line in the compiled code and calls a hook
  there, and when the program fails, before unwinding; the hook stops by not returning, and
  answers the client's requests from the stopped program's frames (`a0-driver/src/dap.rs`).
  Values are read from the runtime's objects by their debug types (`values.rs`).
- The runtime gives the debugger the program's output, its failures and its end
  (`a0_rt_set_output`, `a0_rt_set_fail_hook`, `a0_rt_set_exit_hook`).
- Native debug information is DWARF written with `gimli` from the locations Cranelift reports for
  each instruction (`a0-native/src/dwarf.rs`), relocated against the program's functions.
