# The REPL

Every language built on A0 has a REPL, a read-eval-print loop: a prompt where you type
declarations and statements, and see what they print. Start it with `lang repl`, where `lang` is
the language's command, such as `minnow` or `lox`. The language writes no code for it.

## What you get

A session with Minnow, one of A0's example languages ([the language guides](book/languages.md)):

```
$ minnow repl
minnow 1.0.0 (A0): :help for help, :quit to leave
> let x = 20;
> fn twice(n: int) -> int {
.   return n * 2;
. }
> print(twice(x) + 2);
42
> print(y);
error[E1001]: unknown name `y`
 --> <repl>:7:7
  |
7 | print(y);
  |       ^ help: a variable with a similar name exists: `x`
> let xs = [1, 2];
> print(xs[5]);
panic: index out of bounds: the length is 2 but the index is 5
    at main (<repl>:8:7)
> print(x);
20
> :show
   1 | fn twice(n: int) -> int {
   2 |   return n * 2;
   3 | }
   4 | fn main() {
   5 | let x = 20;
   6 | print(twice(x) + 2);
   7 | let xs = [1, 2];
   8 | print(x);
   9 | }
> :quit
```

What happened:

- `let x = 20;` printed nothing. The REPL does not print values by itself: print them.
- The function took three lines. The first line left a `{` open, so the REPL waited for more,
  with the prompt `. `.
- `print(y)` did not compile, so the REPL showed the error and dropped that input.
- `print(xs[5])` failed while it ran, so the REPL showed the panic and dropped that input too.
- `:show` printed the program the inputs made. The function went before `main`, and the
  statements into `main`. Neither dropped input is in it.

## Use it

Start the REPL with the language's command:

```
minnow repl                      # a language's command
a0 lang calc.a0g repl            # a language defined by its grammar alone
python pebble.py repl            # a language written in Python
```

Then type declarations and statements, as you would write them in a file. Each input runs
together with the ones before it.

**Inputs over several lines.** An input that ends early, with a `{` still open or a `;` missing at
the end, continues on the next line. The prompt changes to `. ` while it does. An empty line
submits the input as it is.

**Commands.** A line that is one of these commands is not an input:

| Command | What it does |
|---|---|
| `:help` | Shows the commands. |
| `:show` | Prints the program so far, with line numbers. |
| `:reset` | Starts over with an empty program. |
| `:quit` | Leaves. So do `:q`, `:exit`, and the end of the input. |

**Piped input.** When standard input is not a terminal, the REPL prints no banner and no prompts,
only what the program prints. That makes it easy to script and to test:

```
$ printf 'let x = 20;\nprint(x * 2);\n' | minnow repl
40
```

**Imports** in the REPL are found from the current directory.

## How it works

The inputs so far make one program. Each new input is added to it, and the whole program is
compiled and run again.

- **Where inputs go.** In a language whose statements run at the top level, such as Lox,
  Kaleidoscope or Calc, the inputs stay in the order they were typed. In a language whose
  statements live in `main`, such as Minnow, declarations go first and statements go into `main`.
  The grammar tells them apart: an input that parses after the declarations so far is a
  declaration, and anything else is a statement.
- **Each input runs the whole program again**, in a process of its own. The REPL runs the
  language's command as `lang run -`, which reads the program from standard input and runs it at
  native speed, compiled into memory with no linking. It uses a new process each time because A0's
  runtime keeps global state, so a process can run only one program.
- **Only new output is shown.** Statements typed before run again, side effects included, but
  their output is not shown again. If the program prints something different this time, for
  example because it reads the clock, all of its output is shown.
- **Inputs that fail are dropped.** An input that does not compile, or that makes the program fail
  (a panic, or an exit status other than 0), has its errors and output shown, and is then left out
  of the program. So the program so far always runs.
- **Line numbers** in errors are those of the program with the new input in its place: the lines
  `:show` would print if the input were kept.

## What a language needs

A language gets the REPL from the driver. Two things decide how well it works:

- **Its grammar**, given with `Language::grammar`. The REPL uses it to see that an input ends early,
  and to tell declarations from statements. Without a grammar, every line is a whole input.
- **Where statements go**, for a language whose statements live in a function. Give the driver
  the program around them, with `{}` where the statements go. Minnow's command does it like this
  (`examples/minnow/src/main.rs`, shortened):

  ```rust
  Language::new("minnow", compile)
      .grammar(minnow::GRAMMAR)
      .repl_main("fn main() {\n{}}\n")
      .main()
  ```

  Tin, whose `main` returns a status, uses `"fn main() -> i32 {\n{}return 0;\n}\n"`. A language
  whose statements run at the top level, such as Lox, gives none. In Python it is the `repl_main`
  argument of `a0.driver.main` ([A0 from Python](python.md)). In C it is
  `a0_language_set_repl_main`, in `a0.h`.

Each example language tests its REPL by piping a session into `lang repl`, in
`examples/*/tests/repl.rs`.
