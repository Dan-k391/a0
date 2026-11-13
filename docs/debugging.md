# Debugging

Every language built on A0 has a debugger. You can set breakpoints, step through the program line
by line, look at the call stack, and see the values of local variables. The language writes no
code for it.

There are two ways to debug:

- **In an editor**, such as VS Code or Neovim. The language's command has a debugger built in,
  `lang dap`, where `lang` is the command, such as `minnow`. It speaks the
  [Debug Adapter Protocol](https://microsoft.github.io/debug-adapter-protocol/) (DAP), the
  standard way editors talk to debuggers.
- **With the platform's debugger**: gdb, lldb, Visual Studio or WinDbg. Build the program with
  `lang build -g`, and debug the executable.

## What you get

Here is a Minnow program ([the language guides](book/languages.md)) with a bug on its last line:

```
// A program to debug.
struct Point { x: float, y: float }

fn norm(p: Point) -> float {
  let squares = p.x * p.x + p.y * p.y;
  return squares / 5.0;
}

fn main() {
  let name = "origin";
  let p = new Point { x: 3.0, y: 4.0 };
  let xs = [1, 2, 3]; let flags = [true, false, true];
  let n = norm(p);
  print(n);
  print(xs[n as int]);
}
```

With a breakpoint on line 5, a debugging session goes like this:

1. The program stops at line 5, at the breakpoint. The stack shows `norm` at line 5, called from
   `main` at line 13.
2. The locals of `norm` show `p` as `Point { x: 3, y: 4 }`, which opens to show its fields `x` and
   `y`. In `main`'s frame, `name` is `"origin"`, `xs` is `[1, 2, 3]` and `flags` is
   `[true, false, true]`.
3. **Step over** stops at line 6, where `squares` is `25`.
4. **Step out** goes back to `main`, at line 14. Hovering over `n` shows `5`.
5. **Continue** runs line 15, where `xs[5]` is out of bounds. The debugger stops there, with the
   message "index out of bounds", before the program ends. The stack and the locals can still be
   looked at.
6. **Continue** again ends the program, with exit status 101. The debug console shows what it
   printed: `5`, then the panic message.

These steps and values are what Minnow's debugger test checks, in `examples/minnow/tests/dap.rs`.

## Debug in VS Code

1. Install A0's VS Code extension, and add your language to the `a0.languages` setting
   ([editors](editors.md#use-it-in-vs-code)).
2. Add a launch configuration to `.vscode/launch.json`:

   ```json
   { "type": "a0", "request": "launch", "name": "Debug the current file", "program": "${file}" }
   ```

3. Open the program, click left of a line number to set a breakpoint, and press F5.

The configuration takes these fields:

| Field | Meaning |
|---|---|
| `program` | The file to debug. Required. |
| `args` | The program's arguments, a list of strings. |
| `stopOnEntry` | `true` to stop at the program's first line. |

The extension finds the program's language by its file ending: the entry of `a0.languages` whose
`extensions` match. It runs that entry's command with `dap`. When the entry has `args`, `lsp` in
them becomes `dap`. An entry that has only `language`, and no `extensions`, cannot be debugged.

VS Code lets you set breakpoints only in files of languages it knows. The extension declares B0,
Lox, Minnow, Kaleidoscope and Tin. For any other language, add this to your settings:

```json
"debug.allowBreakpointsEverywhere": true
```

A project made with `a0 new` already has this setting, and the launch configuration above.

## Debug in Neovim

With [nvim-dap](https://github.com/mfussenegger/nvim-dap):

```lua
local dap = require('dap')
dap.adapters.minnow = { type = 'executable', command = 'minnow', args = { 'dap' } }
dap.configurations.minnow = {
  { type = 'minnow', request = 'launch', name = 'Debug', program = '${file}' },
}
```

Any other DAP client works the same way: it starts the debugger as one of these commands.

```
minnow dap                       # a language's command
a0 lang calc.a0g dap             # a language defined by its grammar alone
python pebble.py dap             # a language written in Python
```

## What the debugger can do

- **Breakpoints** on lines, set before the program starts or while it runs. **Pause** stops a
  running program.
- **Stepping** by source lines: over (`next`), in, and out.
- **The stack**: each call's function and line. The entry code that A0 generates to start the program
  is hidden.
- **Locals** of every frame, with the names and types the language gave them. Numbers, strings,
  arrays, maps, structs, enum variants, nullable references, `any` values and captured variables
  are all shown. A value of an interface type is shown as the type it really has. Structured values
  open to show their parts.
- Locals declared after the current line are left out. When two locals have the same name, the
  inner one is shown.
- **Hovers and watches** of local names. An expression other than a local's name is not evaluated.
- **Failures**: a panic stops the program where it happened, with the panic's message, before its
  calls unwind. So the stack and locals at the failure can be looked at. Continuing ends the
  program with status 101, as it would have ended anyway.
- **Threads** (`std.thread`). Each thread is listed: 1 is the main thread, then 2, 3 and so on in
  the order they start, the same numbers stack traces use. The editor hears when each starts and
  ends.
- When one thread stops, the others pause at their next line, so every thread's stack and locals
  can be shown. A thread that is waiting, for a channel or for another thread, shows no stack until
  it runs again. A step moves only the thread that stopped. Continuing lets every thread go.

The program's output goes to the editor's debug console. Its standard input is empty, since the
debugger uses its own standard input to talk to the editor. Global variables are not shown.

## Debug an executable: `lang build -g`

`-g` adds debug information to an executable: its lines, functions and variables. With it, the
platform's debugger can stop at `file:line`, step by lines, print backtraces with source functions
and lines, and show variables with their values. `-g` also turns off A0's optimizations, so that
the machine code follows the source.

| Platform | Debug information | Debuggers |
|---|---|---|
| Linux | DWARF, in the executable | gdb, lldb |
| macOS | DWARF, gathered into `<program>.dSYM` with `dsymutil` | lldb |
| Windows | CodeView, which the linker gathers into `<program>.pdb` | Visual Studio, WinDbg, cdb |

On Windows the `.pdb` also holds a natvis file, which tells Visual Studio and WinDbg to show
strings as text and arrays as their elements.

To build a program and stop it at line 4:

```
minnow build prog.mn -g -o prog
gdb -ex 'break prog.mn:4' -ex run -ex 'info locals' -ex 'print *p' ./prog
lldb -o 'breakpoint set --file prog.mn --line 4' -o run -o 'frame variable' ./prog
```

On Windows, with cdb:

```
cdb -lines -c "bp `prog!prog.mn:4`;g;dv /t;dx -r1 *p" prog.exe
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

Variables are visible in the whole function, even before their declaration, where they are zero.
`lang dap` shows more: maps and dynamic values too.

## What a language needs

Nothing, if the high-level IR its compiler builds has spans. The high-level IR ([HIR](hir.md)) is
the typed form of the program a compiler gives A0. A span says where in the source each part came
from. The debugger takes lines from the spans of HIR expressions (`HirBuilder::set_span`), and
locals from the names and types the builder knows.

Three things help:

- Set spans on everything the compiler builds, literals included. An expression built with an
  old span puts its code on another line.
- A language whose functions are closures names them, with `HirBuilder::set_closure_name`, as Lox
  does. Otherwise the stack shows a closure in `main` as `main::{closure}`.
- A language with its own runtime in C writes output with `a0_rt_write` (in `a0rt.h`, under
  "Debuggers"), as A0's runtime does. Then the debugger can send that output to the editor.

A language tests its debugger with `a0_driver::dap::Client`, which starts `lang dap`, sends
requests and waits for events. Each example language has a test, in `examples/*/tests/dap.rs`.

## How it works

- `lang dap` compiles the program without the HIR optimizer, so that every function keeps its own
  frame and no code is inlined. It runs the program in A0's interpreter, in the debugger's own
  process.
- Lowering, the step that turns the HIR into A0's core IR, records how to show each local: the
  language's name for its type, and how to read the value (a struct's fields, an enum's variants,
  an array's elements). It also records where each local is declared ([the core
  IR](ir.md#debug-information)).
- The interpreter, started for debugging (`Interpreter::new_debug`), calls the debugger at every
  new source line, and when the program fails, before unwinding. The debugger stops the program by
  not returning. While it is stopped, the debugger answers the editor's requests from the
  program's frames, and reads values from the runtime's objects by the types lowering recorded.
- The runtime hands the debugger the program's output, its failures and its end
  (`a0_rt_set_output`, `a0_rt_set_fail_hook`, `a0_rt_set_exit_hook` in `a0rt.h`).
- Each thread the program starts runs in the interpreter with the same hook. Each thread has its
  own interpreter state over the shared program, and tells the debugger when it starts and ends. A
  thread that reaches a line while another is stopped waits in the hook, where the stopped thread
  can read its frames.
- For `-g`, Cranelift (the code generator A0 uses) reports a source location for each machine
  instruction. Each source variable gets a stack slot, at a fixed offset from the frame pointer,
  and each assignment also stores the value there. The slots are zeroed when the function starts.
  A0 writes DWARF (with the `gimli` library) or CodeView from these, with types from the core IR's
  debug types.
- A line's code can be in several places, such as a loop's condition or a failed bounds check
  that Cranelift moves to the end of the function. Windows' debuggers refuse a breakpoint on such a
  line as ambiguous, so in CodeView only the line's first place has its number. The others are
  hidden (line `0xfeefee`), which the debuggers step through. So a breakpoint on a line whose
  first place runs once, such as the set-up of a `for` loop, stops there once and not on every
  iteration: put it on a line of the body. gdb and lldb read DWARF, which keeps every place.
