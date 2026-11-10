# Getting started

By the end of this page you will have your own small programming language, made with one command,
running a program. You will also see what it can do without any more work: build executables,
report errors, and work in an editor.

## What you need

- **Rust**, from [rustup.rs](https://rustup.rs).
- **A C compiler**, because part of A0 (its runtime) is written in C. On Linux and macOS, A0 uses
  `cc`: install your distribution's build tools, or Xcode's command line tools on macOS. On
  Windows, A0 uses Microsoft's compiler (MSVC), which comes with Visual Studio's "Desktop
  development with C++". A0 finds it on its own.

## Install the `a0` command

```
git clone https://github.com/Dan-k391/a0
cd a0
cargo install --path crates/a0
```

This compiles A0 and puts the `a0` command on your `PATH`. Check that it works:

```
a0 --help
```

If you would rather not compile A0, download the archive for your platform from the
[release page](https://github.com/Dan-k391/a0/releases/latest). It has `a0`, the example
languages, and everything needed to build for other platforms.

## Make a language

Go to the folder where you keep your projects, and run:

```
a0 new mylang
cd mylang
```

This makes a folder `mylang` with a complete small language called Mylang in it. The language is
a Rust project. It builds and passes its tests before you change anything. A Mylang program looks
like this (`examples/hello.mylang`):

```
fn greet(name: string) -> string {
  return "Hello, " + name + "!";
}

fn main() {
  print(greet("Mylang"));
}
```

Run it:

```
cargo run -- run examples/hello.mylang
```

The first time, Cargo downloads and compiles A0, which takes a few minutes. Then it prints:

```
Hello, Mylang!
```

Here, `cargo run --` builds your language's command, `mylang`, and runs it with the words after
`--`. So the line above runs `mylang run examples/hello.mylang`.

Your project gets A0 from GitHub, at the release that matches your `a0`. To use your own copy of
A0 instead, for example to change A0 itself, make the project with
`a0 new mylang --a0 path/to/a0`.

## What is in the project

You will change two files:

- **`mylang.a0g`, the grammar.** It says what Mylang programs look like: their words, symbols and
  rules. [Grammars](../grammar.md) explains the notation.
- **`src/lib.rs`, the compiler.** It reads the program that the grammar has parsed and describes
  each part to A0, in A0's high-level IR: a typed form of the program that A0 knows how to run,
  compile and debug. [The high-level IR](../hir.md) lists what it has.

Two more you will rarely touch:

- **`src/main.rs`** is the `mylang` command. It is a few lines, and gives the command everything
  in the next section.
- **`tests/`** holds the tests. Each file in `tests/programs` is a Mylang program that ends with
  comments saying what it must print. `cargo test` runs each program, builds it into an
  executable and runs that too, and checks that both print what the comments say. `tests/lsp.rs`
  and `tests/dap.rs` test the editor support and the debugger.

For example, `tests/programs/division.mylang` checks what happens when a program divides by zero:

```
// A runtime error stops the program with a message and where it happened.
fn divide(a: int, b: int) -> int {
  return a / b;
}

fn main() {
  print(divide(7, 2));
  print(divide(1, 0));
}
// expect: 3
// expect-stderr: panic: integer division by zero
// expect-stderr:     at divide (division.mylang:3:10)
// expect-stderr:     at main (division.mylang:8:9)
// expect-exit: 101
```

Run all the tests with:

```
cargo test
```

## What your language can do

A new language can already do everything below. In these commands, `mylang` stands for
`cargo run --`. After a build, you can also run the command directly as `target/debug/mylang`.

**Run a program.** A0 compiles it to machine code in memory, then runs it:

```
mylang run examples/hello.mylang
```

**Build an executable** that runs without A0 (`hello.exe` on Windows):

```
mylang build examples/hello.mylang -o hello
./hello
```

**Check a program for errors** without running it. Errors point at the problem and suggest a fix
when there is a likely one:

```
$ mylang check bad.mylang
error[E1001]: unknown name `y`
 --> bad.mylang:3:9
  |
3 |   print(y);
  |         ^ help: a variable with a similar name exists: `x`
```

`mylang explain E1001` says more about an error code.

**Show where a program failed.** When a program fails while it runs, it prints the error and
where each function was called from, and exits with status 101:

```
$ mylang run tests/programs/division.mylang
3
panic: integer division by zero
    at divide (tests/programs/division.mylang:3:10)
    at main (tests/programs/division.mylang:8:9)
```

**And more:**

| Command | What it does |
|---|---|
| `mylang repl` | an interactive prompt that runs what you type ([the REPL](../repl.md)) |
| `mylang lsp` | a language server, which gives editors errors as you type, go to definition, hovers and completion ([editors](../editors.md)) |
| `mylang dap` | a debugger for editors: breakpoints, stepping and variables ([debugging](../debugging.md)) |
| `mylang build -g prog.mylang` | an executable with debug information, for gdb, lldb or Visual Studio |
| `mylang build --target x86_64-windows prog.mylang` | an executable for another platform, once you have that platform's target pack ([other platforms](../cross.md)) |
| `mylang hir prog.mylang` | prints the high-level IR your compiler built, which helps when you change the compiler |

The project also has settings for VS Code in `.vscode`. Install A0's VS Code extension (in
`editors/vscode` in the A0 repository) and run `cargo build`. VS Code then uses your language's
server for `.mylang` files, and F5 debugs the file you are editing.

## Next

- To learn how to change your language, follow [the tutorial](../tutorial.md). It builds a
  complete small language, Kaleidoscope, one piece at a time, and explains each piece.
- To start a bigger language, read [Building a full language](b0.md). It walks through B0, a
  larger language whose compiler is meant to be copied and changed.
