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
- **Threads** (`std.thread`): each is listed (`threads`; 1 is the main thread, then 2, 3, ... as
  started, the numbers stack traces use) and announced as it starts and ends (`thread` events).
  When one stops, the others pause at their next line, so every thread's stack and locals can be
  shown; a thread that is waiting (for a channel, for another thread) shows no stack until it runs
  again. Steps belong to the thread that stopped; continuing lets every thread go.

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

`-g` describes an executable for the platform's debugger: lines, functions and variables, so that
gdb, lldb, Visual Studio, WinDbg and cdb stop at `file:line`, step by lines, print backtraces with
source functions and lines, and show variables with their values. It also turns off the
optimizers, so that the code follows the source.

- **Linux and macOS**: DWARF in the executable (on macOS gathered into `<program>.dSYM` with
  `dsymutil`).
- **Windows**: CodeView, which the linker gathers into `<program>.pdb`, with a natvis file in it
  that shows strings as text and arrays as their elements.

```
minnow build prog.mn -g -o prog
gdb -ex 'break prog.mn:4' -ex run -ex 'info locals' -ex 'print *p' ./prog
lldb -o 'breakpoint set --file prog.mn --line 4' -o run -o 'frame variable' ./prog
cdb -lines -c "bp `prog!prog.mn:4`;g;dv /t;dx -r1 *p" prog.exe      (Windows)
```

How variables look:

| Source type | gdb, lldb | Visual Studio, WinDbg |
|---|---|---|
| integers, floats, booleans | the value | the value |
| strings | a C string (`"hello"`) | `a0::string *`, shown as the text |
| structs, enums, other objects | a pointer to a structure of the fields (`print *p`) | the same (`dx -r1 *p`) |
| arrays | a pointer to `{len, cap, data}` (`print *xs->data@xs->len`) | the elements |
| captured variables | the value | a pointer to a box holding the value |
| maps, `any`, interfaces, closures | a pointer, its target named but not described | the same |

Variables are visible in the whole function, before their declaration too (as zero). `lang dap`
shows everything, maps and dynamic values included.

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
- Threads the program starts are run by the interpreter with the same hook (each thread has its
  own interpreter state over the shared program, and tells the hook when it starts and ends). A
  thread that reaches a line while another is stopped waits in the hook, where the stopped thread
  can read its frames.
- Native debug information comes from the locations Cranelift reports for each instruction and,
  for variables, from stack slots that `-g` gives every source variable (each assignment also
  stores there, at a fixed offset from the frame pointer, D80). It is DWARF written with `gimli`
  (`a0-native/src/dwarf.rs`) or CodeView (`a0-native/src/codeview.rs`), relocated against the
  program's functions; types come from the IR's debug types.
